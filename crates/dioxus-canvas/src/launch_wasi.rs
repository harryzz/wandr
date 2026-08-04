//! The **wasi:canvas backend** for dioxus-canvas (Phase B: fully
//! new-style): [`launch_wasi_canvas!`] is a drop-in alternative to
//! `launch!` that implements the same `CanvasSink` over the
//! `wasi:canvas@0.0.2` draft (proposals/wasi-canvas) and exports the
//! `wasi:input-handlers@0.0.2` trio plus `wandr:ui-shell`
//! shell-events/frame-pacing. No `my:skiko-gfx` legs remain — the host
//! treats the legacy renderer world as an optional probe since the
//! Phase B host prep.
//!
//! Mapping notes (the interesting deltas vs the legacy sink):
//! - Text blobs → host-shaped paragraphs (`wasi:canvas/layout`): the sink
//!   stores the blob SPEC and lazily builds one paragraph per
//!   (blob, color) on first draw — paragraph color is baked at build,
//!   blob color arrives at draw. Blobs draw at a BASELINE origin,
//!   paragraphs paint at top-left: `top = y - alphabetic-baseline`.
//! - `measure-text` → a throwaway layout paragraph (max-intrinsic-width +
//!   height), same recipe the legacy `my:skiko-gfx/paragraph` path used.
//! - Images → `graphics.decode-image` resources behind guest-side u32 ids
//!   (the sink trait speaks ids; resources stay in a thread_local map).
//! - Frames → `embedding.begin-frame`/`end-frame`; the frame's canvas
//!   handle lives in a thread_local for the sink calls between them.

/// Reverse of the host's key-id mapping (Compose-webMain constants) for
/// feeding `DomRenderer::on_key` from a W3C UIEvents code token.
#[doc(hidden)]
pub fn w3c_code_to_key_id(code: &str) -> u32 {
    match code {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" | "NumpadEnter" => 13,
        "Escape" => 27,
        "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        _ => 0,
    }
}

/// Generates the guest bindings for the wasi:canvas backend:
/// 1. at the invocation scope — the `wandr:ui-shell` slice this backend
///    consumes (ime import + shell-events/frame-pacing exports),
/// 2. `mod __dioxus_wasi_canvas` — the wasi:canvas draft imports,
/// 3. `mod __dioxus_input` — the wasi:input-handlers exports.
/// (Three `generate!`s can't share a scope — each emits `exports`/`_rt`.)
#[macro_export]
macro_rules! wasi_canvas_world {
    () => {
        $crate::__wit_bindgen::generate!({
            path: [],
            inline: r#"
package wandr:ui-shell@0.1.0;

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

interface lifecycle {
    enum state { initialized, created, started, resumed, paused, stopped, destroyed }
    get-state: func() -> state;
}

interface shell-events {
    use lifecycle.{state};
    on-scheduled-callback: func(callback-id: u32);
    on-lifecycle-changed:  func(new-state: state);
}

interface frame-pacing {
    next-frame-delay: func() -> u32;
}

world dioxus-wasi-app {
    import ime;
    export shell-events;
    export frame-pacing;
}
"#,
            world: "dioxus-wasi-app",
            pub_export_macro: true,
            export_macro_name: "__dioxus_canvas_export",
            runtime_path: "::dioxus_canvas::__wit_bindgen::rt",
        });

        $crate::wasi_canvas_bindings!();
    };
}

/// JUST the wasi:canvas guest bindings (`mod __dioxus_wasi_canvas`) — for
/// split-form apps that run their own combined `generate!` for the
/// my:skiko-gfx world + extra imports (task-manager, connectivity, …):
/// trim canvas/paragraph from your world, invoke this, then
/// `wire_wasi_canvas!`.
#[macro_export]
macro_rules! wasi_canvas_bindings {
    () => {
        #[doc(hidden)]
        pub mod __dioxus_wasi_canvas {
            $crate::__wit_bindgen::generate!({
                path: [],
                inline: r#"
package wasi:canvas@0.0.2;

interface types {
    type color = u32;

    enum blend-mode {
        src-over, src, dst, dst-over, src-in, dst-in, src-out, dst-out,
        src-atop, dst-atop, xor, plus, modulate, multiply, screen,
        overlay, darken, lighten, color-dodge, color-burn, hard-light,
        soft-light, difference, exclusion, clear,
        hue, saturation, color, luminosity,
    }

    enum paint-style { fill, stroke, fill-and-stroke }
    enum stroke-cap  { butt, round, square }
    enum stroke-join { miter, round, bevel }
    enum fill-rule   { nonzero, evenodd }
    enum tile-mode   { clamp, repeat, mirror, decal }

    enum blur-style { normal, solid, outer, inner }
    record mask-blur {
        style: blur-style,
        sigma: f32,
    }

    record point { x: f32, y: f32 }
    record rect  { x: f32, y: f32, width: f32, height: f32 }

    record rounded-rect {
        rect: rect,
        top-left:     point,
        top-right:    point,
        bottom-right: point,
        bottom-left:  point,
    }

    record transform {
        m00: f32, m01: f32, m02: f32,
        m10: f32, m11: f32, m12: f32,
        m20: f32, m21: f32, m22: f32,
    }

    enum filter-mode { nearest, linear }
    enum mipmap-mode { none, nearest, linear }
    record sampling {
        filter: filter-mode,
        mipmap: mipmap-mode,
    }

    resource shader;

    resource image {
        width:  func() -> u32;
        height: func() -> u32;
    }

    record color-blend {
        color: color,
        mode:  blend-mode,
    }

    variant color-filter {
        blend(color-blend),
        invert,
    }

    record paint {
        style:        paint-style,
        color:        color,
        alpha:        u8,
        blend:        blend-mode,
        anti-alias:   bool,
        shader:       option<borrow<shader>>,
        stroke-width: f32,
        stroke-cap:   stroke-cap,
        stroke-join:  stroke-join,
        stroke-miter: f32,
        blur:         option<mask-blur>,
        filter:       option<color-filter>,
    }
}

interface draw {
    use types.{paint, color, rect, rounded-rect, point, transform,
               fill-rule, sampling, image, shader, blend-mode, tile-mode};

    resource picture;

    resource graphics {
        linear-gradient: func(start: point, end: point,
                              stops: list<tuple<f32, color>>,
                              tile: tile-mode,
                              local: option<transform>) -> shader;
        radial-gradient: func(center: point, radius: f32,
                              stops: list<tuple<f32, color>>,
                              tile: tile-mode,
                              local: option<transform>) -> shader;
        sweep-gradient:  func(center: point,
                              start-angle: f32, end-angle: f32,
                              stops: list<tuple<f32, color>>,
                              tile: tile-mode,
                              local: option<transform>) -> shader;
        shader-blend:    func(mode: blend-mode, dst: borrow<shader>,
                              src: borrow<shader>) -> shader;
        image-pattern:   func(image: borrow<image>,
                              tile-x: tile-mode, tile-y: tile-mode,
                              sampling: sampling,
                              local: transform) -> shader;

        decode-image:     func(bytes: list<u8>) -> result<image>;
        image-from-rgba8: func(width: u32, height: u32,
                               pixels: list<u8>) -> result<image>;

        new-offscreen:    func(width: u32, height: u32) -> canvas;
        start-recording:  func(bounds: rect) -> canvas;
    }

    resource canvas {
        width:  func() -> f32;
        height: func() -> f32;

        save:    func();
        save-layer: func(bounds: option<rect>, alpha: u8);
        restore: func();

        translate: func(dx: f32, dy: f32);
        scale:     func(sx: f32, sy: f32);
        rotate:    func(degrees: f32);
        concat:    func(t: transform);

        clip-rect:         func(r: rect, anti-alias: bool);
        clip-rounded-rect: func(rr: rounded-rect, anti-alias: bool);
        clip-path:         func(path: string, rule: fill-rule,
                                anti-alias: bool);

        clear:             func(color: color);
        draw-paint:        func(paint: paint);
        draw-rect:         func(r: rect, paint: paint);
        draw-rounded-rect: func(rr: rounded-rect, paint: paint);
        draw-double-rounded-rect: func(outer: rounded-rect,
                                       inner: rounded-rect, paint: paint);
        draw-oval:         func(bounds: rect, paint: paint);
        draw-line:         func(start: point, end: point, paint: paint);
        draw-arc:          func(bounds: rect, start-angle: f32,
                                sweep-angle: f32, include-center: bool,
                                paint: paint);
        draw-path:         func(path: string, rule: fill-rule,
                                paint: paint);

        draw-image:        func(image: borrow<image>, at: point,
                                sampling: sampling, paint: paint);
        draw-image-rect:   func(image: borrow<image>, src: rect,
                                dst: rect, sampling: sampling,
                                paint: paint);

        finish-recording: static func(c: canvas) -> picture;
        draw-picture:     func(p: borrow<picture>);

        snapshot:      func() -> result<image>;
    }
}

interface layout {
    use types.{color, rect, point};
    use draw.{canvas};

    enum decoration-line-style { solid, double, dotted, dashed, wavy }

    record decoration {
        underline:    bool,
        overline:     bool,
        line-through: bool,
        color:        color,
        style:        decoration-line-style,
        thickness:    f32,
    }

    record text-shadow {
        color:  color,
        offset: point,
        sigma:  f32,
    }

    record text-style {
        family:    string,
        size:      f32,
        weight:    u32,
        italic:    bool,
        color:     color,
        letter-spacing: f32,
        line-height:    f32,
        baseline-shift: f32,
        decoration:     option<decoration>,
        shadows:        list<text-shadow>,
        background:     option<color>,
    }

    enum align { start, center, end, justify }

    record line-metrics {
        start-offset: u32,
        end-offset:   u32,
        end-excluding-whitespace: u32,
        end-including-newline: u32,
        hard-break:   bool,
        ascent:       f32,
        descent:      f32,
        unscaled-ascent: f32,
        height:       f32,
        width:        f32,
        left:         f32,
        baseline:     f32,
        line-number:  u32,
    }

    enum text-direction { ltr, rtl }

    enum rect-height-style {
        tight, max, include-line-spacing-middle,
        include-line-spacing-top, include-line-spacing-bottom, strut,
    }

    enum rect-width-style { tight, max }

    record text-box {
        rect:      rect,
        direction: text-direction,
    }

    resource paragraph {
        layout:               func(width: f32);
        paint:                func(canvas: borrow<canvas>, at: point);
        height:               func() -> f32;
        max-intrinsic-width:  func() -> f32;
        min-intrinsic-width:  func() -> f32;
        alphabetic-baseline:  func() -> f32;
        ideographic-baseline: func() -> f32;
        line-count:           func() -> u32;
        lines:                func() -> list<line-metrics>;
        selection-boxes:      func(start: u32, end: u32,
                                   height: rect-height-style,
                                   width: rect-width-style) -> list<text-box>;
        offset-at:            func(at: point) -> u32;
        word-boundary:        func(offset: u32) -> tuple<u32, u32>;
        did-exceed-max-lines: func() -> bool;
    }

    resource paragraph-builder {
        new:           static func(default-style: text-style)
                       -> paragraph-builder;
        set-align:     func(a: align);
        set-direction: func(d: text-direction);
        set-max-lines: func(n: u32);
        set-ellipsis:  func(e: string);
        push-style:    func(style: text-style);
        pop-style:     func();
        add-text:      func(text: string);
        build:         static func(b: paragraph-builder) -> paragraph;
    }
}

interface embedding {
    use draw.{canvas, graphics};

    resource canvas-context {
        graphics: func() -> graphics;
        get-current-buffer: func() -> canvas;
        present: func();
    }
    get-context: func() -> canvas-context;
}

world canvas-managed-guest {
    import types;
    import draw;
    import layout;
    import embedding;
}
"#,
                world: "canvas-managed-guest",
                runtime_path: "::dioxus_canvas::__wit_bindgen::rt",
            });
        }

    };
}

/// The wasi:canvas `CanvasSink` + exports wiring. Same contract as
/// `wire!` (the `DomRenderer` core is byte-identical); pair with
/// [`wasi_canvas_world!`] or use [`launch_wasi_canvas!`].
#[macro_export]
macro_rules! wire_wasi_canvas {
    ($root:path $(,)?) => {
        $crate::wire_wasi_canvas!($root, pre_frame: |_r| {});
    };
    ($root:path, pre_frame: $pre:expr $(,)?) => {
        use __dioxus_wasi_canvas::wasi::canvas::draw as __wc_draw;
        use __dioxus_wasi_canvas::wasi::canvas::embedding as __wc_embed;
        use __dioxus_wasi_canvas::wasi::canvas::layout as __wc_layout;
        use __dioxus_wasi_canvas::wasi::canvas::types as __wc_types;

        #[doc(hidden)]
        fn __wc_paint(color: u32) -> __wc_types::Paint<'static> {
            __wc_types::Paint {
                style: __wc_types::PaintStyle::Fill,
                color,
                alpha: 255,
                blend: __wc_types::BlendMode::SrcOver,
                anti_alias: true,
                shader: None,
                stroke_width: 0.0,
                stroke_cap: __wc_types::StrokeCap::Butt,
                stroke_join: __wc_types::StrokeJoin::Miter,
                stroke_miter: 4.0,
                blur: None,
        filter: None,
            }
        }

        #[doc(hidden)]
        fn __wc_rect(x: f32, y: f32, w: f32, h: f32) -> __wc_types::Rect {
            __wc_types::Rect { x, y, width: w, height: h }
        }

        #[doc(hidden)]
        struct __WcBlobSpec {
            text: ::std::string::String,
            family: ::std::string::String,
            size: f32,
            weight: u32,
            italic: bool,
        }

        ::std::thread_local! {
            static __WC_FRAME: ::core::cell::RefCell<::core::option::Option<__wc_draw::Canvas>> =
                ::core::cell::RefCell::new(::core::option::Option::None);
            static __WC_CONTEXT: ::core::cell::RefCell<::core::option::Option<__wc_embed::CanvasContext>> =
                ::core::cell::RefCell::new(::core::option::Option::None);
            static __WC_GRAPHICS: ::core::cell::RefCell<::core::option::Option<__wc_draw::Graphics>> =
                ::core::cell::RefCell::new(::core::option::Option::None);
            static __WC_LAST_SIZE: ::core::cell::Cell<(f32, f32)> = ::core::cell::Cell::new((0.0, 0.0));
            static __WC_BLOBS: ::core::cell::RefCell<::std::collections::HashMap<u32, __WcBlobSpec>> =
                ::core::cell::RefCell::new(::std::collections::HashMap::new());
            static __WC_NEXT_BLOB: ::core::cell::Cell<u32> = ::core::cell::Cell::new(1);
            // One laid-out paragraph per (blob, color) — color is baked at
            // paragraph build, but the sink's blob contract colors at draw.
            static __WC_PARAS: ::core::cell::RefCell<
                ::std::collections::HashMap<(u32, u32), (__wc_layout::Paragraph, f32)>> =
                ::core::cell::RefCell::new(::std::collections::HashMap::new());
            static __WC_IMAGES: ::core::cell::RefCell<::std::collections::HashMap<u32, __wc_types::Image>> =
                ::core::cell::RefCell::new(::std::collections::HashMap::new());
            static __WC_NEXT_IMAGE: ::core::cell::Cell<u32> = ::core::cell::Cell::new(1);
        }

        #[doc(hidden)]
        /// The canvas-context (wasi-gfx graphics-context idiom), initialized on
        /// first use — `get-context` is a factory, callable outside frames.
        #[doc(hidden)]
        fn __wc_ctx<R>(f: impl FnOnce(&__wc_embed::CanvasContext) -> R) -> R {
            __WC_CONTEXT.with(|c| {
                if c.borrow().is_none() {
                    *c.borrow_mut() = ::core::option::Option::Some(__wc_embed::get_context());
                }
                f(c.borrow().as_ref().unwrap())
            })
        }

        #[doc(hidden)]
        fn __wc_with_frame<R>(f: impl FnOnce(&__wc_draw::Canvas) -> R) -> ::core::option::Option<R> {
            __WC_FRAME.with(|c| c.borrow().as_ref().map(f))
        }

        /// The `graphics` factory resource, initialized on FIRST use — not
        /// only in `begin_frame`. Resource creation (image decode, shaders)
        /// legitimately happens during relayout, which runs BEFORE the
        /// frame's `begin-frame` (the legacy backend's create verbs were
        /// always-available plain imports; `get-graphics` is likewise
        /// callable outside a frame — it's a factory, not frame state).
        #[doc(hidden)]
        fn __wc_graphics<R>(f: impl FnOnce(&__wc_draw::Graphics) -> R) -> R {
            __WC_GRAPHICS.with(|g| {
                if g.borrow().is_none() {
                    *g.borrow_mut() = ::core::option::Option::Some(__wc_ctx(|x| x.graphics()));
                }
                f(g.borrow().as_ref().unwrap())
            })
        }

        /// Build + layout a paragraph for a blob spec at a color; returns
        /// (paragraph, alphabetic-baseline).
        #[doc(hidden)]
        fn __wc_build_para(spec: &__WcBlobSpec, color: u32) -> (__wc_layout::Paragraph, f32) {
            const UNCONSTRAINED: f32 = 1.0e6;
            let style = __wc_layout::TextStyle {
                family: spec.family.clone(),
                size: spec.size,
                weight: spec.weight,
                italic: spec.italic,
                color,
                letter_spacing: 0.0,
                line_height: 0.0,
                baseline_shift: 0.0,
                decoration: ::core::option::Option::None,
                shadows: ::std::vec::Vec::new(),
                background: ::core::option::Option::None,
            };
            // 0.0.2 setter-form builder; align defaults to start.
            let b = __wc_layout::ParagraphBuilder::new(&style);
            b.add_text(&spec.text);
            let p = __wc_layout::ParagraphBuilder::build(b);
            p.layout(UNCONSTRAINED);
            let baseline = p.alphabetic_baseline();
            (p, baseline)
        }

        #[doc(hidden)]
        struct __DioxusCanvasSink;
        impl $crate::CanvasSink for __DioxusCanvasSink {
            fn surface_size(&mut self) -> (f32, f32) {
                match __wc_with_frame(|c| (c.width(), c.height())) {
                    ::core::option::Option::Some(s) => {
                        __WC_LAST_SIZE.with(|v| v.set(s));
                        s
                    }
                    ::core::option::Option::None => __WC_LAST_SIZE.with(|v| v.get()),
                }
            }
            fn begin_frame(&mut self) {
                let buf = __wc_ctx(|x| x.get_current_buffer());
                // Refresh the out-of-frame size cache HERE: component bodies
                // run BEFORE begin_frame (relayout precedes the draw pass),
                // so `surface_size()` calls from them hit __WC_LAST_SIZE —
                // which otherwise never gets set (post-canvas-context, size
                // only exists on an acquired buffer). Signal's video rects
                // came out (0,0) → zero surface rect → decode-to-buffer →
                // invisible call video.
                __WC_LAST_SIZE.with(|v| v.set((buf.width(), buf.height())));
                __WC_FRAME.with(|c| {
                    *c.borrow_mut() = ::core::option::Option::Some(buf);
                });
            }
            fn end_frame(&mut self) {
                __WC_FRAME.with(|c| {
                    *c.borrow_mut() = ::core::option::Option::None;
                });
                __wc_ctx(|x| x.present());
            }
            fn clear(&mut self, argb: u32) {
                __wc_with_frame(|c| c.clear(argb));
            }
            fn save(&mut self) {
                __wc_with_frame(|c| c.save());
            }
            fn restore(&mut self) {
                __wc_with_frame(|c| c.restore());
            }
            fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
                __wc_with_frame(|c| c.clip_rect(__wc_rect(x, y, w, h), true));
            }
            fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, f: $crate::Fill) {
                __wc_with_frame(|c| c.draw_rect(__wc_rect(x, y, w, h), &__wc_paint(f.color)));
            }
            fn fill_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32, f: $crate::Fill) {
                let corner = __wc_types::Point { x: rx, y: ry };
                let rr = __wc_types::RoundedRect {
                    rect: __wc_rect(x, y, w, h),
                    top_left: corner,
                    top_right: corner,
                    bottom_right: corner,
                    bottom_left: corner,
                };
                __wc_with_frame(|c| c.draw_rounded_rect(rr, &__wc_paint(f.color)));
            }
            fn create_text_blob(&mut self, text: &str, family: &str, size: f32, weight: u32, italic: bool) -> u32 {
                let id = __WC_NEXT_BLOB.with(|n| {
                    let id = n.get();
                    n.set(id.wrapping_add(1).max(1));
                    id
                });
                __WC_BLOBS.with(|b| {
                    b.borrow_mut().insert(id, __WcBlobSpec {
                        text: text.into(),
                        family: family.into(),
                        size,
                        weight,
                        italic,
                    });
                });
                id
            }
            fn draw_text_blob(&mut self, id: u32, x: f32, y: f32, f: $crate::Fill) {
                __WC_PARAS.with(|cache| {
                    let mut cache = cache.borrow_mut();
                    if !cache.contains_key(&(id, f.color)) {
                        let built = __WC_BLOBS.with(|b| {
                            b.borrow().get(&id).map(|spec| __wc_build_para(spec, f.color))
                        });
                        match built {
                            ::core::option::Option::Some(p) => {
                                cache.insert((id, f.color), p);
                            }
                            ::core::option::Option::None => return,
                        }
                    }
                    if let ::core::option::Option::Some((para, baseline)) = cache.get(&(id, f.color)) {
                        let top = y - baseline;
                        __wc_with_frame(|c| para.paint(c, __wc_types::Point { x, y: top }));
                    }
                });
            }
            fn drop_text_blob(&mut self, id: u32) {
                __WC_BLOBS.with(|b| {
                    b.borrow_mut().remove(&id);
                });
                __WC_PARAS.with(|cache| {
                    cache.borrow_mut().retain(|(bid, _), _| *bid != id);
                });
            }
            fn measure_text(&mut self, text: &str, family: &str, size: f32, weight: u32, italic: bool) -> (f32, f32) {
                measure_text(text, family, size, weight, italic)
            }
            fn create_image(&mut self, bytes: &[u8]) -> u32 {
                match __wc_graphics(|g| g.decode_image(bytes)) {
                    ::core::result::Result::Ok(img) => {
                        let id = __WC_NEXT_IMAGE.with(|n| {
                            let id = n.get();
                            n.set(id.wrapping_add(1).max(1));
                            id
                        });
                        __WC_IMAGES.with(|m| {
                            m.borrow_mut().insert(id, img);
                        });
                        id
                    }
                    _ => 0,
                }
            }
            fn draw_image_rect(&mut self, id: u32, x: f32, y: f32, w: f32, h: f32) {
                __WC_IMAGES.with(|m| {
                    let m = m.borrow();
                    let ::core::option::Option::Some(img) = m.get(&id) else { return };
                    let (iw, ih) = (img.width() as f32, img.height() as f32);
                    if iw <= 0.0 || ih <= 0.0 {
                        return;
                    }
                    let sampling = __wc_types::Sampling {
                        filter: __wc_types::FilterMode::Linear,
                        mipmap: __wc_types::MipmapMode::None,
                    };
                    __wc_with_frame(|c| {
                        c.draw_image_rect(
                            img,
                            __wc_rect(0.0, 0.0, iw, ih),
                            __wc_rect(x, y, w, h),
                            sampling,
                            &__wc_paint(0xFFFF_FFFF),
                        )
                    });
                });
            }
        }

        // ── backend host helpers the app's components call ───────────────────

        /// Measure a single text run's natural `(width, height)` via the
        /// host's paragraph layout (`wasi:canvas/layout`).
        pub fn measure_text(text: &str, family: &str, size: f32, weight: u32, italic: bool) -> (f32, f32) {
            const UNCONSTRAINED: f32 = 1.0e6;
            let style = __wc_layout::TextStyle {
                family: family.into(),
                size,
                weight,
                italic,
                color: 0xFFFF_FFFF,
                letter_spacing: 0.0,
                line_height: 0.0,
                baseline_shift: 0.0,
                decoration: ::core::option::Option::None,
                shadows: ::std::vec::Vec::new(),
                background: ::core::option::Option::None,
            };
            let b = __wc_layout::ParagraphBuilder::new(&style);
            b.add_text(text);
            let p = __wc_layout::ParagraphBuilder::build(b);
            p.layout(UNCONSTRAINED);
            (p.max_intrinsic_width(), p.height())
        }

        /// Current surface size in physical px — the live frame's canvas
        /// when inside a frame, else the last seen size. (Replaces direct
        /// `canvas::surface-width/height` reads from the legacy backend.)
        pub fn surface_size() -> (u32, u32) {
            let s = __WC_FRAME.with(|c| c.borrow().as_ref().map(|c| (c.width(), c.height())));
            match s {
                ::core::option::Option::Some(s) => {
                    __WC_LAST_SIZE.with(|v| v.set(s));
                    (s.0 as u32, s.1 as u32)
                }
                ::core::option::Option::None => {
                    let (w, h) = __WC_LAST_SIZE.with(|v| v.get());
                    (w as u32, h as u32)
                }
            }
        }

        /// Report a focused editor so the host shows the soft keyboard.
        pub fn editor_attach(input_type: &str, hint: &str, initial_text: &str, selection_start: u32, selection_end: u32) {
            crate::wandr::ui_shell::ime::notify_editor_attached(input_type, hint, initial_text, selection_start, selection_end);
        }

        /// The focused editor blurred — hides the keyboard.
        pub fn editor_detach() {
            crate::wandr::ui_shell::ime::notify_editor_detached();
        }

        // ── renderer state + exports (legacy fallback AND input-handlers) ────
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

        fn __dioxus_render_frame_impl() {
            __dioxus_canvas_with(|r| {
                let pre: &dyn ::core::ops::Fn(&mut $crate::DomRenderer) = &$pre;
                pre(r);
                let mut sink = __DioxusCanvasSink;
                r.render_frame(&mut sink);
            });
        }

        impl crate::exports::wandr::ui_shell::shell_events::Guest for __DioxusCanvasGuest {
            fn on_scheduled_callback(_callback_id: u32) {}
            fn on_lifecycle_changed(new_state: crate::exports::wandr::ui_shell::shell_events::State) {
                // DomRenderer keys off the host lifecycle ordinals
                // (resumed=3, paused=4, stopped=5) — the enum order matches.
                __dioxus_canvas_with(|r| r.set_lifecycle(new_state as u32));
            }
        }

        impl crate::exports::wandr::ui_shell::frame_pacing::Guest for __DioxusCanvasGuest {
            fn next_frame_delay() -> u32 {
                __DIOXUS_CANVAS_RENDERER.with(|cell| cell.borrow().as_ref().map_or(0, |d| d.next_frame_delay()))
            }
        }

        __dioxus_canvas_export!(__DioxusCanvasGuest);

        // wasi:input-handlers — preferred by new hosts (exclusive routing).
        // Generated + implemented inside one module: wit-bindgen's export
        // macro is textually scoped, so the export! invocation must live
        // beside the generate! (the slint-wandr pattern).
        #[doc(hidden)]
        mod __dioxus_input {
            use super::{__DioxusCanvasGuest, __dioxus_canvas_with, __dioxus_render_frame_impl};
            $crate::__wit_bindgen::generate!({
                path: [],
                inline: r#"
package wasi:input-handlers@0.0.2;

interface pointer-handler {
    enum kind { down, up, move, scroll, cancel, enter, leave }
    enum pointer-device { unknown, mouse, touch, pen }
    enum button { none, primary, secondary, middle, back, forward }
    flags buttons { primary, secondary, middle, back, forward }
    record pointer-event {
        id: u32,
        kind: kind,
        device: pointer-device,
        x: f32,
        y: f32,
        pressure: f32,
        tilt-x: f32,
        tilt-y: f32,
        twist: f32,
        scroll-dx: f32,
        scroll-dy: f32,
        button: button,
        buttons: buttons,
        alt: bool,
        ctrl: bool,
        meta: bool,
        shift: bool,
    }
    on-pointer: func(ev: pointer-event);
}

interface key-handler {
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

interface frame-handler {
    on-frame:  func(nanos: u64);
    on-resize: func(width: u32, height: u32);
}

world input-guest {
    export pointer-handler;
    export key-handler;
    export frame-handler;
}
"#,
                world: "input-guest",
                pub_export_macro: true,
                export_macro_name: "__dioxus_input_export",
                runtime_path: "::dioxus_canvas::__wit_bindgen::rt",
            });

            impl exports::wasi::input_handlers::pointer_handler::Guest for __DioxusCanvasGuest {
                fn on_pointer(ev: exports::wasi::input_handlers::pointer_handler::PointerEvent) {
                    use exports::wasi::input_handlers::pointer_handler::Kind;
                    __dioxus_canvas_with(|r| match ev.kind {
                        Kind::Down => r.on_pointer_down(ev.x, ev.y),
                        Kind::Move => r.on_pointer_move(ev.x, ev.y),
                        Kind::Up | Kind::Cancel => r.on_pointer_up(ev.x, ev.y),
                        // Mouse-wheel / trackpad scroll → the scroll region under
                        // the cursor (desktop; touch guests scroll by drag).
                        Kind::Scroll => r.on_scroll(ev.x, ev.y, ev.scroll_dx, ev.scroll_dy),
                        // Hover has no DomRenderer mapping.
                        Kind::Enter | Kind::Leave => {}
                    });
                }
            }

            impl exports::wasi::input_handlers::key_handler::Guest for __DioxusCanvasGuest {
                fn on_key(ev: exports::wasi::input_handlers::key_handler::KeyEvent) {
                    let code_point = ev.text.chars().next().map(|c| c as u32).unwrap_or(0);
                    let key_id = $crate::w3c_code_to_key_id(&ev.code);
                    __dioxus_canvas_with(|r| r.on_key(ev.down, code_point, key_id));
                }
            }

            impl exports::wasi::input_handlers::frame_handler::Guest for __DioxusCanvasGuest {
                fn on_frame(_nanos: u64) {
                    __dioxus_render_frame_impl();
                }
                fn on_resize(width: u32, height: u32) {
                    // Refresh the out-of-frame size cache to the NEW dims BEFORE the
                    // resize re-runs the component tree. Component bodies run before
                    // begin_frame (which is where __WC_LAST_SIZE is otherwise set, from
                    // the drawn buffer), so a body that reads `surface_size()` during a
                    // resize-triggered relayout would otherwise see the LAST-DRAWN frame's
                    // size — i.e. the pre-rotation size. That made rotation-derived layout
                    // (e.g. Signal's video-call PiP/remote rects via push_video_layout)
                    // compute against the old orientation and land off-screen. Setting it
                    // here makes the very next component run see the post-rotation size.
                    super::__WC_LAST_SIZE.with(|v| v.set((width as f32, height as f32)));
                    __dioxus_canvas_with(|r| r.on_resize(width as f32, height as f32));
                }
            }

            __dioxus_input_export!(__DioxusCanvasGuest);
        }
    };
}

/// One-liner for a wasi:canvas guest: bindings + sink + exports.
/// Drop-in alternative to `launch!` — same app code, the host must serve
/// `wasi:canvas` (default-on since 2026-06-11).
#[macro_export]
macro_rules! launch_wasi_canvas {
    ($root:path $(,)?) => {
        $crate::launch_wasi_canvas!($root, pre_frame: |_r| {});
    };
    ($root:path, pre_frame: $pre:expr $(,)?) => {
        $crate::wasi_canvas_world!();
        $crate::wire_wasi_canvas!($root, pre_frame: $pre);
    };
}
