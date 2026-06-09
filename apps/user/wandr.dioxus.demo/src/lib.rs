//! wandr.dioxus.demo — a reactive dioxus guest (task 59).
//!
//! Pure dioxus: this file is just the UI (`app()` + components) plus the single
//! `dioxus_canvas::launch!` line that selects the wandr backend. No `.wit` file,
//! no `wit_bindgen::generate!`, no `HostSink`, no `export!` — all of that is the
//! library's job (the `wandr` feature). The same component source would compile
//! for a different backend by changing only the feature, not this file.

use std::sync::atomic::{AtomicU32, Ordering};

use dioxus::prelude::*;

/// UI scale factor (f32 bits), driven by the in-app +/- buttons. Read each frame
/// and pushed to the renderer via the `launch!` `pre_frame` hook — a global, not
/// a renderer call from the button, so the button's handler doesn't re-enter the
/// already-borrowed renderer. Init 1.5 (the panel is hi-dpi). 0x3FC00000 = 1.5f32.
static UI_SCALE: AtomicU32 = AtomicU32::new(0x3FC0_0000);

// The entire wandr backend (WIT imports + exports, sink, IME bridge, renderer
// wiring) comes from the library. The only app-specific render-time behaviour is
// pushing the runtime UI scale each frame.
dioxus_canvas::launch!(app,
    pre_frame: |r| r.set_scale(f32::from_bits(UI_SCALE.load(Ordering::Relaxed))));

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
    let mut scale = use_signal(|| f32::from_bits(UI_SCALE.load(Ordering::Relaxed)));
    let scale_label = format!("{:.2}×", scale());
    rsx! {
        div {
            style: "display:flex; flex-direction:column; padding:40px; gap:32px; background:{BG}; height:100%;",
            // Title + runtime UI-scale control.
            div {
                style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between;",
                div { style: "color:{TEXT}; font-size:60px; font-weight:700;", "Dioxus Gallery" }
                div {
                    style: "display:flex; flex-direction:row; align-items:center; gap:16px;",
                    button {
                        style: "display:flex; justify-content:center; width:64px; height:64px; border-radius:50%; background:{SUBTLE};",
                        onclick: move |_| {
                            let s = (scale() - 0.25).clamp(0.75, 3.0);
                            scale.set(s);
                            UI_SCALE.store(s.to_bits(), Ordering::Relaxed);
                        },
                        div { style: "color:{TEXT}; font-size:40px; font-weight:700;", "−" }
                    }
                    div { style: "display:flex; justify-content:center; width:120px; color:{TEXT}; font-size:34px;", "{scale_label}" }
                    button {
                        style: "display:flex; justify-content:center; width:64px; height:64px; border-radius:50%; background:{SUBTLE};",
                        onclick: move |_| {
                            let s = (scale() + 0.25).clamp(0.75, 3.0);
                            scale.set(s);
                            UI_SCALE.store(s.to_bits(), Ordering::Relaxed);
                        },
                        div { style: "color:{TEXT}; font-size:40px; font-weight:700;", "+" }
                    }
                }
            }
            TabBar { tab }
            // Sticky header above; the panel scrolls inside this region. The
            // inner wrapper is flex-shrink:0 so it keeps its natural (tall)
            // height and overflows → scrolls.
            div {
                style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1;",
                div {
                    style: "display:flex; flex-direction:column; flex-shrink:0;",
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

/// Editable field font size (logical px) — used both by the renderer (which
/// draws the value + caret + selection) and by char_at below (which measures the
/// same font to map a tap x → caret index). Must match the field's `font-size`.
const FIELD_FS: f32 = 44.0;
/// Left inset of the value inside the input box (matches the renderer's
/// `paint_input` inset). Logical px.
const FIELD_PAD: f32 = 24.0;

/// Measure a text run's width at the field font (logical px) — used to map a
/// tap x to a caret index. Backend-neutral: the library routes it to the host.
fn text_width(s: &str) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    measure_text(s, "sans-serif", FIELD_FS, 400, false).0
}

/// Map an element-relative x (logical px) to the nearest caret index in `value`.
fn char_at(value: &str, x: f32) -> usize {
    let x = (x - FIELD_PAD).max(0.0);
    let chars: Vec<char> = value.chars().collect();
    let mut best = 0usize;
    let mut best_d = f32::MAX;
    for i in 0..=chars.len() {
        let prefix: String = chars[..i].iter().collect();
        let d = (text_width(&prefix) - x).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

// One editable field with selection + the soft keyboard (task 61 phase 3). Tap
// places the caret; drag selects a range; keys edit at the caret/selection. The
// renderer draws the value + caret + selection (the `data-input` element); this
// component owns its own (value, caret, anchor) state. `active`/`idx` give the
// panel single-field focus (tapping one field unfocuses the others — only one
// keyboard). `input-type` drives the IME layout (text / number / phone / email /
// …). Tapping attaches the IME (wandr.ime.keyboard); Escape/Enter/Done detach it
// (the ⌄ "hide" key sends Escape).
#[component]
fn EditField(
    idx: usize,
    active: Signal<i32>,
    input_type: String,
    hint: String,
    label: String,
    initial: String,
) -> Element {
    let init_len = initial.chars().count();
    let mut value = use_signal(|| initial.clone());
    let mut caret = use_signal(|| init_len);
    let mut anchor = use_signal(|| init_len);

    let focused = active() == idx as i32;
    let v = value();
    let sel = format!("{}:{}", anchor(), caret());
    // Cloned for the move-closure (the prop strings are read by `{hint}` below).
    let it = input_type.clone();
    let ht = hint.clone();
    rsx! {
        Card { title: "{label}",
            div {
                "data-input": "1",
                "value": "{v}",
                "caret": "{caret}",
                "sel": "{sel}",
                "focused": if focused { "1" } else { "0" },
                onfocusout: move |_| {
                    // Tap outside the field (renderer-dispatched) → blur + hide kbd.
                    if active() == idx as i32 {
                        active.set(-1);
                        anchor.set(caret());
                        editor_detach();
                    }
                },
                style: format!(
                    "display:flex; height:104px; border-radius:18px; font-size:{}px; color:{}; background:{};",
                    FIELD_FS as i32, TEXT, if focused { "#222A4D" } else { SUBTLE }
                ),
                onmousedown: move |e| {
                    let i = char_at(&value(), e.element_coordinates().x as f32);
                    anchor.set(i);
                    caret.set(i);
                    if active() != idx as i32 {
                        active.set(idx as i32);
                        let t = value();
                        let m = t.chars().count() as u32;
                        editor_attach(&it, &ht, &t, m, m);
                    }
                },
                onmousemove: move |e| {
                    // Drag extends the selection from the anchor.
                    caret.set(char_at(&value(), e.element_coordinates().x as f32));
                },
                onkeydown: move |e| {
                    let k = e.key().to_string();
                    let chars: Vec<char> = value().chars().collect();
                    let lo = anchor().min(caret());
                    let hi = anchor().max(caret());
                    match k.as_str() {
                        "Escape" | "Enter" => {
                            active.set(-1);
                            anchor.set(caret()); // collapse selection so the highlight clears
                            editor_detach();
                        }
                        "Backspace" => {
                            if lo != hi {
                                let mut s: String = chars[..lo].iter().collect();
                                s.extend(&chars[hi..]);
                                value.set(s);
                                anchor.set(lo);
                                caret.set(lo);
                            } else if lo > 0 {
                                let mut s: String = chars[..lo - 1].iter().collect();
                                s.extend(&chars[lo..]);
                                value.set(s);
                                anchor.set(lo - 1);
                                caret.set(lo - 1);
                            }
                        }
                        "ArrowLeft" => {
                            let p = if lo != hi { lo } else { lo.saturating_sub(1) };
                            anchor.set(p);
                            caret.set(p);
                        }
                        "ArrowRight" => {
                            let p = if lo != hi { hi } else { (hi + 1).min(chars.len()) };
                            anchor.set(p);
                            caret.set(p);
                        }
                        _ if k.chars().count() == 1 => {
                            // Replace the selection (or insert at caret) with the char.
                            let mut s: String = chars[..lo].iter().collect();
                            s.push_str(&k);
                            s.extend(&chars[hi..]);
                            value.set(s);
                            anchor.set(lo + 1);
                            caret.set(lo + 1);
                        }
                        _ => {}
                    }
                },
            }
            div {
                style: "color:{MUTED}; font-size:26px;",
                if focused { "Tap=caret · drag=select · type · Esc/Done dismiss" } else { "{hint}" }
            }
            if focused {
                button {
                    style: "display:flex; justify-content:center; padding:22px; border-radius:16px; background:{ACCENT};",
                    onclick: move |_| {
                        active.set(-1);
                        anchor.set(caret()); // collapse selection so the highlight clears
                        editor_detach();
                    },
                    div { style: "color:{TEXT}; font-size:30px; font-weight:600;", "Done" }
                }
            }
        }
    }
}

// Four fields, one per IME layout to exercise (text / number / phone / email).
// A single `active` signal enforces one focused field at a time, so each tap
// re-attaches the IME with that field's `input-type` (the keyboard swaps layout).
#[component]
fn TextPanel() -> Element {
    let active = use_signal(|| -1i32);
    rsx! {
        EditField { idx: 0, active, input_type: "text".to_string(),
            hint: "Text — full QWERTY".to_string(), label: "Text".to_string(), initial: "edit me".to_string() }
        EditField { idx: 1, active, input_type: "number".to_string(),
            hint: "Number — numeric keypad".to_string(), label: "Number".to_string(), initial: "42".to_string() }
        EditField { idx: 2, active, input_type: "phone".to_string(),
            hint: "Phone — dial pad".to_string(), label: "Phone".to_string(), initial: String::new() }
        EditField { idx: 3, active, input_type: "email".to_string(),
            hint: "Email — @ and .com row".to_string(), label: "Email".to_string(), initial: String::new() }
    }
}
