//! war.dioxus.demo — a reactive dioxus guest (task 59) rendered via the
//! `dioxus-canvas` "tiny Blitz" over the canvas WIT.
//!
//! This file is the thin guest shell (mirrors `war.launcher`): it wires the
//! `wit_bindgen`-generated `canvas::*` imports to `dioxus_canvas::CanvasSink`,
//! holds the renderer in a thread-local, and forwards the `renderer` export
//! callbacks. All the actual UI logic is the dioxus `app()` component + the
//! reusable renderer.

wit_bindgen::generate!({
    world: "dioxus-app",
    path: "wit/skiko-gfx.wit",
});

use std::cell::RefCell;

use dioxus::prelude::*;
use dioxus_canvas::{CanvasSink, DomRenderer, Fill};

use crate::exports::my::skiko_gfx::renderer::{Guest, KeyKind, PointerKind};
use crate::my::skiko_gfx::canvas::{
    self, BlendMode, ColorFilterKind, PaintAttrs, PaintStyle, StrokeCap, StrokeJoin,
};
use crate::my::skiko_gfx::paragraph::{self, TextStyle};

/// Build a flat-fill `paint-attrs` from a colour (copied from war.launcher).
fn paint(color: u32) -> PaintAttrs {
    PaintAttrs {
        color,
        style: PaintStyle::Fill,
        stroke_width: 0.0,
        stroke_miter: 4.0,
        stroke_cap: StrokeCap::Butt,
        stroke_join: StrokeJoin::Miter,
        anti_alias: true,
        alpha: 255,
        blend_mode: BlendMode::SrcOver,
        shader_id: 0,
        color_filter_kind: ColorFilterKind::None,
        color_filter_color: 0,
    }
}

/// Forwards the renderer's `CanvasSink` calls to the host canvas WIT imports.
struct HostSink;

impl CanvasSink for HostSink {
    fn surface_size(&mut self) -> (f32, f32) {
        (canvas::surface_width() as f32, canvas::surface_height() as f32)
    }
    fn begin_frame(&mut self) {
        canvas::begin_frame();
    }
    fn end_frame(&mut self) {
        canvas::end_frame();
    }
    fn clear(&mut self, argb: u32) {
        canvas::clear(argb);
    }
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, f: Fill) {
        canvas::draw_rect(x, y, w, h, paint(f.color));
    }
    fn fill_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32, f: Fill) {
        canvas::draw_rrect(x, y, w, h, rx, ry, paint(f.color));
    }
    fn create_text_blob(&mut self, text: &str, family: &str, size: f32, weight: u32, italic: bool) -> u32 {
        canvas::create_text_blob(text.as_bytes(), family.as_bytes(), size, weight, italic)
    }
    fn draw_text_blob(&mut self, id: u32, x: f32, y: f32, f: Fill) {
        canvas::draw_text_blob(id, x, y, paint(f.color));
    }
    fn drop_text_blob(&mut self, id: u32) {
        canvas::drop_text_blob(id);
    }
    fn measure_text(&mut self, text: &str, family: &str, size: f32, weight: u32, italic: bool) -> (f32, f32) {
        // Measure via the host's existing Skia paragraph layout (no bespoke
        // WIT verb): build a single-run paragraph, lay it out unconstrained,
        // and read its natural width + height. The renderer caches results, so
        // this builder/paragraph churn happens once per unique (text,font).
        const UNCONSTRAINED: f32 = 1.0e6;
        let b = paragraph::create_paragraph_builder(UNCONSTRAINED);
        paragraph::push_text_style(
            b,
            &TextStyle {
                font_size: size,
                font_weight: weight,
                italic,
                color: 0xFFFF_FFFF,
                font_family: family.as_bytes().to_vec(),
            },
        );
        paragraph::add_text(b, text.as_bytes());
        paragraph::pop_text_style(b);
        let p = paragraph::build_paragraph(b);
        paragraph::drop_paragraph_builder(b);
        paragraph::layout(p, UNCONSTRAINED);
        let w = paragraph::get_max_intrinsic_width(p);
        let h = paragraph::get_height(p);
        paragraph::drop_paragraph(p);
        (w, h)
    }
}

thread_local! {
    static RENDERER: RefCell<Option<DomRenderer>> = RefCell::new(None);
}

fn with_renderer<F: FnOnce(&mut DomRenderer)>(f: F) {
    RENDERER.with(|r| {
        let mut b = r.borrow_mut();
        if b.is_none() {
            *b = Some(DomRenderer::new(app));
        }
        f(b.as_mut().unwrap());
    });
}

struct App;

impl Guest for App {
    fn render_frame(_nanos: u64) {
        with_renderer(|r| r.render_frame(&mut HostSink));
    }
    fn on_resize(w: u32, h: u32) {
        with_renderer(|r| r.on_resize(w as f32, h as f32));
    }
    fn on_pointer_event_v2(_pid: u32, kind: PointerKind, x: f32, y: f32, _pressure: f32) {
        if matches!(kind, PointerKind::Down) {
            with_renderer(|r| r.on_pointer_down(x, y));
        }
    }

    // Unused inputs.
    fn on_pointer_event(_kind: PointerKind, _x: f32, _y: f32) {}
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

export!(App);

// ── The reactive UI ────────────────────────────────────────────────────────

const ITEMS: [&str; 4] = ["Reactive", "Flexbox", "Host fonts", "No Kotlin"];

fn app() -> Element {
    let mut count = use_signal(|| 0);
    rsx! {
        div {
            style: "display:flex; flex-direction:column; padding:72px; gap:44px; background:#12121A;",
            div { style: "color:#FFFFFF; font-size:80px; font-weight:700;", "Dioxus on wart" }
            div { style: "color:#9AA0FF; font-size:52px;", "count: {count}" }
            button {
                style: "display:flex; background:#4285F4; padding:52px; border-radius:36px; justify-content:center;",
                onclick: move |_| count += 1,
                div { style: "color:#FFFFFF; font-size:56px; font-weight:600;", "Tap to increment" }
            }
            div {
                style: "display:flex; flex-direction:column; gap:24px;",
                for item in ITEMS.iter() {
                    div {
                        style: "display:flex; background:#1F1F33; padding:40px; border-radius:24px;",
                        div { style: "color:#E0E0E0; font-size:46px;", "• {item}" }
                    }
                }
            }
        }
    }
}
