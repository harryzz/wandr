//! The `launch!` macro — makes a wart guest from a dioxus app with one line —
//! plus its two composable halves, `skiko_world!` and `wire!`.
//!
//! `dioxus-canvas` stays WIT-agnostic (talks to the host only through
//! [`CanvasSink`](crate::CanvasSink)); these macros are what bridge it to the
//! concrete `my:skiko-gfx` host contract. They **expand in the guest crate**
//! because a wasm component's exported symbols must be compiled into the final
//! cdylib — so the backend is *generated*, never hand-written. The guest's own
//! source stays pure dioxus: components + one call.
//!
//! Two halves so a guest can add **extra host imports** (e.g. an engine contract
//! like `wart:signal/chat`) without a second `generate!` (which would conflict on
//! `_rt` / `cabi_realloc` / the component-type section):
//!   * [`skiko_world!`] — the single `wit_bindgen::generate!` over the
//!     `my:skiko-gfx` world (imports + exports), emitting `crate::my::skiko_gfx::*`
//!     + `crate::exports::*` + the `__dioxus_canvas_export!` wiring macro.
//!   * [`wire!`] — everything else: a [`CanvasSink`](crate::CanvasSink) over the
//!     host `canvas` interface, the `crate::measure_text` / `editor_attach` /
//!     `editor_detach` helpers, and the thread-local [`DomRenderer`] +
//!     `renderer`/`frame-pacing` `Guest` impls + the component export. It assumes
//!     the `my:skiko-gfx` bindings already exist in the crate, so it works whether
//!     they came from `skiko_world!` or a guest's own combined `generate!`.
//!   * [`launch!`] = `skiko_world!()` + `wire!(app)` — the one-liner for guests
//!     with no extra imports.
//!
//! An engine-backed guest does its own combined `generate!` (the `my:skiko-gfx`
//! world *plus* its extra import, in one `generate!`, with
//! `export_macro_name: "__dioxus_canvas_export"` + `runtime_path:
//! "::dioxus_canvas::__wit_bindgen::rt"`) and then calls `dioxus_canvas::wire!(app)`.
//! See `apps/user/war.signal/ui` for an example.

/// The `my:skiko-gfx` `generate!` (imports `canvas`/`paragraph`/`ime`, exports
/// `renderer`/`frame-pacing`). Pairs with [`wire!`]; together they are [`launch!`].
#[macro_export]
macro_rules! skiko_world {
    () => {
        // Trimmed package (the canonical WIT's `matrix-3x3` is rejected by the
        // guest wit-parser); records/enums verbatim for structural matching
        // against the host's full interface. `pub_export_macro` makes the
        // export-wiring macro `#[macro_export]` so `wire!` can invoke it.
        $crate::__wit_bindgen::generate!({
            inline: r#"
package my:skiko-gfx@0.1.0;

interface canvas {
    enum paint-style { fill, stroke, fill-and-stroke }
    enum stroke-cap  { butt, round, square }
    enum stroke-join { miter, round, bevel }
    enum blend-mode {
        src-over, src, dst-in, dst-out, src-atop, dst-atop, xor,
        multiply, screen, overlay, darken, lighten, color-dodge,
        color-burn, hard-light, soft-light, difference, exclusion, clear,
    }
    enum color-filter-kind { none, blend, invert }
    record paint-attrs {
        color:              u32,
        style:              paint-style,
        stroke-width:       f32,
        stroke-miter:       f32,
        stroke-cap:         stroke-cap,
        stroke-join:        stroke-join,
        anti-alias:         bool,
        alpha:              u8,
        blend-mode:         blend-mode,
        shader-id:          u32,
        color-filter-kind:  color-filter-kind,
        color-filter-color: u32,
    }
    surface-width:  func() -> u32;
    surface-height: func() -> u32;
    begin-frame: func();
    end-frame:   func();
    clear:       func(argb: u32);
    save:       func();
    restore:    func();
    clip-rect:  func(x: f32, y: f32, w: f32, h: f32, anti-alias: bool);
    draw-rect:   func(x: f32, y: f32, w: f32, h: f32, paint: paint-attrs);
    draw-rrect:  func(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32, paint: paint-attrs);
    create-text-blob: func(text: list<u8>, font-family: list<u8>, size: f32, weight: u32, italic: bool) -> u32;
    draw-text-blob:   func(id: u32, x: f32, y: f32, paint: paint-attrs);
    drop-text-blob:   func(id: u32);
    create-image-from-encoded: func(bytes: list<u8>) -> u32;
    get-image-width:  func(image-id: u32) -> u32;
    get-image-height: func(image-id: u32) -> u32;
    draw-image-rect:  func(image-id: u32,
                           src-x: f32, src-y: f32, src-w: f32, src-h: f32,
                           dst-x: f32, dst-y: f32, dst-w: f32, dst-h: f32,
                           paint: paint-attrs);
    drop-image:       func(image-id: u32);
}

interface paragraph {
    record text-style {
        font-size:   f32,
        font-weight: u32,
        italic:      bool,
        color:       u32,
        font-family: list<u8>,
    }
    create-paragraph-builder: func(width: f32) -> u32;
    push-text-style:          func(id: u32, style: text-style);
    add-text:                 func(id: u32, text: list<u8>);
    pop-text-style:           func(id: u32);
    build-paragraph:          func(id: u32) -> u32;
    drop-paragraph-builder:   func(id: u32);
    layout:                   func(id: u32, width: f32);
    get-height:               func(id: u32) -> f32;
    get-max-intrinsic-width:  func(id: u32) -> f32;
    drop-paragraph:           func(id: u32);
}

interface ime {
    notify-editor-attached: func(
        input-type: string,
        hint: string,
        initial-text: string,
        selection-start: u32,
        selection-end: u32,
    );
    notify-editor-detached: func();
}

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

world dioxus-app {
    import canvas;
    import paragraph;
    import ime;
    export renderer;
    export frame-pacing;
}
"#,
            world: "dioxus-app",
            pub_export_macro: true,
            export_macro_name: "__dioxus_canvas_export",
            // Reach the wit-bindgen runtime through dioxus-canvas's re-export,
            // so the guest crate needs no wit-bindgen dependency of its own.
            runtime_path: "::dioxus_canvas::__wit_bindgen::rt",
        });
    };
}

/// The renderer/sink/IME wiring. Assumes the `my:skiko-gfx` bindings already
/// exist in the crate (from [`skiko_world!`] or a guest's own combined
/// `generate!`). Pairs with that; together they are [`launch!`].
#[macro_export]
macro_rules! wire {
    ($root:path $(,)?) => {
        $crate::wire!($root, pre_frame: |_r| {});
    };
    ($root:path, pre_frame: $pre:expr $(,)?) => {
        // ── host canvas adapter (CanvasSink → my:skiko-gfx/canvas) ───────────
        #[doc(hidden)]
        fn __dioxus_canvas_paint(color: u32) -> crate::my::skiko_gfx::canvas::PaintAttrs {
            use crate::my::skiko_gfx::canvas as c;
            c::PaintAttrs {
                color,
                style: c::PaintStyle::Fill,
                stroke_width: 0.0,
                stroke_miter: 4.0,
                stroke_cap: c::StrokeCap::Butt,
                stroke_join: c::StrokeJoin::Miter,
                anti_alias: true,
                alpha: 255,
                blend_mode: c::BlendMode::SrcOver,
                shader_id: 0,
                color_filter_kind: c::ColorFilterKind::None,
                color_filter_color: 0,
            }
        }

        #[doc(hidden)]
        struct __DioxusCanvasSink;
        impl $crate::CanvasSink for __DioxusCanvasSink {
            fn surface_size(&mut self) -> (f32, f32) {
                use crate::my::skiko_gfx::canvas as c;
                (c::surface_width() as f32, c::surface_height() as f32)
            }
            fn begin_frame(&mut self) { crate::my::skiko_gfx::canvas::begin_frame(); }
            fn end_frame(&mut self) { crate::my::skiko_gfx::canvas::end_frame(); }
            fn clear(&mut self, argb: u32) { crate::my::skiko_gfx::canvas::clear(argb); }
            fn save(&mut self) { crate::my::skiko_gfx::canvas::save(); }
            fn restore(&mut self) { crate::my::skiko_gfx::canvas::restore(); }
            fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
                crate::my::skiko_gfx::canvas::clip_rect(x, y, w, h, true);
            }
            fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, f: $crate::Fill) {
                crate::my::skiko_gfx::canvas::draw_rect(x, y, w, h, __dioxus_canvas_paint(f.color));
            }
            fn fill_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32, f: $crate::Fill) {
                crate::my::skiko_gfx::canvas::draw_rrect(x, y, w, h, rx, ry, __dioxus_canvas_paint(f.color));
            }
            fn create_text_blob(&mut self, text: &str, family: &str, size: f32, weight: u32, italic: bool) -> u32 {
                crate::my::skiko_gfx::canvas::create_text_blob(text.as_bytes(), family.as_bytes(), size, weight, italic)
            }
            fn draw_text_blob(&mut self, id: u32, x: f32, y: f32, f: $crate::Fill) {
                crate::my::skiko_gfx::canvas::draw_text_blob(id, x, y, __dioxus_canvas_paint(f.color));
            }
            fn drop_text_blob(&mut self, id: u32) { crate::my::skiko_gfx::canvas::drop_text_blob(id); }
            fn measure_text(&mut self, text: &str, family: &str, size: f32, weight: u32, italic: bool) -> (f32, f32) {
                crate::measure_text(text, family, size, weight, italic)
            }
            fn create_image(&mut self, bytes: &[u8]) -> u32 {
                crate::my::skiko_gfx::canvas::create_image_from_encoded(bytes)
            }
            fn draw_image_rect(&mut self, id: u32, x: f32, y: f32, w: f32, h: f32) {
                use crate::my::skiko_gfx::canvas as c;
                // Full source → dst box (the renderer scales the image to fit).
                let (iw, ih) = (c::get_image_width(id) as f32, c::get_image_height(id) as f32);
                if iw <= 0.0 || ih <= 0.0 { return; }
                c::draw_image_rect(id, 0.0, 0.0, iw, ih, x, y, w, h, __dioxus_canvas_paint(0xFFFF_FFFF));
            }
        }

        // ── backend host helpers the app's components call ───────────────────

        /// Measure a single text run's natural `(width, height)` via the host's
        /// Skia paragraph layout. Components call this instead of raw WIT.
        pub fn measure_text(text: &str, family: &str, size: f32, weight: u32, italic: bool) -> (f32, f32) {
            use crate::my::skiko_gfx::paragraph as p;
            const UNCONSTRAINED: f32 = 1.0e6;
            let b = p::create_paragraph_builder(UNCONSTRAINED);
            p::push_text_style(b, &p::TextStyle {
                font_size: size, font_weight: weight, italic,
                color: 0xFFFF_FFFF, font_family: family.as_bytes().to_vec(),
            });
            p::add_text(b, text.as_bytes());
            p::pop_text_style(b);
            let par = p::build_paragraph(b);
            p::drop_paragraph_builder(b);
            p::layout(par, UNCONSTRAINED);
            let w = p::get_max_intrinsic_width(par);
            let h = p::get_height(par);
            p::drop_paragraph(par);
            (w, h)
        }

        /// Report a focused editor so the host shows the soft keyboard; keys
        /// come back via the renderer's `on-key-event-v2`. `selection` is chars.
        pub fn editor_attach(input_type: &str, hint: &str, initial_text: &str, selection_start: u32, selection_end: u32) {
            crate::my::skiko_gfx::ime::notify_editor_attached(input_type, hint, initial_text, selection_start, selection_end);
        }

        /// The focused editor blurred — reverse of `editor_attach`; hides the kbd.
        pub fn editor_detach() {
            crate::my::skiko_gfx::ime::notify_editor_detached();
        }
        // Images: a guest just renders `img { src: "data:…;base64,…" }` or
        // `img { src: "/assets/…" }`; the renderer (DomRenderer) decodes/reads,
        // caches, and blits via the CanvasSink. No guest-side image API needed.

        // ── renderer + frame-pacing exports → DomRenderer ────────────────────
        ::std::thread_local! {
            static __DIOXUS_CANVAS_RENDERER:
                ::core::cell::RefCell<::core::option::Option<$crate::DomRenderer>> =
                ::core::cell::RefCell::new(::core::option::Option::None);
        }
        fn __dioxus_canvas_with<F: ::core::ops::FnOnce(&mut $crate::DomRenderer)>(f: F) {
            __DIOXUS_CANVAS_RENDERER.with(|cell| {
                let mut slot = cell.borrow_mut();
                if slot.is_none() {
                    *slot = ::core::option::Option::Some($crate::DomRenderer::new($root));
                }
                f(slot.as_mut().unwrap());
            });
        }

        #[doc(hidden)]
        struct __DioxusCanvasGuest;

        impl crate::exports::my::skiko_gfx::renderer::Guest for __DioxusCanvasGuest {
            fn render_frame(_nanos: u64) {
                __dioxus_canvas_with(|r| {
                    let pre: &dyn ::core::ops::Fn(&mut $crate::DomRenderer) = &$pre;
                    pre(r);
                    let mut sink = __DioxusCanvasSink;
                    r.render_frame(&mut sink);
                });
            }
            fn on_resize(w: u32, h: u32) {
                __dioxus_canvas_with(|r| r.on_resize(w as f32, h as f32));
            }
            fn on_pointer_event_v2(
                _pid: u32,
                kind: crate::exports::my::skiko_gfx::renderer::PointerKind,
                x: f32, y: f32, _pressure: f32,
            ) {
                use crate::exports::my::skiko_gfx::renderer::PointerKind;
                __dioxus_canvas_with(|r| match kind {
                    PointerKind::Down => r.on_pointer_down(x, y),
                    PointerKind::Move => r.on_pointer_move(x, y),
                    PointerKind::Up => r.on_pointer_up(x, y),
                    PointerKind::Scroll => {}
                });
            }
            fn on_key_event_v2(
                kind: crate::exports::my::skiko_gfx::renderer::KeyKind,
                code_point: u32, key_id: u32,
            ) {
                use crate::exports::my::skiko_gfx::renderer::KeyKind;
                __dioxus_canvas_with(|r| r.on_key(matches!(kind, KeyKind::Down), code_point, key_id));
            }
            fn on_pointer_event(_kind: crate::exports::my::skiko_gfx::renderer::PointerKind, _x: f32, _y: f32) {}
            fn on_key_event(_kind: crate::exports::my::skiko_gfx::renderer::KeyKind, _key_code: u32) {}
            fn on_scheduled_callback(_callback_id: u32) {}
            fn on_lifecycle_changed(_state: u32) {}
        }

        impl crate::exports::my::skiko_gfx::frame_pacing::Guest for __DioxusCanvasGuest {
            fn next_frame_delay() -> u32 {
                __DIOXUS_CANVAS_RENDERER.with(|cell| cell.borrow().as_ref().map_or(0, |d| d.next_frame_delay()))
            }
        }

        __dioxus_canvas_export!(__DioxusCanvasGuest);
    };
}

/// One-liner for a guest with no extra host imports. Usage:
/// ```ignore
/// fn app() -> Element { rsx! { /* ... */ } }
/// dioxus_canvas::launch!(app);
/// // optional per-frame hook (e.g. push a runtime UI scale):
/// // dioxus_canvas::launch!(app, pre_frame: |r| r.set_scale(scale()));
/// ```
/// For a guest that also imports an engine contract, do your own combined
/// `generate!` then call [`wire!`] — see the module docs and `apps/user/war.signal/ui`.
#[macro_export]
macro_rules! launch {
    ($root:path $(,)?) => {
        $crate::launch!($root, pre_frame: |_r| {});
    };
    ($root:path, pre_frame: $pre:expr $(,)?) => {
        $crate::skiko_world!();
        $crate::wire!($root, pre_frame: $pre);
    };
}
