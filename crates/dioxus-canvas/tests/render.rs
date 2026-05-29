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
}

struct MockSink {
    rec: Rc<RefCell<Recorder>>,
}

impl CanvasSink for MockSink {
    fn surface_size(&mut self) -> (f32, f32) {
        (1000.0, 2000.0)
    }
    fn begin_frame(&mut self) {}
    fn end_frame(&mut self) {}
    fn clear(&mut self, _argb: u32) {}
    fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _f: Fill) {}
    fn fill_rrect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _rx: f32, _ry: f32, _f: Fill) {
        self.rec.borrow_mut().rrects += 1;
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
        // Deterministic stand-in for Skia metrics.
        (text.chars().count() as f32 * size * 0.5, size * 1.2)
    }
}

fn counter_app() -> Element {
    let mut count = use_signal(|| 0);
    rsx! {
        div { style: "display:flex; flex-direction:column; padding:20px; gap:12px; background:#1A1A2E;",
            div { "Dioxus on wart" }
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
    let mut sink = MockSink { rec: rec.clone() };
    r.render_frame(&mut sink);

    {
        let rd = rec.borrow();
        assert!(rd.drawn_text.iter().any(|t| t == "Dioxus on wart"), "title text drawn: {:?}", rd.drawn_text);
        assert!(rd.drawn_text.iter().any(|t| t == "count: 0"), "initial count drawn: {:?}", rd.drawn_text);
        assert!(rd.drawn_text.iter().any(|t| t == "tap"), "button label drawn");
        assert!(rd.rrects >= 1, "at least the column + button backgrounds filled");
    }

    // There should be exactly one clickable rect (the button).
    let (cx, cy) = r.first_hit_center().expect("button is hit-testable");

    // Click it, then render the next frame: the signal increments and the
    // count text must update via the incremental diff path.
    r.on_pointer_down(cx, cy);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    let mut sink2 = MockSink { rec: rec2.clone() };
    r.render_frame(&mut sink2);

    let rd = rec2.borrow();
    assert!(rd.drawn_text.iter().any(|t| t == "count: 1"), "count updated after click: {:?}", rd.drawn_text);
    assert!(!rd.drawn_text.iter().any(|t| t == "count: 0"), "old count gone");
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
    r.render_frame(&mut MockSink { rec: rec.clone() });

    // Tap to focus, then type "Hi".
    let (cx, cy) = r.first_hit_center().expect("field is focusable");
    r.on_pointer_down(cx, cy);
    assert!(r.has_focus(), "field focused after tap");
    r.on_key(true, 'H' as u32, 0);
    r.on_key(true, 'i' as u32, 0);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink { rec: rec2.clone() });
    assert!(rec2.borrow().drawn_text.iter().any(|t| t == "txt:Hi"), "typed Hi: {:?}", rec2.borrow().drawn_text);

    // Backspace (key_id=8) deletes the last char.
    r.on_key(true, 0, 8);
    let rec3 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink { rec: rec3.clone() });
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
    r.render_frame(&mut MockSink { rec: rec.clone() });

    // Down at surface x=50 (track is at origin, 200 wide) → element x=50.
    r.on_pointer_down(50.0, 20.0);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink { rec: rec2.clone() });
    assert!(rec2.borrow().drawn_text.iter().any(|t| t == "val: 50"), "down sets val=50: {:?}", rec2.borrow().drawn_text);

    // Drag to x=150 (captured) → element x=150.
    r.on_pointer_move(150.0, 20.0);
    let rec3 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink { rec: rec3.clone() });
    assert!(rec3.borrow().drawn_text.iter().any(|t| t == "val: 150"), "drag sets val=150: {:?}", rec3.borrow().drawn_text);

    // Release → moves after up no longer update.
    r.on_pointer_up(150.0, 20.0);
    r.on_pointer_move(80.0, 20.0);
    let rec4 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink { rec: rec4.clone() });
    assert!(rec4.borrow().drawn_text.iter().any(|t| t == "val: 150"), "no update after release: {:?}", rec4.borrow().drawn_text);
}

#[test]
fn conditional_rendering_toggles() {
    let rec = Rc::new(RefCell::new(Recorder::default()));
    let mut r = DomRenderer::new(toggle_app);
    let mut sink = MockSink { rec: rec.clone() };
    r.render_frame(&mut sink);
    assert!(rec.borrow().drawn_text.iter().any(|t| t == "empty"), "starts unchecked: {:?}", rec.borrow().drawn_text);
    assert!(!rec.borrow().drawn_text.iter().any(|t| t == "CHECK"), "no check yet");

    let (cx, cy) = r.first_hit_center().expect("row is clickable");

    // Toggle on → CHECK appears, empty gone.
    r.on_pointer_down(cx, cy);
    let rec2 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink { rec: rec2.clone() });
    assert!(rec2.borrow().drawn_text.iter().any(|t| t == "CHECK"), "checked after 1st click: {:?}", rec2.borrow().drawn_text);
    assert!(!rec2.borrow().drawn_text.iter().any(|t| t == "empty"), "empty replaced");

    // Toggle off → back to empty (remove + recreate path).
    r.on_pointer_down(cx, cy);
    let rec3 = Rc::new(RefCell::new(Recorder::default()));
    r.render_frame(&mut MockSink { rec: rec3.clone() });
    assert!(rec3.borrow().drawn_text.iter().any(|t| t == "empty"), "unchecked after 2nd click: {:?}", rec3.borrow().drawn_text);
    assert!(!rec3.borrow().drawn_text.iter().any(|t| t == "CHECK"), "check removed");
}
