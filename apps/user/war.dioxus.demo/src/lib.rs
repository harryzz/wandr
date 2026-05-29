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
use crate::my::skiko_gfx::ime;

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
        with_renderer(|r| match kind {
            PointerKind::Down => r.on_pointer_down(x, y),
            PointerKind::Move => r.on_pointer_move(x, y),
            PointerKind::Up => r.on_pointer_up(x, y),
            PointerKind::Scroll => {}
        });
    }

    fn on_key_event_v2(kind: KeyKind, code_point: u32, key_id: u32) {
        with_renderer(|r| r.on_key(matches!(kind, KeyKind::Down), code_point, key_id));
    }

    // Unused inputs.
    fn on_pointer_event(_kind: PointerKind, _x: f32, _y: f32) {}
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

export!(App);

// ── The component gallery ────────────────────────────────────────────────
//
// A tabbed gallery (tabs avoid scrolling — each page fits a screen). Phase 1:
// click-only components. Drag inputs (slider, HSV picker) + the text edit box
// land in later phases (task 61).

const BG: &str = "#12121A";
const CARD: &str = "#1F1F33";
const SUBTLE: &str = "#2A2A44";
const ACCENT: &str = "#4285F4";
const GREEN: &str = "#34A853";
const TEXT: &str = "#FFFFFF";
const MUTED: &str = "#C7C7D9";

const TAB_NAMES: [&str; 5] = ["Inputs", "Pickers", "Calendar", "Color", "Text"];

/// HSV (h in degrees, s/v in 0..1) → 0xFFRRGGBB.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> u32 {
    let c = v * s;
    let h6 = (h / 60.0).rem_euclid(6.0);
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let (r, g, b) = match h6 as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| (((f + m) * 255.0).round() as u32).min(255);
    0xFF00_0000 | (to(r) << 16) | (to(g) << 8) | to(b)
}

fn hex(rgb: u32) -> String {
    format!("#{:06X}", rgb & 0xFFFFFF)
}

fn app() -> Element {
    let tab = use_signal(|| 0usize);
    rsx! {
        div {
            style: "display:flex; flex-direction:column; padding:40px; gap:32px; background:{BG};",
            div { style: "color:{TEXT}; font-size:60px; font-weight:700;", "Dioxus Gallery" }
            TabBar { tab }
            {match tab() {
                0 => rsx! { InputsPanel {} },
                1 => rsx! { PickersPanel {} },
                2 => rsx! { CalendarPanel {} },
                3 => rsx! { ColorPanel {} },
                _ => rsx! { TextPanel {} },
            }}
        }
    }
}

#[component]
fn TabBar(tab: Signal<usize>) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:row; gap:16px;",
            for (i, name) in TAB_NAMES.iter().enumerate() {
                button {
                    style: format!(
                        "display:flex; justify-content:center; padding:24px; border-radius:20px; flex-grow:1; background:{};",
                        if tab() == i { ACCENT } else { CARD }
                    ),
                    onclick: move |_| tab.set(i),
                    div { style: "color:{TEXT}; font-size:34px; font-weight:600;", "{name}" }
                }
            }
        }
    }
}

/// A titled card wrapping a control.
#[component]
fn Card(title: String, children: Element) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:20px; background:{CARD}; padding:32px; border-radius:24px;",
            div { style: "color:{MUTED}; font-size:30px; font-weight:600;", "{title}" }
            {children}
        }
    }
}

#[component]
fn InputsPanel() -> Element {
    let mut checked = use_signal(|| true);
    let mut switch_on = use_signal(|| false);
    let mut radio = use_signal(|| 0usize);
    let mut steps = use_signal(|| 4i32);
    let mut slider = use_signal(|| 50.0f32);
    let radios = ["Small", "Medium", "Large"];
    let pct = format!("{:.0}%", slider());
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:24px;",

            Card { title: "Checkbox",
                button {
                    style: "display:flex; flex-direction:row; align-items:center; gap:24px;",
                    onclick: move |_| checked.toggle(),
                    div {
                        style: format!(
                            "display:flex; justify-content:center; width:56px; height:56px; border-radius:14px; background:{};",
                            if checked() { GREEN } else { SUBTLE }
                        ),
                        if checked() { div { style: "color:{TEXT}; font-size:40px; font-weight:700;", "✓" } }
                    }
                    div { style: "color:{TEXT}; font-size:34px;", "Enable feature" }
                }
            }

            Card { title: "Switch",
                button {
                    style: format!(
                        "display:flex; flex-direction:row; align-items:center; width:120px; height:60px; border-radius:50%; padding:6px; background:{}; justify-content:{};",
                        if switch_on() { ACCENT } else { SUBTLE },
                        if switch_on() { "flex-end" } else { "flex-start" }
                    ),
                    onclick: move |_| switch_on.toggle(),
                    div { style: "width:48px; height:48px; border-radius:50%; background:{TEXT};" }
                }
            }

            Card { title: "Radio group",
                div {
                    style: "display:flex; flex-direction:column; gap:18px;",
                    for (i, label) in radios.iter().enumerate() {
                        button {
                            style: "display:flex; flex-direction:row; align-items:center; gap:20px;",
                            onclick: move |_| radio.set(i),
                            div {
                                style: format!(
                                    "display:flex; justify-content:center; align-items:center; width:48px; height:48px; border-radius:50%; background:{};",
                                    if radio() == i { ACCENT } else { SUBTLE }
                                ),
                                if radio() == i { div { style: "width:20px; height:20px; border-radius:50%; background:{TEXT};" } }
                            }
                            div { style: "color:{TEXT}; font-size:32px;", "{label}" }
                        }
                    }
                }
            }

            Card { title: "Stepper + progress",
                div {
                    style: "display:flex; flex-direction:row; align-items:center; gap:28px;",
                    button {
                        style: "display:flex; justify-content:center; width:72px; height:72px; border-radius:50%; background:{SUBTLE};",
                        onclick: move |_| { if steps() > 0 { steps -= 1; } },
                        div { style: "color:{TEXT}; font-size:48px; font-weight:700;", "−" }
                    }
                    div { style: "display:flex; justify-content:center; width:60px; color:{TEXT}; font-size:40px;", "{steps}" }
                    button {
                        style: "display:flex; justify-content:center; width:72px; height:72px; border-radius:50%; background:{SUBTLE};",
                        onclick: move |_| { if steps() < 10 { steps += 1; } },
                        div { style: "color:{TEXT}; font-size:48px; font-weight:700;", "+" }
                    }
                }
                div {
                    style: "display:flex; flex-direction:row; height:24px; border-radius:12px; background:{SUBTLE};",
                    div { style: format!("height:24px; border-radius:12px; background:{}; width:{}%;", ACCENT, steps() * 10) }
                }
            }

            Card { title: "Slider (drag)",
                div {
                    style: "display:flex; flex-direction:row; align-items:center; width:600px; height:48px; border-radius:24px; background:{SUBTLE};",
                    onmousedown: move |e| slider.set((e.element_coordinates().x as f32 / SLIDER_W * 100.0).clamp(0.0, 100.0)),
                    onmousemove: move |e| slider.set((e.element_coordinates().x as f32 / SLIDER_W * 100.0).clamp(0.0, 100.0)),
                    div { style: format!("height:48px; border-radius:24px; background:{}; width:{}px;", ACCENT, slider() / 100.0 * SLIDER_W) }
                }
                div { style: "color:{TEXT}; font-size:32px;", "{pct}" }
            }
        }
    }
}

const SLIDER_W: f32 = 600.0;

#[component]
fn PickersPanel() -> Element {
    let mut open = use_signal(|| false);
    let mut choice = use_signal(|| 0usize);
    let mut color = use_signal(|| 3usize);
    let options = ["Apple", "Banana", "Cherry", "Date"];
    let swatches = ["#EA4335", "#FBBC05", "#34A853", "#4285F4", "#AB47BC", "#00ACC1", "#FF7043", "#5C6BC0"];
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:24px;",

            Card { title: "Dropdown",
                button {
                    style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between; background:{SUBTLE}; padding:26px; border-radius:16px;",
                    onclick: move |_| open.toggle(),
                    div { style: "color:{TEXT}; font-size:34px;", "{options[choice()]}" }
                    div { style: "color:{MUTED}; font-size:34px;", if open() { "^" } else { "v" } }
                }
                if open() {
                    div {
                        style: "display:flex; flex-direction:column; gap:8px;",
                        for (i, opt) in options.iter().enumerate() {
                            button {
                                style: "display:flex; padding:22px; border-radius:12px; background:{CARD};",
                                onclick: move |_| { choice.set(i); open.set(false); },
                                div { style: "color:{MUTED}; font-size:30px;", "{opt}" }
                            }
                        }
                    }
                }
            }

            Card { title: "Color picker",
                div {
                    style: "display:flex; flex-direction:row; gap:18px; align-items:center;",
                    div { style: format!("width:96px; height:96px; border-radius:24px; background:{};", swatches[color()]) }
                    div { style: "color:{MUTED}; font-size:30px;", "{swatches[color()]}" }
                }
                div {
                    style: "display:flex; flex-direction:row; gap:18px;",
                    for (i, sw) in swatches.iter().enumerate() {
                        button {
                            style: format!(
                                "display:flex; justify-content:center; align-items:center; width:84px; height:84px; border-radius:50%; background:{};",
                                if color() == i { TEXT } else { sw }
                            ),
                            onclick: move |_| color.set(i),
                            div {
                                style: format!(
                                    "width:{}; height:{}; border-radius:50%; background:{};",
                                    if color() == i { "60px" } else { "84px" },
                                    if color() == i { "60px" } else { "84px" }, sw
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CalendarPanel() -> Element {
    let mut month = use_signal(|| 4usize); // 0=Jan … 4=May
    let mut day = use_signal(|| 15i32);
    let names = ["January", "February", "March", "April", "May", "June",
                 "July", "August", "September", "October", "November", "December"];
    let dim: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let days = dim[month()];
    rsx! {
        Card { title: "Calendar",
            div {
                style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between;",
                button {
                    style: "display:flex; justify-content:center; width:64px; height:64px; border-radius:50%; background:{SUBTLE};",
                    onclick: move |_| { let m = month(); month.set(if m == 0 { 11 } else { m - 1 }); },
                    div { style: "color:{TEXT}; font-size:38px;", "<" }
                }
                div { style: "color:{TEXT}; font-size:38px; font-weight:600;", "{names[month()]}" }
                button {
                    style: "display:flex; justify-content:center; width:64px; height:64px; border-radius:50%; background:{SUBTLE};",
                    onclick: move |_| { let m = month(); month.set(if m == 11 { 0 } else { m + 1 }); },
                    div { style: "color:{TEXT}; font-size:38px;", ">" }
                }
            }
            div {
                style: "display:flex; flex-direction:column; gap:12px;",
                for row in 0..((days + 6) / 7) {
                    div {
                        style: "display:flex; flex-direction:row; gap:12px;",
                        for col in 0..7 {
                            {
                                let d = row * 7 + col + 1;
                                if d <= days {
                                    rsx! {
                                        button {
                                            style: format!(
                                                "display:flex; justify-content:center; align-items:center; width:80px; height:80px; border-radius:16px; background:{};",
                                                if day() == d { ACCENT } else { SUBTLE }
                                            ),
                                            onclick: move |_| day.set(d),
                                            div { style: "color:{TEXT}; font-size:30px;", "{d}" }
                                        }
                                    }
                                } else {
                                    rsx! { div { style: "width:80px; height:80px;" } }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// HSV color picker — gradients are discretized into solid cells (hue strip +
// saturation/value grid), so it needs only solid fills + drag, no gradient
// primitive. Drag updates the selected segment/cell indices; the picked colour
// is computed from them and previewed.
const HUE_SEGS: usize = 24;
const SV_COLS: usize = 12;
const SV_ROWS: usize = 8;
const SQ_W: f32 = 648.0;
const SQ_H: f32 = 352.0;
const STRIP_W: f32 = 648.0;

#[component]
fn ColorPanel() -> Element {
    let mut hue_seg = use_signal(|| 16usize);
    let mut sv_col = use_signal(|| 11usize);
    let mut sv_row = use_signal(|| 0usize);

    let hue = hue_seg() as f32 / HUE_SEGS as f32 * 360.0;
    let sat = sv_col() as f32 / (SV_COLS - 1) as f32;
    let val = 1.0 - sv_row() as f32 / (SV_ROWS - 1) as f32;
    let picked = hsv_to_rgb(hue, sat, val);
    let picked_hex = hex(picked);
    let preview_bg = hex(picked);

    rsx! {
        Card { title: "HSV color picker (drag)",
            div {
                style: "display:flex; flex-direction:row; align-items:center; gap:20px;",
                div { style: format!("width:96px; height:96px; border-radius:24px; background:{};", preview_bg) }
                div { style: "color:{MUTED}; font-size:32px;", "{picked_hex}" }
            }

            // Saturation (x) × value (y) grid.
            div {
                style: format!("display:flex; flex-direction:column; width:{}px; height:{}px;", SQ_W, SQ_H),
                onmousedown: move |e| {
                    let ex = e.element_coordinates().x as f32;
                    let ey = e.element_coordinates().y as f32;
                    sv_col.set(((ex / SQ_W * SV_COLS as f32) as usize).min(SV_COLS - 1));
                    sv_row.set(((ey / SQ_H * SV_ROWS as f32) as usize).min(SV_ROWS - 1));
                },
                onmousemove: move |e| {
                    let ex = e.element_coordinates().x as f32;
                    let ey = e.element_coordinates().y as f32;
                    sv_col.set(((ex / SQ_W * SV_COLS as f32) as usize).min(SV_COLS - 1));
                    sv_row.set(((ey / SQ_H * SV_ROWS as f32) as usize).min(SV_ROWS - 1));
                },
                for row in 0..SV_ROWS {
                    div {
                        style: "display:flex; flex-direction:row;",
                        for col in 0..SV_COLS {
                            {
                                let cs = col as f32 / (SV_COLS - 1) as f32;
                                let cv = 1.0 - row as f32 / (SV_ROWS - 1) as f32;
                                let cc = hex(hsv_to_rgb(hue, cs, cv));
                                let selected = sv_col() == col && sv_row() == row;
                                rsx! {
                                    div {
                                        style: format!(
                                            "display:flex; justify-content:center; align-items:center; width:54px; height:44px; background:{};",
                                            if selected { TEXT } else { cc.as_str() }
                                        ),
                                        if selected { div { style: format!("width:38px; height:28px; background:{};", cc) } }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Hue strip.
            div {
                style: format!("display:flex; flex-direction:row; width:{}px; height:56px;", STRIP_W),
                onmousedown: move |e| {
                    let ex = e.element_coordinates().x as f32;
                    hue_seg.set(((ex / STRIP_W * HUE_SEGS as f32) as usize).min(HUE_SEGS - 1));
                },
                onmousemove: move |e| {
                    let ex = e.element_coordinates().x as f32;
                    hue_seg.set(((ex / STRIP_W * HUE_SEGS as f32) as usize).min(HUE_SEGS - 1));
                },
                for seg in 0..HUE_SEGS {
                    {
                        let hc = hex(hsv_to_rgb(seg as f32 / HUE_SEGS as f32 * 360.0, 1.0, 1.0));
                        let selected = hue_seg() == seg;
                        rsx! {
                            div {
                                style: format!(
                                    "display:flex; justify-content:center; align-items:center; width:27px; height:56px; background:{};",
                                    if selected { TEXT } else { hc.as_str() }
                                ),
                                if selected { div { style: format!("width:19px; height:42px; background:{};", hc) } }
                            }
                        }
                    }
                }
            }
        }
    }
}

// Text input with the soft keyboard (task 61 phase 3). Tapping the field
// focuses it (the renderer routes keys here) AND calls ime::notify_editor_attached
// so the host shows the war.ime.keyboard overlay; the IME's keystrokes come back
// via renderer.on-key-event-v2 → on_key → this field's onkeydown.
#[component]
fn TextPanel() -> Element {
    let mut text = use_signal(|| String::from("edit me"));
    let mut focused = use_signal(|| false);
    let display = text();
    rsx! {
        Card { title: "Text field (soft keyboard)",
            div {
                style: format!(
                    "display:flex; flex-direction:row; align-items:center; min-height:96px; padding:28px; border-radius:18px; background:{};",
                    if focused() { "#222A4D" } else { SUBTLE }
                ),
                onclick: move |_| {
                    focused.set(true);
                    let t = text();
                    let n = t.chars().count() as u32;
                    ime::notify_editor_attached("text", "Type here", &t, n, n);
                },
                onkeydown: move |e| {
                    let k = e.key().to_string();
                    if k == "Backspace" {
                        let mut t = text();
                        t.pop();
                        text.set(t);
                    } else if k == "Enter" {
                        focused.set(false);
                        ime::notify_editor_detached();
                    } else if k.chars().count() == 1 {
                        let mut t = text();
                        t.push_str(&k);
                        text.set(t);
                    }
                },
                div { style: "color:{TEXT}; font-size:40px;", "{display}" }
                if focused() {
                    div { style: "width:4px; height:48px; background:{ACCENT};" }
                }
            }
            div {
                style: "color:{MUTED}; font-size:28px;",
                if focused() { "Keyboard active — type; Enter or Done to dismiss" } else { "Tap the field to type" }
            }
            if focused() {
                button {
                    style: "display:flex; justify-content:center; padding:26px; border-radius:16px; background:{ACCENT};",
                    onclick: move |_| {
                        focused.set(false);
                        ime::notify_editor_detached();
                    },
                    div { style: "color:{TEXT}; font-size:32px; font-weight:600;", "Done" }
                }
            }
        }
    }
}
