//! war.statusbar — the wart system status bar as a LIGHT Rust canvas
//! guest (task 55). Runs on a top-anchored overlay strip; draws the
//! clock + battery via the canvas WIT, reading them from the
//! `my:skiko-gfx/status` host verbs (local time + sysfs battery,
//! ART-free). No Kotlin/Compose → leak-immune + tiny, which matters for
//! an always-on chrome process.
//!
//! Text is rebuilt only when it changes (clock ~1/min, battery rarely),
//! polled ~1 Hz; otherwise render_frame just replays cached blobs — no
//! per-frame allocation, no animation loop.

wit_bindgen::generate!({
    world: "statusbar-app",
    path: "wit/statusbar.wit",
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::renderer::{Guest, KeyKind, PointerKind};
use crate::my::skiko_gfx::canvas::{
    self, BlendMode, ColorFilterKind, PaintAttrs, PaintStyle, StrokeCap, StrokeJoin,
};
use crate::my::skiko_gfx::status;

const BG: u32 = 0xFF12121C; // opaque dark strip
const FG: u32 = 0xFFECECEC;
const POLL_FRAMES: u64 = 60; // ~1 Hz at 60 fps

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    frame: u64,
    clock: String,
    battery: String,
    clock_blob: Option<u32>,
    batt_blob: Option<u32>,
    label_blob: Option<u32>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

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

const FAMILY: &[u8] = b"sans-serif";

fn blob(text: &str, size: f32, weight: u32) -> u32 {
    canvas::create_text_blob(text.as_bytes(), FAMILY, size, weight, false)
}

struct Bar;

impl Guest for Bar {
    fn render_frame(_nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            if s.w == 0.0 {
                s.w = canvas::surface_width() as f32;
                s.h = canvas::surface_height() as f32;
            }
            // One-time static label. Font sizes are proportional to the
            // bar height (WART_STATUSBAR_PX) so they scale with it.
            if s.label_blob.is_none() {
                s.label_blob = Some(blob("wart", s.h * 0.32, 600));
            }
            // ~1 Hz: refresh clock + battery; rebuild blobs only on change.
            if s.frame % POLL_FRAMES == 0 {
                let clock = status::clock_text();
                if clock != s.clock || s.clock_blob.is_none() {
                    if let Some(b) = s.clock_blob.take() { canvas::drop_text_blob(b); }
                    s.clock_blob = Some(blob(&clock, s.h * 0.42, 600));
                    s.clock = clock;
                }
                let battery = status::battery_text();
                if battery != s.battery || s.batt_blob.is_none() {
                    if let Some(b) = s.batt_blob.take() { canvas::drop_text_blob(b); }
                    s.batt_blob = Some(blob(&battery, s.h * 0.32, 400));
                    s.battery = battery;
                }
            }
            s.frame = s.frame.wrapping_add(1);

            let w = s.w;
            let h = s.h;
            // ~vertically centered for the proportional font sizes.
            let baseline = h * 0.64;
            canvas::begin_frame();
            canvas::clear(BG);
            canvas::draw_rect(0.0, 0.0, w, h, paint(BG));
            if let Some(b) = s.label_blob { canvas::draw_text_blob(b, 40.0, baseline, paint(0xFF8AB4F8)); }
            // Clock centered.
            if let Some(b) = s.clock_blob { canvas::draw_text_blob(b, w * 0.5 - 48.0, baseline, paint(FG)); }
            // Battery right-aligned-ish.
            if let Some(b) = s.batt_blob { canvas::draw_text_blob(b, w - 160.0, baseline, paint(FG)); }
            canvas::end_frame();
        });
    }

    fn on_resize(w: u32, h: u32) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            s.w = w as f32;
            s.h = h as f32;
        });
    }

    fn on_pointer_event(_kind: PointerKind, _x: f32, _y: f32) {}
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_pointer_event_v2(_pid: u32, _kind: PointerKind, _x: f32, _y: f32, _pressure: f32) {}
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

export!(Bar);
