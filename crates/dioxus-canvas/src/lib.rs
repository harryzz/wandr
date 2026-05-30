//! dioxus-canvas — a "tiny Blitz" that drives the wart canvas WIT from a dioxus
//! `VirtualDom` (task 59). The reactive, flexbox-laid-out alternative to a
//! hand-rolled canvas painter (`war.launcher`) for richer Rust guests, without
//! Kotlin/Compose's binary size / leak / working-set cost.
//!
//! Pipeline (the four steps from the spike README):
//!   1. VirtualDom mutations → retained node arena (`dom`, a `WriteMutations`
//!      stack machine over dioxus templates).
//!   2. node tags/styles → `taffy::Style`; `compute_layout` (text leaves measured
//!      via the host `measure-text` WIT verb through `CanvasSink`).
//!   3. walk the laid-out tree → canvas-WIT draw verbs (`begin_frame` / `clear` /
//!      `fill_rrect` / `create_text_blob` + `draw_text_blob`).
//!   4. hit-test pointer events → dioxus `handle_event` → re-render.
//!
//! Incremental like the launcher: re-diff + relayout only when the VirtualDom
//! reports mutations or input fired; otherwise replay the cached draw list.

mod dom;
mod events;
mod launch; // defines the exported `launch!` macro
mod sink;
mod style;

use std::collections::HashMap;

use dioxus_core::{ElementId, VirtualDom};
use taffy::{AvailableSpace, Dimension, Display, FlexDirection, Size, Style, TaffyTree};

use dom::{Dom, NodeId, NodeKind};
use style::PaintProps;

pub use sink::{CanvasSink, Fill};

/// Re-exported wit-bindgen so a guest needs no direct dependency on it — the
/// [`launch!`] macro invokes `generate!` through this path. Not a stable API.
#[doc(hidden)]
pub use wit_bindgen as __wit_bindgen;

/// Per-text-leaf measurement context handed to taffy.
struct TextCtx {
    text: String,
    family: String,
    size: f32,
    weight: u32,
    italic: bool,
}

/// One cached paint command, in content (layout) space. The replay applies the
/// scroll offset; `PushClip`/`PopClip` bracket a scroll region's ops (their
/// `y` is offset by the active scroll, and they're clipped to the region box).
enum DrawOp {
    Rrect { x: f32, y: f32, w: f32, h: f32, r: f32, color: u32 },
    Text { blob: u32, x: f32, y: f32, color: u32 },
    /// Enter a scroll region: save + clip to `[x,y,w,h]`; ops until `PopClip`
    /// are offset by `scroll_y`.
    PushClip { x: f32, y: f32, w: f32, h: f32 },
    /// Leave the scroll region: restore.
    PopClip,
}

// Pointer/key-listener flags (which dioxus events an element subscribes to).
const F_CLICK: u8 = 1;
const F_DOWN: u8 = 2;
const F_MOVE: u8 = 4;
const F_KEY: u8 = 8; // listens for keydown → focusable text input

struct HitRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    eid: u32,
    flags: u8,
    /// Inside the scroll region → its visible y is `y - scroll_y`, clipped to
    /// the region viewport.
    scrolled: bool,
}

/// In-progress non-slider gesture (tap vs. scroll). A press that isn't on a
/// draggable element starts here; on move past a threshold it becomes a scroll,
/// otherwise on release it's a tap → `click`.
struct Down {
    start_y: f32,
    last_y: f32,
    content_y: f32,
    click_eid: Option<u32>,
    /// The press began inside the scroll region → movement scrolls it.
    can_scroll: bool,
    scrolling: bool,
}

/// Movement (px) past which a press becomes a scroll instead of a tap.
const SCROLL_THRESHOLD: f32 = 16.0;

/// The renderer. Construct with the guest's root component, then drive it from
/// the `renderer` WIT export (`render_frame` / `on_pointer_down` / `on_resize`).
pub struct DomRenderer {
    vdom: VirtualDom,
    dom: Dom,
    /// Set when the VirtualDom may have pending mutations (initial build + after
    /// each dispatched event). Triggers a re-diff + relayout next frame.
    dirty: bool,
    surface: (f32, f32),
    draw_ops: Vec<DrawOp>,
    hits: Vec<HitRect>,
    /// Host blob ids from the current layout, dropped on relayout.
    /// Element capturing an in-progress drag: `(eid, rect x, y, w, h, scrolled)`.
    /// Set on pointer-down over a draggable element; routes moves/ups to it.
    captured: Option<(u32, f32, f32, f32, f32, bool)>,
    /// The focused text-input element (listens for keydown); `on_key` routes
    /// host key events here.
    focused: Option<u32>,
    /// Physical-px height occluded by the soft keyboard while an input is focused
    /// (the IME runs on a bottom overlay surface covering the panel bottom). 0 =
    /// use the default fraction of surface height. Drives scroll keyboard-avoidance.
    keyboard_inset: f32,
    /// Set when focus moves to a new input → scroll it above the keyboard on the
    /// next layout (one-shot; doesn't fight subsequent manual scrolling).
    pending_focus_scroll: bool,
    /// DOM-declared focus reconciliation: a guest that puts a `focused` attr on
    /// its `data-input` elements drives focus authoritatively (so blur/detach
    /// clears it + the keyboard inset); guests that don't keep pointer focus.
    saw_focus_attr: bool,
    dom_focus: Option<u32>,
    /// UI scale factor. Guest styles are authored in logical px; the layout +
    /// paint multiply lengths/fonts by this, and input element-relative coords
    /// are divided by it (so guest coordinate math stays in logical px).
    scale: f32,
    /// The single `overflow:scroll` region (one supported for now). When laid
    /// out: its viewport box `(x,y,w,h)` + content height; `scroll_y` is the
    /// offset, applied at paint-replay + hit-test time (no relayout on scroll).
    scroll_active: bool,
    scroll_vp: (f32, f32, f32, f32),
    scroll_content_h: f32,
    /// Max bottom-edge y of scroll-region content, accumulated during paint_walk
    /// (taffy's `content_size` is unreliable for our nested flex structure).
    scroll_bottom: f32,
    scroll_y: f32,
    /// In-progress tap/scroll gesture (non-slider press).
    down: Option<Down>,
    blobs: Vec<u32>,
    /// `(text, family, size-bits, weight, italic)` → `(w, h)`. Avoids a host
    /// round-trip per text leaf per layout (taffy measures repeatedly).
    measure_cache: HashMap<(String, String, u32, u32, bool), (f32, f32)>,
    /// Idle frame-delay floor (ms) the guest can lower so the host keeps calling
    /// `render_frame` on a cadence — needed by guests that poll an external data
    /// source (e.g. an engine's `poll-events`) rather than being purely
    /// input-driven. Default 60_000 (fully on-demand, like the launcher).
    min_frame_delay: u32,
}

impl DomRenderer {
    pub fn new(app: fn() -> dioxus_core::Element) -> Self {
        events::install();
        DomRenderer {
            vdom: VirtualDom::new(app),
            dom: Dom::new(),
            dirty: true,
            surface: (0.0, 0.0),
            draw_ops: Vec::new(),
            hits: Vec::new(),
            captured: None,
            focused: None,
            keyboard_inset: 0.0,
            pending_focus_scroll: false,
            saw_focus_attr: false,
            dom_focus: None,
            scale: 1.0,
            scroll_active: false,
            scroll_vp: (0.0, 0.0, 0.0, 0.0),
            scroll_content_h: 0.0,
            scroll_bottom: 0.0,
            scroll_y: 0.0,
            down: None,
            blobs: Vec::new(),
            measure_cache: HashMap::new(),
            min_frame_delay: 60_000,
        }
    }

    pub fn on_resize(&mut self, w: f32, h: f32) {
        self.surface = (w, h);
        self.dirty = true;
    }

    /// Set the UI scale factor (1.0 = author px). Triggers a relayout if changed.
    /// The guest can drive this at runtime (e.g. +/- buttons) by calling it each
    /// frame from `render_frame` with a value it owns.
    pub fn set_scale(&mut self, scale: f32) {
        let scale = scale.clamp(0.5, 4.0);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            self.dirty = true;
        }
    }

    /// Task 64 — ms until the next frame this UI wants, for the host's
    /// on-demand-render gate. `0` while there's pending VirtualDom work to
    /// flush (an event was injected since the last render); otherwise idle
    /// — clamped by the host. The renderer is purely event-driven (no async
    /// runtime / timers), so `dirty` fully captures "needs another frame".
    pub fn next_frame_delay(&self) -> u32 {
        if self.dirty {
            0
        } else {
            self.min_frame_delay
        }
    }

    /// Lower the idle frame-delay floor (ms) so the host keeps calling
    /// `render_frame` on a cadence even when nothing is dirty — for a guest that
    /// must poll an external source (e.g. an engine's `poll-events`) every frame.
    /// Pass `60_000` to return to fully on-demand. Call from the `pre_frame` hook.
    pub fn set_min_frame_delay(&mut self, ms: u32) {
        self.min_frame_delay = ms;
    }

    /// Force a re-diff + relayout on the next `render_frame` (e.g. after external
    /// data — engine events — changed the model the components read). The guest's
    /// root must also re-run (call `dioxus::prelude::needs_update()` in it) for
    /// new model data to reach the tree.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Paint a frame. Re-diffs + relayouts only when dirty, then replays the
    /// cached draw list every frame (cheap; mirrors the launcher).
    pub fn render_frame<S: CanvasSink>(&mut self, sink: &mut S) {
        if self.surface.0 == 0.0 {
            let (w, h) = sink.surface_size();
            self.surface = (w, h);
        }
        if self.dirty {
            // Apply the initial build the first time, incremental diffs after.
            if self.blobs.is_empty() && self.dom.root_children().is_empty() {
                self.vdom.rebuild(&mut self.dom);
            } else {
                self.vdom.process_events();
                self.vdom.render_immediate(&mut self.dom);
            }
            self.relayout(sink);
            self.dirty = false;
        }

        // Ops are in content space. Outside a scroll region the active offset is
        // 0; between PushClip/PopClip it's scroll_y (and a clip is applied), so
        // the region's content scrolls under a sticky header. Scrolling is a
        // pure replay change — no relayout.
        let sy = self.scroll_y;
        let mut active = 0.0f32;
        sink.begin_frame();
        sink.clear(0xFF12121A);
        for op in &self.draw_ops {
            match *op {
                DrawOp::Rrect { x, y, w, h, r, color } => {
                    sink.fill_rrect(x, y - active, w, h, r, r, Fill { color });
                }
                DrawOp::Text { blob, x, y, color } => {
                    sink.draw_text_blob(blob, x, y - active, Fill { color });
                }
                DrawOp::PushClip { x, y, w, h } => {
                    sink.save();
                    sink.clip_rect(x, y, w, h);
                    active = sy;
                }
                DrawOp::PopClip => {
                    sink.restore();
                    active = 0.0;
                }
            }
        }
        // Scrollbar indicator for the scroll region.
        let max = self.max_scroll();
        if self.scroll_active {
            let (vx, vy, vw, vh) = self.scroll_vp;
            let ch = self.scroll_content_h.max(vh);
            let thumb_h = (vh * vh / ch).clamp(48.0, vh);
            let thumb_y = if max > 0.0 { vy + (sy / max) * (vh - thumb_h) } else { vy };
            let bw = 10.0;
            sink.fill_rrect(vx + vw - bw - 6.0, thumb_y, bw, thumb_h, bw / 2.0, bw / 2.0, Fill { color: 0x70FF_FFFF });
        }
        sink.end_frame();
    }

    /// Pointer down → hit-test the cached layout (top-most clickable rect under
    /// the point) → dispatch a dioxus `click` → mark dirty for re-render.
    /// Dispatch a dioxus event to an element. `(x,y)` are surface-absolute;
    /// `(ex,ey)` are relative to the element's rect (what sliders/pickers read
    /// via `event.element_coordinates()`).
    fn dispatch(&self, name: &str, eid: u32, x: f32, y: f32, ex: f32, ey: f32) {
        // Element-relative coords go back to the guest in LOGICAL px (the guest
        // authors + does its slider/picker math in logical units).
        let (ex, ey) = (ex / self.scale, ey / self.scale);
        #[allow(deprecated)]
        self.vdom
            .handle_event(name, events::mouse_event(x, y, ex, ey), ElementId(eid as usize), true);
    }

    /// Hit-test at SCREEN point `(x,y)`. Scrolled hits use their visible rect
    /// (`y - scroll_y`) and must fall within the scroll viewport. Returns the
    /// element + whether it's scrolled (the caller derives content-y).
    fn hit_at(&self, x: f32, y: f32) -> Option<(u32, u8, f32, f32, f32, f32, bool)> {
        let (vx, vy, vw, vh) = self.scroll_vp;
        self.hits.iter().rev().find_map(|h| {
            if h.scrolled && !(x >= vx && x <= vx + vw && y >= vy && y <= vy + vh) {
                return None; // scrolled content is clipped to the viewport
            }
            let cy = if h.scrolled { y + self.scroll_y } else { y };
            if x >= h.x && x <= h.x + h.w && cy >= h.y && cy <= h.y + h.h {
                Some((h.eid, h.flags, h.x, h.y, h.w, h.h, h.scrolled))
            } else {
                None
            }
        })
    }

    /// Override the soft-keyboard inset (physical px occluded at the panel bottom
    /// while an input is focused). 0 restores the default (a fraction of surface
    /// height). A guest can call this if it knows the real IME overlay height.
    pub fn set_keyboard_inset(&mut self, px: f32) {
        self.keyboard_inset = px.max(0.0);
    }

    /// Bottom inset (physical px) occluded by the soft keyboard — only while an
    /// input is focused. Defaults to 0.45 × surface height (the wart IME overlay
    /// covers ~40% of the panel) when not explicitly set.
    fn kb_inset_px(&self) -> f32 {
        if self.focused.is_none() {
            0.0
        } else if self.keyboard_inset > 0.0 {
            self.keyboard_inset
        } else {
            self.surface.1 * 0.45
        }
    }

    fn max_scroll(&self) -> f32 {
        // While an input is focused the keyboard occludes the bottom inset, so the
        // scrollable range gains that much extra padding (lets the last field rise
        // above the keyboard — standard IME bottom-pad behaviour).
        (self.scroll_content_h - self.scroll_vp.3 + self.kb_inset_px()).max(0.0)
    }

    fn in_scroll_viewport(&self, x: f32, y: f32) -> bool {
        let (vx, vy, vw, vh) = self.scroll_vp;
        self.scroll_active && x >= vx && x <= vx + vw && y >= vy && y <= vy + vh
    }

    pub fn on_pointer_down(&mut self, x: f32, y: f32) {
        let hit = self.hit_at(x, y);
        let new_is_field = matches!(hit, Some((_, flags, ..)) if flags & F_KEY != 0);
        // Tap outside the focused field (empty space, a button, a tab, a slider)
        // blurs it → the guest's `onfocusout` hides the soft keyboard. Re-tapping
        // the same field (or another one) is handled below without an explicit
        // blur (the field's own handler re-targets focus).
        if !new_is_field {
            if let Some(old) = self.focused {
                self.dispatch_focus("focusout", old);
                self.dirty = true;
            }
        }
        match hit {
            // A draggable element (slider/picker) captures the pointer — no
            // scroll/tap gesture; drag goes straight to it.
            Some((eid, flags, hx, hy, hw, hh, scrolled)) if flags & F_MOVE != 0 => {
                let cy = if scrolled { y + self.scroll_y } else { y };
                self.captured = Some((eid, hx, hy, hw, hh, scrolled));
                if flags & F_DOWN != 0 {
                    self.dispatch("mousedown", eid, x, y, x - hx, cy - hy);
                }
                // A draggable that also listens for keys is a text field (drag
                // selects, keys type) → focus it + scroll above the keyboard.
                if flags & F_KEY != 0 {
                    self.focused = Some(eid);
                    self.pending_focus_scroll = true;
                } else {
                    self.focused = None;
                }
                self.dirty = true;
            }
            // Otherwise begin a tap/scroll gesture; defer click to release.
            _ => {
                let (click_eid, focus) = match hit {
                    Some((eid, flags, ..)) => (
                        if flags & F_CLICK != 0 { Some(eid) } else { None },
                        if flags & F_KEY != 0 { Some(eid) } else { None },
                    ),
                    None => (None, None),
                };
                let scrolled = hit.map(|h| h.6).unwrap_or(false);
                let content_y = if scrolled { y + self.scroll_y } else { y };
                self.focused = focus;
                self.down = Some(Down {
                    start_y: y,
                    last_y: y,
                    content_y,
                    click_eid,
                    can_scroll: self.in_scroll_viewport(x, y),
                    scrolling: false,
                });
                if focus.is_some() {
                    self.pending_focus_scroll = true;
                    self.dirty = true;
                }
            }
        }
    }

    /// Dispatch a data-less focus event (`focusout`) to an element by id.
    fn dispatch_focus(&self, name: &str, eid: u32) {
        #[allow(deprecated)]
        self.vdom
            .handle_event(name, events::focus_event(), ElementId(eid as usize), true);
    }

    /// Route a host key event (`on-key-event-v2`) to the focused text input as a
    /// dioxus `keydown`/`keyup`. `code_point` is the Unicode scalar (0 for
    /// non-printable); `key_id` is a Compose-Key id (8=Backspace, 13=Enter, …).
    pub fn on_key(&mut self, down: bool, code_point: u32, key_id: u32) {
        if let Some(eid) = self.focused {
            let name = if down { "keydown" } else { "keyup" };
            #[allow(deprecated)]
            self.vdom.handle_event(
                name,
                events::key_event(code_point, key_id),
                ElementId(eid as usize),
                true,
            );
            self.dirty = true;
        }
    }

    /// Whether a text input currently has focus (the guest uses this to decide
    /// whether to keep the soft keyboard attached).
    pub fn has_focus(&self) -> bool {
        self.focused.is_some()
    }

    /// Surface size in the guest's authoring (logical) units — physical px ÷ the
    /// UI scale. Lets a guest size layout against the panel (e.g. reserve a
    /// bottom strip for the soft keyboard so a fixed bottom bar rides above it).
    pub fn surface_size_logical(&self) -> (f32, f32) {
        let s = self.scale.max(0.001);
        (self.surface.0 / s, self.surface.1 / s)
    }

    pub fn on_pointer_move(&mut self, x: f32, y: f32) {
        if let Some((eid, hx, hy, hw, hh, scrolled)) = self.captured {
            // Slider/picker drag (content-space, clamped to the element box).
            let cy = if scrolled { y + self.scroll_y } else { y };
            let ex = (x - hx).clamp(0.0, hw);
            let ey = (cy - hy).clamp(0.0, hh);
            self.dispatch("mousemove", eid, x, y, ex, ey);
            self.dirty = true;
            return;
        }
        // Otherwise: a press inside the scroll region scrolls it once it moves
        // past the threshold.
        let max = self.max_scroll();
        if let Some(d) = self.down.as_mut() {
            if d.can_scroll && !d.scrolling && (y - d.start_y).abs() > SCROLL_THRESHOLD {
                d.scrolling = true;
            }
            if d.scrolling {
                let dy = y - d.last_y;
                d.last_y = y;
                self.scroll_y = (self.scroll_y - dy).clamp(0.0, max);
                self.dirty = true;
            }
        }
    }

    pub fn on_pointer_up(&mut self, x: f32, y: f32) {
        if let Some((eid, hx, hy, hw, hh, scrolled)) = self.captured.take() {
            let cy = if scrolled { y + self.scroll_y } else { y };
            let ex = (x - hx).clamp(0.0, hw);
            let ey = (cy - hy).clamp(0.0, hh);
            self.dispatch("mouseup", eid, x, y, ex, ey);
            self.dirty = true;
            return;
        }
        // A press that didn't become a scroll is a tap → click on release.
        if let Some(d) = self.down.take() {
            if !d.scrolling {
                if let Some(eid) = d.click_eid {
                    self.dispatch("click", eid, x, d.content_y, 0.0, 0.0);
                    self.dirty = true;
                }
            }
        }
    }

    /// Centre of the first clickable rect. Exposed (doc-hidden) so tests can
    /// click it without reaching into private layout state.
    #[doc(hidden)]
    pub fn first_hit_center(&self) -> Option<(f32, f32)> {
        self.hits.first().map(|h| (h.x + h.w / 2.0, h.y + h.h / 2.0))
    }

    /// Test/debug accessor: `(scroll_y, max_scroll, has_scroll_region)`.
    #[doc(hidden)]
    pub fn scroll_state(&self) -> (f32, f32, bool) {
        (self.scroll_y, self.max_scroll(), self.scroll_active)
    }

    // ── Layout + paint ────────────────────────────────────────────────────

    fn relayout<S: CanvasSink>(&mut self, sink: &mut S) {
        // Drop the previous frame's host blobs.
        for id in self.blobs.drain(..) {
            sink.drop_text_blob(id);
        }
        self.draw_ops.clear();
        self.hits.clear();

        let mut taffy: TaffyTree<TextCtx> = TaffyTree::new();
        let mut map: HashMap<NodeId, taffy::NodeId> = HashMap::new();

        // A flex-column root sized to the surface; the app's roots are children.
        let root_kids: Vec<taffy::NodeId> = self
            .dom
            .root_children()
            .to_vec()
            .iter()
            .map(|&n| self.build_taffy(&mut taffy, &mut map, n, &PaintProps::default(), sink))
            .collect();
        // Root fills the surface (fixed); overflow is handled by inner
        // `overflow:scroll` regions, not the root. The app's root child should
        // `flex-grow:1` to fill it.
        let root_style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            size: Size {
                width: Dimension::Length(self.surface.0),
                height: Dimension::Length(self.surface.1),
            },
            ..Default::default()
        };
        let root = taffy.new_with_children(root_style, &root_kids).unwrap();

        {
            // Bind disjoint fields so the measure closure can borrow the cache
            // while `taffy` is borrowed for the compute call.
            let cache = &mut self.measure_cache;
            taffy
                .compute_layout_with_measure(
                    root,
                    Size {
                        width: AvailableSpace::Definite(self.surface.0),
                        height: AvailableSpace::Definite(self.surface.1),
                    },
                    |known, _avail, _id, ctx, _style| match ctx {
                        Some(t) => {
                            let (w, h) = measure_cached(cache, sink, t);
                            Size {
                                width: known.width.unwrap_or(w),
                                height: known.height.unwrap_or(h),
                            }
                        }
                        None => Size::ZERO,
                    },
                )
                .unwrap();
        }

        // The scroll region (if any) is discovered during paint_walk; reset first.
        self.scroll_active = false;
        // DOM focus reconciliation accumulators (see paint_walk + the struct doc).
        self.saw_focus_attr = false;
        self.dom_focus = None;

        // Walk the laid-out tree (absolute coords) → draw ops + hit rects.
        for &n in &self.dom.root_children().to_vec() {
            self.paint_walk(&taffy, &map, n, 0.0, 0.0, &PaintProps::default(), false, sink);
        }

        // If the guest declares focus on its inputs (a `focused` attr), it's
        // authoritative — so detach/blur clears focus (and the keyboard inset)
        // even when no pointer event re-targeted the renderer.
        if self.saw_focus_attr {
            self.focused = self.dom_focus;
        }

        // Re-clamp the scroll offset to the (possibly changed) content height.
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll());

        // Keyboard-avoidance: when a field just gained focus, scroll it above the
        // soft keyboard (one-shot — leaves later manual scrolling alone).
        if self.pending_focus_scroll {
            self.ensure_focused_visible();
            self.pending_focus_scroll = false;
        }
    }

    /// Scroll the focused text input fully above the soft keyboard (and below the
    /// sticky header), if it's inside the scroll region. No-op when nothing is
    /// focused or there's no scroll region.
    fn ensure_focused_visible(&mut self) {
        let inset = self.kb_inset_px();
        if inset <= 0.0 || !self.scroll_active {
            return;
        }
        let Some(feid) = self.focused else { return };
        let field = self
            .hits
            .iter()
            .find(|h| h.eid == feid && h.scrolled)
            .map(|h| (h.y, h.h));
        let Some((hy, hh)) = field else { return };
        let (_, vy, _, vh) = self.scroll_vp;
        let visible_bottom = vy + vh - inset;
        let margin = 16.0 * self.scale;
        // Field below the keyboard line → scroll down to lift it above.
        let field_bottom = hy + hh - self.scroll_y;
        if field_bottom + margin > visible_bottom {
            let want = self.scroll_y + (field_bottom + margin - visible_bottom);
            self.scroll_y = want.min(self.max_scroll());
        }
        // Field above the viewport top → scroll up to reveal it.
        let field_top = hy - self.scroll_y;
        if field_top < vy {
            self.scroll_y = (self.scroll_y - (vy - field_top)).max(0.0);
        }
    }

    /// Recursively build the taffy tree mirroring the dom arena, carrying
    /// inherited font/colour props down to text leaves for measurement.
    fn build_taffy<S: CanvasSink>(
        &self,
        taffy: &mut TaffyTree<TextCtx>,
        map: &mut HashMap<NodeId, taffy::NodeId>,
        node: NodeId,
        inherited: &PaintProps,
        sink: &mut S,
    ) -> taffy::NodeId {
        let n = self.dom.node(node);
        let tid = match &n.kind {
            NodeKind::Element { style, .. } => {
                let paint = style::to_paint(style, inherited);
                let tstyle = style::to_taffy(style, self.scale);
                let kids: Vec<taffy::NodeId> = n
                    .children
                    .iter()
                    .map(|&c| self.build_taffy(taffy, map, c, &paint, sink))
                    .collect();
                taffy.new_with_children(tstyle, &kids).unwrap()
            }
            NodeKind::Text(text) => taffy
                .new_leaf_with_context(
                    Style::DEFAULT,
                    TextCtx {
                        text: text.clone(),
                        family: "sans-serif".to_string(),
                        size: inherited.font_size * self.scale,
                        weight: inherited.font_weight,
                        italic: inherited.italic,
                    },
                )
                .unwrap(),
            NodeKind::Placeholder => taffy.new_leaf(Style::DEFAULT).unwrap(),
        };
        let _ = sink;
        map.insert(node, tid);
        tid
    }

    fn paint_walk<S: CanvasSink>(
        &mut self,
        taffy: &TaffyTree<TextCtx>,
        map: &HashMap<NodeId, taffy::NodeId>,
        node: NodeId,
        parent_x: f32,
        parent_y: f32,
        inherited: &PaintProps,
        in_scroll: bool,
        sink: &mut S,
    ) {
        let Some(&tid) = map.get(&node) else { return };
        // `display:none` → not painted (taffy gives it a zero layout at (0,0),
        // but the painter must skip it + its subtree too, or its text/children
        // bleed into the top-left corner).
        if let NodeKind::Element { style, .. } = &self.dom.node(node).kind {
            if style.get("display").map(String::as_str) == Some("none") {
                return;
            }
        }
        let layout = taffy.layout(tid).unwrap();
        let x = parent_x + layout.location.x;
        let y = parent_y + layout.location.y;
        let (w, h) = (layout.size.width, layout.size.height);
        // Track the content extent of the active scroll region (robust vs taffy's
        // content_size for nested flex).
        if in_scroll {
            self.scroll_bottom = self.scroll_bottom.max(y + h);
        }

        // Clone what we need before borrowing self mutably for draw ops.
        let n = self.dom.node(node);
        let kind_is_text = matches!(n.kind, NodeKind::Text(_));
        let (paint, children, eid, text, is_scroll, input, focus_attr) = match &n.kind {
            NodeKind::Element { style, listeners, .. } => {
                let p = style::to_paint(style, inherited);
                let mut flags = 0u8;
                for l in listeners {
                    match l.as_str() {
                        "click" => flags |= F_CLICK,
                        "mousedown" => flags |= F_DOWN,
                        "mousemove" => flags |= F_MOVE,
                        "keydown" => flags |= F_KEY,
                        _ => {}
                    }
                }
                let eid = if flags != 0 { n.element_id } else { None };
                // Text-input element: `data-input` marker + value/caret/sel attrs
                // (the renderer draws the value + caret + selection itself).
                // A `focused` attr (if present) drives focus authoritatively.
                let (input, focus_attr) = if style.contains_key("data-input") {
                    let value = style.get("value").cloned().unwrap_or_default();
                    let caret = style.get("caret").and_then(|s| s.parse().ok()).unwrap_or(0usize);
                    let (a, b) = style
                        .get("sel")
                        .and_then(|s| s.split_once(':'))
                        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
                        .unwrap_or((caret, caret));
                    let focus_attr = style.get("focused").map(|f| f == "1");
                    (Some((value, caret, a, b)), focus_attr)
                } else {
                    (None, None)
                };
                (p, n.children.clone(), eid.map(|e| (e, flags)), None, style::is_scroll(style), input, focus_attr)
            }
            NodeKind::Text(t) => (*inherited, Vec::new(), None, Some(t.clone()), false, None, None),
            NodeKind::Placeholder => (*inherited, Vec::new(), None, None, false, None, None),
        };
        let input_eid = eid.map(|(e, _)| e);
        // Apply DOM-declared focus (the `n` borrow of self.dom has ended).
        if let Some(f) = focus_attr {
            self.saw_focus_attr = true;
            if f {
                self.dom_focus = input_eid;
            }
        }

        if !kind_is_text {
            if paint.background != 0 {
                // Negative radius = percent-of-min-dimension (e.g. 50% → circle/pill).
                let r = if paint.radius < 0.0 { w.min(h) * -paint.radius } else { paint.radius * self.scale };
                self.draw_ops.push(DrawOp::Rrect { x, y, w, h, r, color: paint.background });
            }
            if let Some((eid, flags)) = eid {
                self.hits.push(HitRect { x, y, w, h, eid, flags, scrolled: in_scroll });
            }
            if let Some((value, caret, sa, sb)) = input {
                self.paint_input(sink, &value, caret, sa, sb, input_eid, x, y, h, &paint);
            }
        } else if let Some(t) = text {
            if !t.trim().is_empty() {
                let fs = paint.font_size * self.scale;
                let blob = sink.create_text_blob(&t, "sans-serif", fs, paint.font_weight, paint.italic);
                self.blobs.push(blob);
                // Baseline ≈ top + cap height. taffy gave the leaf its measured
                // box; sit the baseline near the bottom of that box.
                let baseline = y + fs;
                self.draw_ops.push(DrawOp::Text { blob, x, y: baseline, color: paint.color });
            }
        }

        // A scroll region: its children are clipped to its box + scrolled. (One
        // region supported; the element's own bg above is drawn unscrolled.)
        if is_scroll {
            self.scroll_active = true;
            self.scroll_vp = (x, y, w, h);
            self.scroll_bottom = y;
            self.draw_ops.push(DrawOp::PushClip { x, y, w, h });
            for c in &children {
                self.paint_walk(taffy, map, *c, x, y, &paint, true, sink);
            }
            self.draw_ops.push(DrawOp::PopClip);
            // Content height = furthest child bottom below the viewport top.
            self.scroll_content_h = (self.scroll_bottom - y).max(h);
        } else {
            for c in children {
                self.paint_walk(taffy, map, c, x, y, &paint, in_scroll, sink);
            }
        }
    }

    /// Width (physical px) of a text run at the given font (cached).
    fn measure_w<S: CanvasSink>(&mut self, sink: &mut S, text: &str, fs: f32, weight: u32, italic: bool) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let ctx = TextCtx { text: text.to_string(), family: "sans-serif".to_string(), size: fs, weight, italic };
        measure_cached(&mut self.measure_cache, sink, &ctx).0
    }

    /// Render a text input's value + selection highlight + caret. The element's
    /// background is already drawn; this adds (behind→front) the selection rect,
    /// the value text, and the caret (when the element is focused). Pixel
    /// positions come from measuring substrings.
    #[allow(clippy::too_many_arguments)]
    fn paint_input<S: CanvasSink>(
        &mut self,
        sink: &mut S,
        value: &str,
        caret: usize,
        sa: usize,
        sb: usize,
        eid: Option<u32>,
        x: f32,
        y: f32,
        h: f32,
        paint: &PaintProps,
    ) {
        let fs = paint.font_size * self.scale;
        let tx = x + 24.0 * self.scale; // left inset for the value
        let baseline = y + h * 0.5 + fs * 0.35;
        let n = value.chars().count();
        let (lo, hi) = (sa.min(sb).min(n), sa.max(sb).min(n));
        if lo < hi {
            let p0: String = value.chars().take(lo).collect();
            let p1: String = value.chars().take(hi).collect();
            let x0 = tx + self.measure_w(sink, &p0, fs, paint.font_weight, paint.italic);
            let x1 = tx + self.measure_w(sink, &p1, fs, paint.font_weight, paint.italic);
            self.draw_ops.push(DrawOp::Rrect {
                x: x0, y: y + h * 0.18, w: (x1 - x0).max(2.0), h: h * 0.64, r: 4.0 * self.scale, color: 0x8042_85F4,
            });
        }
        if !value.is_empty() {
            let blob = sink.create_text_blob(value, "sans-serif", fs, paint.font_weight, paint.italic);
            self.blobs.push(blob);
            self.draw_ops.push(DrawOp::Text { blob, x: tx, y: baseline, color: paint.color });
        }
        if eid.is_some() && self.focused == eid {
            let cp: String = value.chars().take(caret.min(n)).collect();
            let cx = tx + self.measure_w(sink, &cp, fs, paint.font_weight, paint.italic);
            self.draw_ops.push(DrawOp::Rrect {
                x: cx, y: y + h * 0.18, w: 3.0 * self.scale, h: h * 0.64, r: 0.0, color: 0xFF42_85F4,
            });
        }
    }
}

/// Measure a text run, caching by content+font so repeated taffy measure calls
/// and unchanged-across-frames text don't re-cross the WIT boundary.
fn measure_cached<S: CanvasSink>(
    cache: &mut HashMap<(String, String, u32, u32, bool), (f32, f32)>,
    sink: &mut S,
    t: &TextCtx,
) -> (f32, f32) {
    let key = (t.text.clone(), t.family.clone(), t.size.to_bits(), t.weight, t.italic);
    if let Some(&v) = cache.get(&key) {
        return v;
    }
    let v = sink.measure_text(&t.text, &t.family, t.size, t.weight, t.italic);
    cache.insert(key, v);
    v
}
