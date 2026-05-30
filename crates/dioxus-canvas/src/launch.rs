//! The `launch!` macro — makes a wart guest from a dioxus app with one line.
//!
//! `dioxus-canvas` stays WIT-agnostic (talks to the host only through
//! [`CanvasSink`](crate::CanvasSink)); this macro is what bridges it to the
//! concrete `my:skiko-gfx` host contract. It **expands in the guest crate**
//! because a wasm component's exported symbols must be compiled into the final
//! cdylib — so the backend is *generated*, never hand-written. The guest's own
//! source stays pure dioxus: components + this one call.
//!
//! What it emits, all in the guest crate (a single `wit_bindgen::generate!` over
//! the full `my:skiko-gfx` world — imports + exports — so there's no
//! split-package conflict):
//!   * the WIT bindings (`crate::my::skiko_gfx::*` imports, `crate::exports::*`),
//!   * a [`CanvasSink`](crate::CanvasSink) over the host `canvas` interface,
//!   * `crate::measure_text` / `crate::editor_attach` / `crate::editor_detach`
//!     — backend host helpers the components call (instead of raw WIT),
//!   * the thread-local [`DomRenderer`](crate::DomRenderer) + the `renderer` /
//!     `frame-pacing` `Guest` impls + the component export.
//!
//! Requires `wit-bindgen` in the guest crate (the export `generate!` runs
//! there). The component source never names it; a different backend would be a
//! different `launch!`-shaped macro, with no change to the components.

/// See the module docs. Usage:
/// ```ignore
/// fn app() -> Element { rsx! { /* ... */ } }
/// dioxus_canvas::launch!(app);
/// // optional per-frame hook (e.g. push a runtime UI scale):
/// // dioxus_canvas::launch!(app, pre_frame: |r| r.set_scale(scale()));
/// ```
#[macro_export]
macro_rules! launch {
    ($root:path $(,)?) => {
        $crate::launch!($root, pre_frame: |_r| {});
    };
    ($root:path, pre_frame: $pre:expr $(,)?) => {
        // The full `my:skiko-gfx` world (imports + exports) in one generate!.
        // Trimmed package (the canonical WIT's `matrix-3x3` is rejected by the
        // guest wit-parser); records/enums verbatim for structural matching
        // against the host's full interface. `pub_export_macro` makes the
        // export-wiring macro `#[macro_export]` so this macro can invoke it.
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
