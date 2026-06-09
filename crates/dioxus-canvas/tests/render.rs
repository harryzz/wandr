//! Host-side validation of the full pipeline against a mock sink: VirtualDom
//! mutations → arena → taffy layout → draw ops, plus click → re-render.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_canvas::{CanvasSink, DomRenderer, Fill};

/// Records what the renderer painted so tests can assert on it.
#[derive(Default)]
struct Recorder {
    blobs: Vec<String>, // text content, indexed by blob id - 1
    drawn_text: Vec<String>,
    rrects: usize,
    rrect_xw: Vec<(f32, f32)>, // (x, w) of each filled rrect (caret = thin one)
}

struct MockSink {
    rec: Rc<RefCell<Recorder>>,
    /// Mimic Skia's `getMaxIntrinsicWidth`, which strips TRAILING whitespace —
    /// the metric quirk that froze the caret on a typed space (default off so
    /// existing tests keep exact char-count metrics).
    strip_trailing: bool,
}

impl MockSink {
    fn new(rec: Rc<RefCell<Recorder>>) -> Self {
        MockSink { rec, strip_trailing: false }
    }
}

impl CanvasSink for MockSink {
    fn surface_size(&mut self) -> (f32, f32) {
        (1000.0, 2000.0)
    }
    fn begin_frame(&mut self) {}
    fn end_frame(&mut self) {}
    fn clear(&mut self, _argb: u32) {}
    fn save(&mut self) {}
    fn restore(&mut self) {}
    fn clip_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}
    fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _f: Fill) {}
    fn fill_rrect(&mut self, x: f32, _y: f32, w: f32, _h: f32, _rx: f32, _ry: f32, _f: Fill) {
        let mut r = self.rec.borrow_mut();
        r.rrects += 1;
        r.rrect_xw.push((x, w));
    }
    fn create_text_blob(&mut self, text: &str, _family: &str, _size: f32, _weight: u32, _italic: bool) -> u32 {
        let mut r = self.rec.borrow_mut();
        r.blobs.push(text.to_string());
        r.blobs.len() as u32
    }
    fn draw_text_blob(&mut self, id: u32, _x: f32, _y: f32, _f: Fill) {
        let text = self.rec.borrow().blobs.get((id - 1) as usize).cloned();
        if let Some(t) = text {
            self.rec.borrow_mut().drawn_text.push(t);
        }
    }
    fn drop_text_blob(&mut self, _id: u32) {}
    fn measure_text(&mut self, text: &str, _family: &str, size: f32, _weight: u32, _italic: bool) -> (f32, f32) {
        // Deterministic stand-in for Skia metrics. `strip_trailing` reproduces
        // Skia's trailing-whitespace stripping (the caret-on-space bug).
        let counted = if self.strip_trailing { text.trim_end() } else { text };
        (counted.chars().count() as f32 * size * 0.5, size * 1.2)
    }

    fn create_image(&mut self, _bytes: &[u8]) -> u32 { 1 }
    fn draw_image_rect(&mut self, _id: u32, _x: f32, _y: f32, _w: f32, _h: f32) {}
}

fn counter_app() -> Element {
    let mut count = use_signal(|| 0);
    rsx! {
        div { style: "display:flex; flex-direction:column; padding:20px; gap:12px; background:#1A1A2E;",
            div { "Dioxus on wandr" }
            div { "count: {count}" }
            button { style: "background:#4285F4; padding:16px;", onclick: move |_| count += 1, "tap" }
        }
    }
}

#[test]
fn renders_and_reacts_to_click() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(counter_app);

    // First frame: initial build + layout + paint.
    let mut sink = MockSink::new(rec.clone());
    r.render_frame(&mut sink);

    {
        let rd = rec.borrow();
        assert!(rd.drawn_text.iter().any(|t| t == "Dioxus on wandr"), "title text drawn: {:?}", rd.drawn_text);
        assert!(rd.drawn_text.iter().any(|t| t == "count: 0"), "initial count drawn: {:?}", rd.drawn_text);
        assert!(rd.drawn_text.iter().any(|t| t == "tap"), "button label drawn");
        assert!(rd.rrects >= 1, "at least the column + button backgrounds filled");
    }

    // There should be exactly one clickable rect (the button).
    let (cx, cy) = r.first_hit_center().expect("button is hit-testable");

    // Tap it (down+up, no movement → click on release), then render: the signal
    // increments and the count text updates via the incremental diff path.
    r.on_pointer_down(cx, cy);
    r.on_pointer_up(cx, cy);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    let mut sink2 = MockSink::new(rec2.clone());
    r.render_frame(&mut sink2);

    let rd = rec2.borrow();
    assert!(rd.drawn_text.iter().any(|t| t == "count: 1"), "count updated after click: {:?}", rd.drawn_text);
    assert!(!rd.drawn_text.iter().any(|t| t == "count: 0"), "old count gone");
}

#[test]
fn drag_coords_stay_logical_under_scale() {
    // At scale 2.0 the 200-logical-px track lays out 400 physical px wide; a tap
    // at physical x=100 must report a LOGICAL element x of 50 (guest math is in
    // logical px), i.e. the renderer divides element-relative coords by scale.
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(slider_app);
    r.set_scale(2.0);
    r.render_frame(&mut MockSink::new(rec.clone()));
    r.on_pointer_down(100.0, 40.0);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink::new(rec2.clone()));
    assert!(rec2.borrow().drawn_text.iter().any(|t| t == "val: 50"), "logical x under 2x scale: {:?}", rec2.borrow().drawn_text);
}

/// A fixed header + an `overflow:scroll` region with content taller than the
/// viewport. Exercises scroll-region detection + drag-to-scroll.
fn scroll_app() -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; height:2000px;",
            // Sticky header (outside the scroll region).
            button { style: "height:200px;", onclick: move |_| {}, div { "header" } }
            // Scroll region: 10 × 400px = 4000px content in an ~1800px viewport.
            div {
                style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1;",
                for i in 0..10 {
                    div { style: "height:400px; flex-shrink:0;", div { "item {i}" } }
                }
            }
        }
    }
}

#[test]
fn scroll_region_scrolls() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(scroll_app);
    r.render_frame(&mut MockSink::new(rec.clone()));

    let (sy0, max, active) = r.scroll_state();
    assert!(active, "scroll region detected");
    assert!(max > 0.0, "content overflows the viewport (max_scroll>0): {max}");
    assert_eq!(sy0, 0.0, "starts unscrolled");

    // Drag up inside the viewport (below the 200px header) past the threshold.
    r.on_pointer_down(500.0, 1000.0);
    r.on_pointer_move(500.0, 850.0);
    r.on_pointer_move(500.0, 700.0);
    let (sy1, _, _) = r.scroll_state();
    assert!(sy1 > 0.0, "dragging up scrolls the region down: {sy1}");
}

/// Mirrors the gallery's NESTED scroll structure: an `overflow:scroll` region
/// whose child is a `flex-shrink:0` content wrapper holding *shrinkable* cards
/// (default flex-shrink). Verifies the wrapper keeps natural height so the
/// cards overflow (the gallery relies on this, not on per-card flex-shrink:0).
fn nested_scroll_app() -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; height:2000px;",
            button { style: "height:200px;", onclick: move |_| {}, div { "header" } }
            div {
                style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1;",
                div {
                    style: "display:flex; flex-direction:column; flex-shrink:0;",
                    for i in 0..10 {
                        div { style: "height:400px;", div { "card {i}" } }
                    }
                }
            }
        }
    }
}

/// Exactly the gallery's chain: flex-grow outer (no explicit height, relies on
/// the synthetic root) → header → scroll div → flex-shrink:0 wrapper → a
/// default-shrink inner column → tall children.
fn gallery_exact_app() -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; height:100%;",
            button { style: "height:200px;", onclick: move |_| {}, div { "h" } }
            div {
                style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1;",
                div {
                    style: "display:flex; flex-direction:column; flex-shrink:0;",
                    div {
                        style: "display:flex; flex-direction:column;",
                        for i in 0..10 {
                            div { style: "height:400px;", div { "c{i}" } }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn gallery_exact_scrolls() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(gallery_exact_app);
    r.render_frame(&mut MockSink::new(rec.clone()));
    let (_, max, active) = r.scroll_state();
    assert!(active, "scroll region detected");
    assert!(max > 0.0, "gallery-exact chain overflows (max={max})");
}

/// A scroll region whose focusable input sits low enough that the soft keyboard
/// would cover it. The `focused` attr (driven by a signal) + `onfocusout` let the
/// renderer reconcile focus authoritatively and blur on tap-outside.
fn kbd_field_app() -> Element {
    let mut focused = use_signal(|| false);
    rsx! {
        div {
            style: "display:flex; flex-direction:column; height:100%;",
            button { style: "height:100px;", onclick: move |_| {}, div { "hdr" } }
            div {
                style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1;",
                div {
                    style: "display:flex; flex-direction:column; flex-shrink:0;",
                    div { style: "height:1400px;", div { "spacer" } }
                    div {
                        "data-input": "1",
                        "value": "x",
                        "caret": "1",
                        "sel": "1:1",
                        "focused": if focused() { "1" } else { "0" },
                        style: "height:80px; background:#333333;",
                        onmousedown: move |_| focused.set(true),
                        onmousemove: move |_| {},
                        onfocusout: move |_| focused.set(false),
                        onkeydown: move |_| {},
                    }
                }
            }
        }
    }
}

#[test]
fn keyboard_avoidance_scrolls_focused_field_then_blurs() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(kbd_field_app);
    r.render_frame(&mut MockSink::new(rec.clone()));
    let (sy0, _, active) = r.scroll_state();
    assert!(active, "scroll region detected");
    assert_eq!(sy0, 0.0, "starts unscrolled");

    // Tap the input (header 100 + spacer 1400 → input at screen ~1540, which is
    // below the keyboard line on a 2000px surface). It should focus + the layout
    // should scroll it up above the keyboard inset.
    r.on_pointer_down(500.0, 1540.0);
    assert!(r.has_focus(), "input focused after tap");
    r.render_frame(&mut MockSink::new(rec.clone()));
    let (sy1, _, _) = r.scroll_state();
    assert!(sy1 > 100.0, "focused field scrolled above the keyboard (scroll_y={sy1})");

    // Tap the header (a non-field) → blur → the guest's onfocusout clears focus.
    r.on_pointer_down(500.0, 50.0);
    r.render_frame(&mut MockSink::new(rec.clone()));
    assert!(!r.has_focus(), "tap-outside blurred the field (keyboard would hide)");
}

#[test]
fn nested_scroll_region_overflows() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(nested_scroll_app);
    r.render_frame(&mut MockSink::new(rec.clone()));
    let (_, max, active) = r.scroll_state();
    assert!(active, "scroll region detected");
    assert!(max > 0.0, "flex-shrink:0 wrapper keeps natural height → overflows (max={max})");
}

/// A text-input component: focusable (listens for keydown), edits a String on
/// Character/Backspace. Exercises focus-on-tap + on_key → keydown routing.
fn input_app() -> Element {
    let mut text = use_signal(String::new);
    rsx! {
        div {
            style: "width:400px; height:60px; background:#333333;",
            onkeydown: move |e| {
                let k = e.key().to_string();
                if k == "Backspace" {
                    let mut t = text();
                    t.pop();
                    text.set(t);
                } else if k.chars().count() == 1 {
                    let mut t = text();
                    t.push_str(&k);
                    text.set(t);
                }
            },
            div { "txt:{text}" }
        }
    }
}

#[test]
fn keyboard_types_into_focused_field() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(input_app);
    r.render_frame(&mut MockSink::new(rec.clone()));

    // Tap to focus, then type "Hi".
    let (cx, cy) = r.first_hit_center().expect("field is focusable");
    r.on_pointer_down(cx, cy);
    assert!(r.has_focus(), "field focused after tap");
    r.on_key(true, 'H' as u32, 0);
    r.on_key(true, 'i' as u32, 0);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink::new(rec2.clone()));
    assert!(rec2.borrow().drawn_text.iter().any(|t| t == "txt:Hi"), "typed Hi: {:?}", rec2.borrow().drawn_text);

    // Backspace (key_id=8) deletes the last char.
    r.on_key(true, 0, 8);
    let rec3 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink::new(rec3.clone()));
    assert!(rec3.borrow().drawn_text.iter().any(|t| t == "txt:H"), "backspace: {:?}", rec3.borrow().drawn_text);
}

/// A checkbox-style component: a clickable row that conditionally renders a
/// checkmark child when `on`. Exercises the add/remove mutation path
/// (`render_immediate` create/replace-placeholder/remove) that toggling
/// components (checkbox, dropdown, tabs) all depend on — which the counter
/// (SetText only) never hits.
fn toggle_app() -> Element {
    let mut on = use_signal(|| false);
    rsx! {
        div {
            style: "display:flex; background:#222;",
            onclick: move |_| on.toggle(),
            if on() {
                div { "CHECK" }
            } else {
                div { "empty" }
            }
        }
    }
}

/// A slider-style component reading element-relative drag coordinates.
/// Exercises pointer capture + mousedown/mousemove dispatch + the
/// element_coordinates() offset that sliders/HSV pickers depend on.
fn slider_app() -> Element {
    let mut val = use_signal(|| 0i32);
    rsx! {
        div {
            style: "width:200px; height:40px; background:#333333;",
            onmousedown: move |e| val.set(e.element_coordinates().x as i32),
            onmousemove: move |e| val.set(e.element_coordinates().x as i32),
            div { "val: {val}" }
        }
    }
}

#[test]
fn drag_reports_element_relative_coords() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(slider_app);
    r.render_frame(&mut MockSink::new(rec.clone()));
    assert!(rec.borrow().drawn_text.iter().any(|t| t == "val: 0"), "FIRST render: {:?}", rec.borrow().drawn_text);

    // Down at surface x=50 (track is at origin, 200 wide) → element x=50.
    r.on_pointer_down(50.0, 20.0);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink::new(rec2.clone()));
    assert!(rec2.borrow().drawn_text.iter().any(|t| t == "val: 50"), "down sets val=50: {:?}", rec2.borrow().drawn_text);

    // Drag to x=150 (captured) → element x=150.
    r.on_pointer_move(150.0, 20.0);
    let rec3 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink::new(rec3.clone()));
    assert!(rec3.borrow().drawn_text.iter().any(|t| t == "val: 150"), "drag sets val=150: {:?}", rec3.borrow().drawn_text);

    // Release → moves after up no longer update.
    r.on_pointer_up(150.0, 20.0);
    r.on_pointer_move(80.0, 20.0);
    let rec4 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink::new(rec4.clone()));
    assert!(rec4.borrow().drawn_text.iter().any(|t| t == "val: 150"), "no update after release: {:?}", rec4.borrow().drawn_text);
}

#[test]
fn conditional_rendering_toggles() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(toggle_app);
    let mut sink = MockSink::new(rec.clone());
    r.render_frame(&mut sink);
    assert!(rec.borrow().drawn_text.iter().any(|t| t == "empty"), "starts unchecked: {:?}", rec.borrow().drawn_text);
    assert!(!rec.borrow().drawn_text.iter().any(|t| t == "CHECK"), "no check yet");

    let (cx, cy) = r.first_hit_center().expect("row is clickable");

    // Toggle on → CHECK appears, empty gone.
    r.on_pointer_down(cx, cy);
    r.on_pointer_up(cx, cy);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink::new(rec2.clone()));
    assert!(rec2.borrow().drawn_text.iter().any(|t| t == "CHECK"), "checked after 1st click: {:?}", rec2.borrow().drawn_text);
    assert!(!rec2.borrow().drawn_text.iter().any(|t| t == "empty"), "empty replaced");

    // Toggle off → back to empty (remove + recreate path).
    r.on_pointer_down(cx, cy);
    r.on_pointer_up(cx, cy);
    let rec3 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink::new(rec3.clone()));
    assert!(rec3.borrow().drawn_text.iter().any(|t| t == "empty"), "unchecked after 2nd click: {:?}", rec3.borrow().drawn_text);
    assert!(!rec3.borrow().drawn_text.iter().any(|t| t == "CHECK"), "check removed");
}

/// A long single run inside a `max-width` box: the renderer must wrap it onto
/// several lines instead of drawing (and clipping) one over-wide line.
fn wrap_app() -> Element {
    rsx! {
        div { style: "display:flex; flex-direction:column; width:1000px;",
            div { style: "max-width:120px;",
                div { style: "white-space:normal;", "alpha bravo charlie delta echo foxtrot" }
            }
        }
    }
}

#[test]
fn long_text_wraps_into_multiple_lines() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(wrap_app);
    r.render_frame(&mut MockSink::new(rec.clone()));
    let rd = rec.borrow();
    // The run was broken into multiple line blobs (mock metrics: 8px/char at the
    // 16px default ⇒ the 38-char run can't fit 120px on one line).
    assert!(rd.drawn_text.len() >= 2, "wrapped into >=2 lines: {:?}", rd.drawn_text);
    assert!(
        !rd.drawn_text.iter().any(|t| t == "alpha bravo charlie delta echo foxtrot"),
        "the full run was not drawn as one over-wide line: {:?}", rd.drawn_text
    );
    // No word is lost across the break.
    let joined = rd.drawn_text.join(" ");
    for w in ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"] {
        assert!(joined.contains(w), "word {w} preserved: {:?}", rd.drawn_text);
    }
}

/// A focused composer whose value ends in a space, caret at the very end — the
/// trailing-space caret case.
fn trailing_space_app() -> Element {
    rsx! {
        div {
            "data-input": "1",
            "value": "ab ",
            "caret": "3",
            "sel": "3:3",
            "focused": "1",
            style: "width:600px; height:80px; background:#333333;",
            onmousedown: move |_| {},
            onkeydown: move |_| {},
        }
    }
}

#[test]
fn caret_advances_past_trailing_space() {
    // The mock strips trailing whitespace from measured widths, like Skia — so a
    // naive prefix measure would put the caret at the width of "ab", freezing it
    // on a typed space. The fix counts the trailing space, so the caret sits past
    // "ab" (≈ the width of 3 glyphs, not 2).
    let mut r = DomRenderer::new(trailing_space_app);
    let mut s0 = MockSink::new(Rc::new(RefCell::new(Recorder::default())));
    s0.strip_trailing = true;
    r.render_frame(&mut s0);
    // Tap the field to focus it (marks dirty → relayout → caret drawn).
    r.on_pointer_down(300.0, 40.0);
    r.on_pointer_up(300.0, 40.0);
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut sink = MockSink::new(rec.clone());
    sink.strip_trailing = true;
    r.render_frame(&mut sink);

    let rd = rec.borrow();
    // The caret is the thin rrect (w≈3); the field background is the wide one.
    let caret_x = rd.rrect_xw.iter().find(|(_, w)| *w < 5.0).map(|(x, _)| *x)
        .expect("a caret rrect was drawn for the focused field");
    // Font size 16 ⇒ 8px/glyph in the mock; left inset 24. "ab" = 16px, "ab " = 24px.
    let inset = 24.0;
    assert!(
        caret_x > inset + 16.0 + 1.0,
        "caret advanced past the trailing space (x={caret_x}, > {} expected)", inset + 16.0
    );
}

/// Without `white-space:normal`, a long run in a narrow box stays a single
/// (clipped) line — wrapping is opt-in, so existing single-line layouts (list
/// rows, labels) keep their height and don't reflow.
fn nowrap_app() -> Element {
    rsx! {
        div { style: "display:flex; flex-direction:column; width:1000px;",
            div { style: "max-width:120px;",
                div { "alpha bravo charlie delta echo foxtrot" }
            }
        }
    }
}

#[test]
fn text_does_not_wrap_without_white_space_normal() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(nowrap_app);
    r.render_frame(&mut MockSink::new(rec.clone()));
    let rd = rec.borrow();
    assert!(
        rd.drawn_text.iter().any(|t| t == "alpha bravo charlie delta echo foxtrot"),
        "the run is drawn as one line by default: {:?}", rd.drawn_text
    );
}

/// A `data-stick-key` scroll region taller than its viewport: opening it pins to
/// the newest content (bottom), unlike a plain scroll region (which opens at top).
fn stick_app() -> Element {
    rsx! {
        div { style: "display:flex; flex-direction:column; height:2000px;",
            div {
                style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1;",
                "data-stick-key": "thread-1",
                for i in 0..10 {
                    div { style: "height:400px; flex-shrink:0;", div { "row {i}" } }
                }
            }
        }
    }
}

#[test]
fn stick_to_bottom_opens_at_end() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(stick_app);
    r.render_frame(&mut MockSink::new(rec.clone()));
    let (sy, max, active) = r.scroll_state();
    assert!(active && max > 0.0, "scroll region overflows (max={max})");
    assert!((sy - max).abs() < 1.0, "stick-key region opens pinned to the bottom: sy={sy} max={max}");
}
