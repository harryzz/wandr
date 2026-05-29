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
