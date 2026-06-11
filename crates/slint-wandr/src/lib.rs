//! # slint-wandr — Slint platform + renderer for wandr guests (task 100)
//!
//! Runs a Slint UI as a wasm32-wasip2 wandr guest: a custom
//! [`i_slint_core::platform::Platform`] + `WindowAdapter` + renderer whose
//! `ItemRenderer` draws through the **wasi:canvas DRAFT**
//! (proposals/wasi-canvas — this crate is its PROVING CONSUMER): the
//! embedder hands over `canvas`/`graphics` resources via the `embedding`
//! interface inside each host-driven `render-frame`. Text is shaped IN
//! the guest by Slint's parley stack (`shared-parley`); glyph runs go to
//! the host as `glyphs.draw-glyphs` against `typeface` resources built
//! from the same font bytes. Platform bits (window metrics, IME) stay on
//! `my:skiko-gfx`. History: the original my:skiko-gfx port was task 100;
//! canonical mapping `docs/skia-wit-mapping.md`.
//!
//! There is no event loop: the host drives everything through the exported
//! `renderer` interface (`render-frame`, `on-pointer-event-v2`, …) and reads
//! `frame-pacing.next-frame-delay` for on-demand rendering. A guest app
//! wires those exports with [`launch!`]:
//!
//! ```ignore
//! slint::include_modules!();
//! slint_wandr::launch!(|| {
//!     let ui = MainWindow::new().unwrap();
//!     ui.show().unwrap();
//!     ui // kept alive by the macro's thread_local
//! });
//! ```
//!
//! The app's `slint` dependency must pin the SAME git rev as this crate's
//! `i-slint-core` (one crate instance = one platform global).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use i_slint_core::api::Window;
use i_slint_core::item_tree::ItemTreeWeak;
use i_slint_core::platform::{PlatformError, WindowEvent};
use i_slint_core::renderer::RendererSealed;
use i_slint_core::textlayout::sharedparley;
use i_slint_core::window::{WindowAdapter, WindowInner};

mod itemrenderer;

/// wit-bindgen re-export for the [`launch!`] expansion (`runtime_path`),
/// so the guest crate needs no wit-bindgen dependency of its own.
#[doc(hidden)]
pub use wit_bindgen as __wit_bindgen;

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "slint-wandr-imports",
    });
}
pub(crate) use bindings::my::skiko_gfx::{ime as host_ime, window as host_window};

/// Drawing comes from the wasi:canvas DRAFT (proposals/wasi-canvas) — this
/// crate is its proving consumer. Single source of truth: the generate!
/// points straight at the proposal's wit dir.
mod canvas_bindings {
    wit_bindgen::generate!({
        path: "../../proposals/wasi-canvas/wit",
        world: "embedded-canvas-guest",
    });
}
pub(crate) use canvas_bindings::wasi::canvas::draw as wdraw;
pub(crate) use canvas_bindings::wasi::canvas::embedding as wembed;
pub(crate) use canvas_bindings::wasi::canvas::glyphs as wglyphs;
pub(crate) use canvas_bindings::wasi::canvas::types as wtypes;

// ─── platform ────────────────────────────────────────────────────────────────

/// The wandr [`i_slint_core::platform::Platform`]: hands out the single
/// [`WandrWindowAdapter`] (wandr guests own exactly one surface).
struct WandrPlatform;

thread_local! {
    static ADAPTER: RefCell<Option<Rc<WandrWindowAdapter>>> = const { RefCell::new(None) };
}

impl i_slint_core::platform::Platform for WandrPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let adapter = WandrWindowAdapter::new();
        ADAPTER.with(|a| *a.borrow_mut() = Some(adapter.clone()));
        Ok(adapter)
    }

    fn debug_log(&self, arguments: core::fmt::Arguments) {
        eprintln!("slint: {arguments}");
    }
}

/// Install the platform. Idempotent; called by the [`launch!`] expansion
/// before the app factory constructs the first component.
pub fn init_platform() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // ‼️ Must be set BEFORE anything calls detect_operating_system():
        // on target_family="wasm" its fallback queries web_sys::window()
        // (browser-only) → js-sys "cannot access imported statics" panic →
        // SIGILL. Hit live on the first keystroke (the shortcut-matching
        // path in input.rs calls it). Android is the truthful answer and
        // picks the right shortcut/dialog conventions.
        i_slint_core::OPERATING_SYSTEM_OVERRIDE
            .with(|o| o.set(Some(i_slint_core::OperatingSystemType::Android)));
        // with_global_context installs the platform AND hands us the
        // SlintContext, where the emoji fallback gets registered.
        let _ = i_slint_core::context::with_global_context(
            || Ok(Box::new(WandrPlatform)),
            register_emoji_fallback,
        );
    });
}

/// Slint's wasm builds embed only Inter (i-slint-common sharedfontique),
/// which has no emoji coverage — emoji render as tofu. parley resolves
/// emoji clusters through `fontique::GenericFamily::Emoji`, so register the
/// device's color-emoji font (wandr-host preopens /system/fonts read-only
/// at /system-fonts) under that family. Best-effort: missing font = tofu,
/// not an error.
fn register_emoji_fallback(ctx: &i_slint_core::SlintContext) {
    const EMOJI_FONT: &str = "/system-fonts/NotoColorEmoji.ttf";
    let Ok(bytes) = std::fs::read(EMOJI_FONT) else {
        eprintln!("slint-wandr: no emoji font at /system-fonts/NotoColorEmoji.ttf");
        return;
    };
    use i_slint_core::textlayout::sharedparley::fontique;
    let mut font_ctx = ctx.font_context().borrow_mut();
    let fonts = font_ctx.collection.register_fonts(bytes.into(), None);
    font_ctx.collection.append_generic_families(
        fontique::GenericFamily::Emoji,
        fonts.iter().map(|(family_id, _)| *family_id),
    );
}

// ─── window adapter ──────────────────────────────────────────────────────────

/// The one window: sized from the host surface, scale factor = the host
/// window density (px per dp — Slint's logical unit becomes dp).
pub struct WandrWindowAdapter {
    window: Window,
    renderer: WandrRenderer,
    size: Cell<i_slint_core::api::PhysicalSize>,
    needs_redraw: Cell<bool>,
    /// An editor is focused and the wandr keyboard overlay is engaged
    /// (set by input_method_request Enable/Disable).
    ime_attached: Cell<bool>,
    /// The wasi:canvas creation factory (embedding.get-graphics) —
    /// acquired once, lives as long as the adapter.
    graphics: std::cell::OnceCell<wdraw::Graphics>,
}

impl WandrWindowAdapter {
    fn new() -> Rc<Self> {
        // Surface size is discovered at the first frame (the canvas handle
        // carries it); the adapter starts 0x0 and render() dispatches the
        // real Resized before the first draw.
        let adapter = Rc::new_cyclic(|weak: &Weak<Self>| Self {
            window: Window::new(weak.clone()),
            renderer: WandrRenderer::default(),
            size: Cell::new(i_slint_core::api::PhysicalSize::new(0, 0)),
            needs_redraw: Cell::new(true),
            ime_attached: Cell::new(false),
            graphics: std::cell::OnceCell::new(),
        });
        let density = host_window::get_density().max(0.1);
        adapter
            .window
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: density });
        adapter
    }

    /// Blur the focused editor (Slint fires FocusOut → our
    /// input_method_request(Disable) → notify-editor-detached → the
    /// arbiter hides the keyboard overlay). Used by the keyboard's hide
    /// button (which arrives as ESC, the task-47 convention) and by
    /// tap-outside-the-editor dismissal.
    fn clear_editor_focus(&self) {
        let window_inner = WindowInner::from_pub(&self.window);
        let focus = window_inner.focus_item.borrow().clone();
        if let Some(item) = focus.upgrade() {
            window_inner.set_focus_item(
                &item,
                false,
                i_slint_core::items::FocusReason::Programmatic,
            );
        }
    }

    /// One full frame through the canvas WIT. The host clears + presents
    /// around begin/end-frame; everything between goes through the
    /// [`itemrenderer::WandrItemRenderer`].
    fn render(&self) {
        let window_inner = WindowInner::from_pub(&self.window);
        let Some(window_adapter) = self.renderer.window_adapter() else { return };
        let graphics = self.graphics.get_or_init(wembed::get_graphics);
        let canvas = wembed::begin_frame();
        // The canvas handle carries the surface size — sync Slint's window
        // before drawing (covers both the first frame and embedder-side
        // resizes the on-resize export may not have delivered yet).
        let (cw, ch) = (canvas.width() as u32, canvas.height() as u32);
        let scale = self.window.scale_factor();
        if self.size.get() != i_slint_core::api::PhysicalSize::new(cw, ch) {
            self.size.set(i_slint_core::api::PhysicalSize::new(cw, ch));
            self.window.dispatch_event(WindowEvent::Resized {
                size: i_slint_core::api::LogicalSize::new(cw as f32 / scale, ch as f32 / scale),
            });
        }
        self.renderer.text_layout_cache.clear_cache_if_scale_factor_changed(&self.window);
        window_inner.draw_contents(|components, post_render| {
            let mut ir = itemrenderer::WandrItemRenderer::new(
                &self.window,
                scale,
                &self.renderer.text_layout_cache,
                &canvas,
                graphics,
            );
            // The window background (the WindowItem brush) — painted before
            // the item tree like the upstream renderers do.
            if let Some(window_item) = window_inner.window_item() {
                let brush = window_item.as_pin_ref().background();
                ir.fill_window_background(brush);
            }
            for (component, origin) in components {
                if let Some(component) = ItemTreeWeak::upgrade(component) {
                    i_slint_core::item_rendering::render_component_items(
                        &component,
                        &mut ir,
                        *origin,
                        &window_adapter,
                    );
                }
            }
            post_render(&mut ir);
        });
        drop(canvas);
        wembed::end_frame();
    }
}

impl WindowAdapter for WandrWindowAdapter {
    fn window(&self) -> &Window {
        &self.window
    }

    fn size(&self) -> i_slint_core::api::PhysicalSize {
        self.size.get()
    }

    fn renderer(&self) -> &dyn i_slint_core::platform::Renderer {
        &self.renderer
    }

    fn request_redraw(&self) {
        self.needs_redraw.set(true);
    }

    fn internal(
        &self,
        _: i_slint_core::InternalToken,
    ) -> Option<&dyn i_slint_core::window::WindowAdapterInternal> {
        Some(self)
    }
}

impl i_slint_core::window::WindowAdapterInternal for WandrWindowAdapter {
    /// Soft-keyboard wiring (task 47 IME model): Slint raises this when a
    /// TextInput gains/loses focus. Attach summons the wandr keyboard
    /// overlay (which resizes our surface via on-resize); typed text
    /// arrives back through on-key-event-v2. `Update` fires per keystroke
    /// and is deliberately ignored — the keyboard tracks content from the
    /// keys it sent; re-attaching every keystroke would churn the overlay.
    fn input_method_request(&self, request: i_slint_core::window::InputMethodRequest) {
        use i_slint_core::window::InputMethodRequest as R;
        match request {
            R::Enable(props) => {
                let input_type = match props.input_type {
                    i_slint_core::items::InputType::Number => "number",
                    i_slint_core::items::InputType::Decimal => "decimal",
                    i_slint_core::items::InputType::Password => "password",
                    _ => "text",
                };
                // The wandr contract carries CHAR offsets (what the
                // keyboard guests consume); Slint reports BYTE offsets.
                let to_chars = |byte_off: usize| {
                    props.text.as_str().get(..byte_off).map_or(0, |s| s.chars().count()) as u32
                };
                let cursor = to_chars(props.cursor_position);
                let anchor = props.anchor_position.map_or(cursor, &to_chars);
                host_ime::notify_editor_attached(
                    input_type,
                    "",
                    props.text.as_str(),
                    cursor.min(anchor),
                    cursor.max(anchor),
                );
                self.ime_attached.set(true);
            }
            R::Disable => {
                host_ime::notify_editor_detached();
                self.ime_attached.set(false);
            }
            _ => {}
        }
    }
}

// ─── renderer (RendererSealed) ───────────────────────────────────────────────

/// The `RendererSealed` impl. All text layout/metrics delegate to the shared
/// parley helpers (`sharedparley::*`) — identical to what the upstream skia
/// and femtovg renderers do; only the drawing differs (our `ItemRenderer`).
#[derive(Default)]
pub struct WandrRenderer {
    maybe_window_adapter: RefCell<Option<Weak<dyn WindowAdapter>>>,
    text_layout_cache: sharedparley::TextLayoutCache,
}

impl RendererSealed for WandrRenderer {
    fn text_size(
        &self,
        text_item: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderString>,
        item_rc: &i_slint_core::item_tree::ItemRc,
        max_width: Option<i_slint_core::lengths::LogicalLength>,
        text_wrap: i_slint_core::items::TextWrap,
    ) -> i_slint_core::lengths::LogicalSize {
        sharedparley::text_size(
            self,
            text_item,
            item_rc,
            max_width,
            text_wrap,
            Some(&self.text_layout_cache),
        )
        .unwrap_or_default()
    }

    fn char_size(
        &self,
        text_item: core::pin::Pin<&dyn i_slint_core::item_rendering::HasFont>,
        item_rc: &i_slint_core::item_tree::ItemRc,
        ch: char,
    ) -> i_slint_core::lengths::LogicalSize {
        self.slint_context()
            .and_then(|ctx| {
                let mut font_ctx = ctx.font_context().borrow_mut();
                sharedparley::char_size(&mut font_ctx, text_item, item_rc, ch)
            })
            .unwrap_or_default()
    }

    fn font_metrics(
        &self,
        font_request: i_slint_core::graphics::FontRequest,
    ) -> i_slint_core::items::FontMetrics {
        self.slint_context()
            .map(|ctx| {
                let mut font_ctx = ctx.font_context().borrow_mut();
                sharedparley::font_metrics(&mut font_ctx, font_request)
            })
            .unwrap_or_default()
    }

    fn text_input_byte_offset_for_position(
        &self,
        text_input: core::pin::Pin<&i_slint_core::items::TextInput>,
        item_rc: &i_slint_core::item_tree::ItemRc,
        pos: i_slint_core::lengths::LogicalPoint,
    ) -> usize {
        sharedparley::text_input_byte_offset_for_position(self, text_input, item_rc, pos)
    }

    fn text_input_cursor_rect_for_byte_offset(
        &self,
        text_input: core::pin::Pin<&i_slint_core::items::TextInput>,
        item_rc: &i_slint_core::item_tree::ItemRc,
        byte_offset: usize,
    ) -> i_slint_core::lengths::LogicalRect {
        sharedparley::text_input_cursor_rect_for_byte_offset(self, text_input, item_rc, byte_offset)
    }

    fn register_font_from_memory(
        &self,
        data: &'static [u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ctx = self.slint_context().ok_or("slint platform not initialized")?;
        ctx.font_context().borrow_mut().register_static_font(data);
        Ok(())
    }

    fn register_font_from_path(
        &self,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let contents = std::fs::read(path)?;
        let ctx = self.slint_context().ok_or("slint platform not initialized")?;
        ctx.font_context().borrow_mut().collection.register_fonts(contents.into(), None);
        Ok(())
    }

    fn free_graphics_resources(
        &self,
        component: i_slint_core::item_tree::ItemTreeRef,
        _items: &mut dyn Iterator<Item = core::pin::Pin<i_slint_core::items::ItemRef<'_>>>,
    ) -> Result<(), PlatformError> {
        self.text_layout_cache.component_destroyed(component);
        Ok(())
    }

    fn set_window_adapter(&self, window_adapter: &Rc<dyn WindowAdapter>) {
        *self.maybe_window_adapter.borrow_mut() = Some(Rc::downgrade(window_adapter));
        self.text_layout_cache.clear_all();
    }

    fn window_adapter(&self) -> Option<Rc<dyn WindowAdapter>> {
        self.maybe_window_adapter.borrow().as_ref().and_then(|w| w.upgrade())
    }

    fn supports_transformations(&self) -> bool {
        // The canvas WIT rotates/scales natively (ItemRenderer::rotate/scale).
        true
    }
}

// ─── runtime entry points (called from the launch! expansion) ────────────────

fn with_adapter(f: impl FnOnce(&Rc<WandrWindowAdapter>)) {
    ADAPTER.with(|a| {
        if let Some(adapter) = a.borrow().as_ref() {
            f(adapter);
        }
    });
}

fn has_active_animations() -> bool {
    i_slint_core::animations::CURRENT_ANIMATION_DRIVER.with(|d| d.has_active_animations())
}

/// Host `render-frame`: tick timers/animations, redraw if dirty or animating.
pub fn handle_render_frame(_nanos: u64) {
    i_slint_core::platform::update_timers_and_animations();
    with_adapter(|adapter| {
        if adapter.needs_redraw.replace(false) || has_active_animations() {
            adapter.render();
        }
    });
}

/// Pointer kinds as wired by [`launch!`] from the WIT `pointer-kind` enum.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Down,
    Up,
    Move,
    Scroll,
}

/// Host `on-pointer-event-v2`. x/y are surface (physical) pixels; Slint
/// wants logical. Multi-touch: Slint's input model is single-pointer, so
/// pointer-id is ignored (first finger wins at the host's event order).
/// Scroll is unused on a touch panel (drags drive Flickable) — ignored.
pub fn handle_pointer_event(_pointer_id: u32, kind: PointerKind, x: f32, y: f32) {
    with_adapter(|adapter| {
        let scale = adapter.window.scale_factor();
        let position = i_slint_core::api::LogicalPosition::new(x / scale, y / scale);
        let button = i_slint_core::items::PointerEventButton::Left;
        let event = match kind {
            PointerKind::Down => WindowEvent::PointerPressed { position, button },
            PointerKind::Up => WindowEvent::PointerReleased { position, button },
            PointerKind::Move => WindowEvent::PointerMoved { position },
            PointerKind::Scroll => return,
        };
        adapter.window.dispatch_event(event);

        // Android keyboard UX: a tap OUTSIDE the focused editor dismisses
        // the keyboard. Checked AFTER dispatch so focus moves (and possible
        // re-attach to another editor) settle first; if an editor is still
        // attached and the tap landed outside its rect, blur it.
        if kind == PointerKind::Down && adapter.ime_attached.get() {
            let window_inner = WindowInner::from_pub(&adapter.window);
            let focus = window_inner.focus_item.borrow().clone();
            if let Some(item) = focus.upgrade() {
                let origin = item.map_to_window(i_slint_core::lengths::LogicalPoint::default());
                let size = item.geometry().size;
                let inside = position.x >= origin.x
                    && position.x <= origin.x + size.width
                    && position.y >= origin.y
                    && position.y <= origin.y + size.height;
                if !inside {
                    adapter.clear_editor_focus();
                }
            }
        }
    });
}

thread_local! {
    /// True once any v3 key event arrived: the host emits v2 AND v3 for
    /// every keystroke, so once v3 is proven live, v2 is ignored to avoid
    /// double key delivery (v2 stays as the fallback under an old host).
    static SAW_KEY_V3: Cell<bool> = const { Cell::new(false) };
}

/// Host `key-input.on-key` (v3, optional export) — the W3C UIEvents model:
/// physical `code` token + modifiers + produced `text`. This is the
/// desktop-grade path (winit/Wayland hosts send real modifiers, so Slint
/// shortcuts like ctrl+c work: modifier keys are forwarded as Slint's
/// special code points and Slint tracks the state itself).
pub fn handle_key_v3(
    down: bool,
    repeat: bool,
    code: &str,
    text: &str,
    _alt: bool, _ctrl: bool, _meta: bool, _shift: bool,
) {
    use i_slint_core::platform::Key;
    SAW_KEY_V3.with(|c| c.set(true));
    // The wandr keyboard's hide button arrives as ESC (task-47): while an
    // editor is attached, blur + swallow (see handle_key_event for detail).
    if code == "Escape" && down {
        let mut dismissed = false;
        with_adapter(|adapter| {
            if adapter.ime_attached.get() {
                adapter.clear_editor_focus();
                dismissed = true;
            }
        });
        if dismissed {
            return;
        }
    }
    let ch: Option<char> = match code {
        // Modifier keys — Slint derives its modifier state from these.
        "ShiftLeft" => Some(Key::Shift.into()),
        "ShiftRight" => Some(Key::ShiftR.into()),
        "ControlLeft" => Some(Key::Control.into()),
        "ControlRight" => Some(Key::ControlR.into()),
        "AltLeft" => Some(Key::Alt.into()),
        "AltRight" => Some(Key::AltGr.into()),
        "MetaLeft" => Some(Key::Meta.into()),
        "MetaRight" => Some(Key::MetaR.into()),
        // Named editing/navigation keys.
        "Backspace" => Some(Key::Backspace.into()),
        "Tab" => Some(Key::Tab.into()),
        "Enter" | "NumpadEnter" => Some(Key::Return.into()),
        "Escape" => Some(Key::Escape.into()),
        "Delete" => Some(Key::Delete.into()),
        "Insert" => Some(Key::Insert.into()),
        "Home" => Some(Key::Home.into()),
        "End" => Some(Key::End.into()),
        "PageUp" => Some(Key::PageUp.into()),
        "PageDown" => Some(Key::PageDown.into()),
        "ArrowLeft" => Some(Key::LeftArrow.into()),
        "ArrowUp" => Some(Key::UpArrow.into()),
        "ArrowRight" => Some(Key::RightArrow.into()),
        "ArrowDown" => Some(Key::DownArrow.into()),
        "F1" => Some(Key::F1.into()),
        "F2" => Some(Key::F2.into()),
        "F3" => Some(Key::F3.into()),
        "F4" => Some(Key::F4.into()),
        "F5" => Some(Key::F5.into()),
        "F6" => Some(Key::F6.into()),
        "F7" => Some(Key::F7.into()),
        "F8" => Some(Key::F8.into()),
        "F9" => Some(Key::F9.into()),
        "F10" => Some(Key::F10.into()),
        "F11" => Some(Key::F11.into()),
        "F12" => Some(Key::F12.into()),
        _ => None,
    };
    let key_text: i_slint_core::SharedString = match ch {
        Some(ch) => ch.into(),
        // Printable input: the host already resolved layout + modifiers.
        None if !text.is_empty() => text.into(),
        None => return,
    };
    with_adapter(|adapter| {
        let event = if !down {
            WindowEvent::KeyReleased { text: key_text.clone() }
        } else if repeat {
            WindowEvent::KeyPressRepeated { text: key_text.clone() }
        } else {
            WindowEvent::KeyPressed { text: key_text.clone() }
        };
        adapter.window.dispatch_event(event);
    });
}

/// Host `on-key-event-v2`. `code_point` carries printable chars; `key-id`
/// uses the Compose-webMain key constants for specials (the same contract
/// dioxus-canvas consumes) — mapped to Slint's special-key code points.
/// FALLBACK path: ignored once v3 (`handle_key_v3`) is observed, since
/// hosts that know v3 emit both.
pub fn handle_key_event(down: bool, code_point: u32, key_id: u32) {
    use i_slint_core::platform::Key;
    if SAW_KEY_V3.with(|c| c.get()) {
        return;
    }
    // The wandr keyboard's hide button arrives as ESC (the task-47
    // convention: "reuse the lose-focus path"). While an editor is
    // attached, ESC means "dismiss the keyboard": blur the editor (which
    // detaches + hides the overlay) and swallow the key — matching the
    // Android back-closes-keyboard behavior. With no editor attached, ESC
    // flows through (arbiter `back` routes it to the app).
    if key_id == 27 && down {
        let mut dismissed = false;
        with_adapter(|adapter| {
            if adapter.ime_attached.get() {
                adapter.clear_editor_focus();
                dismissed = true;
            }
        });
        if dismissed {
            return;
        }
    }
    let ch: Option<char> = match key_id {
        8 => Some(Key::Backspace.into()),
        9 => Some(Key::Tab.into()),
        13 => Some(Key::Return.into()),
        27 => Some(Key::Escape.into()),
        37 => Some(Key::LeftArrow.into()),
        38 => Some(Key::UpArrow.into()),
        39 => Some(Key::RightArrow.into()),
        40 => Some(Key::DownArrow.into()),
        46 => Some(Key::Delete.into()),
        _ => char::from_u32(code_point).filter(|c| *c != '\0'),
    };
    let Some(ch) = ch else { return };
    let text: i_slint_core::SharedString = ch.into();
    with_adapter(|adapter| {
        let event = if down {
            WindowEvent::KeyPressed { text: text.clone() }
        } else {
            WindowEvent::KeyReleased { text: text.clone() }
        };
        adapter.window.dispatch_event(event);
    });
}

/// Host `on-resize` (rotation, IME inset, …): physical surface pixels.
pub fn handle_resize(w: u32, h: u32) {
    with_adapter(|adapter| {
        adapter.size.set(i_slint_core::api::PhysicalSize::new(w, h));
        let scale = adapter.window.scale_factor();
        adapter.window.dispatch_event(WindowEvent::Resized {
            size: i_slint_core::api::LogicalSize::new(w as f32 / scale, h as f32 / scale),
        });
        adapter.needs_redraw.set(true);
    });
}

/// Host `frame-pacing.next-frame-delay` (on-demand rendering): 0 while
/// animating or dirty; otherwise the next Slint timer deadline; large when
/// fully idle (the host clamps to its idle cap).
pub fn next_frame_delay() -> u32 {
    let dirty = ADAPTER.with(|a| {
        a.borrow().as_ref().map_or(false, |adapter| adapter.needs_redraw.get())
    });
    if dirty || has_active_animations() {
        return 0;
    }
    match i_slint_core::platform::duration_until_next_timer_update() {
        Some(d) => d.as_millis().min(60_000) as u32,
        None => 60_000,
    }
}

/// Wire a Slint UI to the wandr host. Invoke ONCE at the guest crate root.
/// The factory runs lazily on the first `render-frame`; whatever it returns
/// is kept alive for the process lifetime (return your component handle).
#[macro_export]
macro_rules! launch {
    ($factory:expr $(,)?) => {
        $crate::__wit_bindgen::generate!({
            // Hermetic: `path: []` stops generate! from also parsing the
            // invoking crate's default `wit/` directory (which may declare
            // my:skiko-gfx again, or unrelated packages).
            path: [],
            inline: r#"
package my:skiko-gfx@0.1.0;

interface renderer {
    enum pointer-kind { down, up, move, scroll }
    enum key-kind     { down, up }
    render-frame:          func(nanos: u64);
    on-pointer-event:      func(kind: pointer-kind, x: f32, y: f32);
    on-key-event:          func(kind: key-kind, key-code: u32);
    on-resize:             func(w: u32, h: u32);
    on-scheduled-callback: func(callback-id: u32);
    on-pointer-event-v2:   func(pointer-id: u32, kind: pointer-kind, x: f32, y: f32, pressure: f32);
    on-key-event-v2:       func(kind: key-kind, code-point: u32, key-id: u32);
    on-lifecycle-changed:  func(state: u32);
}

interface frame-pacing {
    next-frame-delay: func() -> u32;
}

interface key-input {
    record key-event {
        down:   bool,
        repeat: bool,
        code:   string,
        text:   string,
        alt:    bool,
        ctrl:   bool,
        meta:   bool,
        shift:  bool,
    }
    on-key: func(ev: key-event);
}

world slint-app {
    export renderer;
    export frame-pacing;
    export key-input;
}
"#,
            world: "slint-app",
            pub_export_macro: true,
            export_macro_name: "__slint_wandr_export",
            runtime_path: "::slint_wandr::__wit_bindgen::rt",
        });

        ::std::thread_local! {
            static __SLINT_WANDR_APP:
                ::core::cell::RefCell<::core::option::Option<::std::boxed::Box<dyn ::core::any::Any>>> =
                ::core::cell::RefCell::new(::core::option::Option::None);
        }

        fn __slint_wandr_ensure_app() {
            __SLINT_WANDR_APP.with(|slot| {
                if slot.borrow().is_none() {
                    ::slint_wandr::init_platform();
                    let app = ($factory)();
                    *slot.borrow_mut() =
                        ::core::option::Option::Some(::std::boxed::Box::new(app) as _);
                }
            });
        }

        #[doc(hidden)]
        struct __SlintWandrGuest;

        impl exports::my::skiko_gfx::renderer::Guest for __SlintWandrGuest {
            fn render_frame(nanos: u64) {
                __slint_wandr_ensure_app();
                ::slint_wandr::handle_render_frame(nanos);
            }
            fn on_resize(w: u32, h: u32) {
                ::slint_wandr::handle_resize(w, h);
            }
            fn on_pointer_event_v2(
                pid: u32,
                kind: exports::my::skiko_gfx::renderer::PointerKind,
                x: f32, y: f32, _pressure: f32,
            ) {
                use exports::my::skiko_gfx::renderer::PointerKind as K;
                let kind = match kind {
                    K::Down => ::slint_wandr::PointerKind::Down,
                    K::Up => ::slint_wandr::PointerKind::Up,
                    K::Move => ::slint_wandr::PointerKind::Move,
                    K::Scroll => ::slint_wandr::PointerKind::Scroll,
                };
                ::slint_wandr::handle_pointer_event(pid, kind, x, y);
            }
            fn on_key_event_v2(
                kind: exports::my::skiko_gfx::renderer::KeyKind,
                code_point: u32, key_id: u32,
            ) {
                use exports::my::skiko_gfx::renderer::KeyKind as K;
                ::slint_wandr::handle_key_event(matches!(kind, K::Down), code_point, key_id);
            }
            fn on_pointer_event(
                _kind: exports::my::skiko_gfx::renderer::PointerKind, _x: f32, _y: f32,
            ) {}
            fn on_key_event(
                _kind: exports::my::skiko_gfx::renderer::KeyKind, _key_code: u32,
            ) {}
            fn on_scheduled_callback(_callback_id: u32) {}
            fn on_lifecycle_changed(_state: u32) {}
        }

        impl exports::my::skiko_gfx::frame_pacing::Guest for __SlintWandrGuest {
            fn next_frame_delay() -> u32 {
                ::slint_wandr::next_frame_delay()
            }
        }

        impl exports::my::skiko_gfx::key_input::Guest for __SlintWandrGuest {
            fn on_key(ev: exports::my::skiko_gfx::key_input::KeyEvent) {
                ::slint_wandr::handle_key_v3(
                    ev.down, ev.repeat, &ev.code, &ev.text,
                    ev.alt, ev.ctrl, ev.meta, ev.shift,
                );
            }
        }

        __slint_wandr_export!(__SlintWandrGuest);
    };
}
