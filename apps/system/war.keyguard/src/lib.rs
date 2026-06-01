//! war.keyguard — the wart lockscreen as a LIGHT Rust canvas guest (like
//! war.statusbar). A full-screen dark surface with a big centered clock and a
//! "swipe up to unlock" hint. Launched as a high-layer overlay
//! (`--standalone-overlay-lock`); the arbiter keyguard module shows it
//! (Role::Lockscreen) while locked. The unlock gesture (swipe → arbiter) is
//! wired in M3; for now it just renders.

wit_bindgen::generate!({
    world: "keyguard-app",
    path: "wit",
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::frame_pacing::Guest as FramePacingGuest;
use crate::exports::my::skiko_gfx::renderer::{Guest as RendererGuest, KeyKind, PointerKind};
use crate::my::skiko_gfx::canvas::{
    self, BlendMode, ColorFilterKind, PaintAttrs, PaintStyle, StrokeCap, StrokeJoin,
};
use crate::my::skiko_gfx::status;

const BG: u32 = 0xFF0A0A12; // near-black lock background
const CLOCK_FG: u32 = 0xFFFFFFFF;
const HINT_FG: u32 = 0xFF9AA0B4;
const FAMILY: &[u8] = b"sans-serif";
const HINT: &str = "swipe up to unlock";

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    clock: String,
    clock_blob: Option<u32>,
    hint_blob: Option<u32>,
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

fn blob(text: &str, size: f32, weight: u32) -> u32 {
    canvas::create_text_blob(text.as_bytes(), FAMILY, size, weight, false)
}

struct Lock;

impl RendererGuest for Lock {
    fn render_frame(_nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            let (w, h) = (s.w.max(1.0), s.h.max(1.0));
            // Big clock — rebuilt only when the minute changes.
            let clock = status::clock_text();
            if clock != s.clock || s.clock_blob.is_none() {
                if let Some(b) = s.clock_blob.take() { canvas::drop_text_blob(b); }
                s.clock_blob = Some(blob(&clock, h * 0.13, 300));
                s.clock = clock;
            }
            if s.hint_blob.is_none() {
                s.hint_blob = Some(blob(HINT, h * 0.030, 500));
            }

            let clock_size = h * 0.13;
            // No measure-text API; approximate centering (~0.27×size per glyph).
            let clock_w = s.clock.chars().count() as f32 * clock_size * 0.27;
            let hint_w = HINT.chars().count() as f32 * (h * 0.030) * 0.27;

            canvas::begin_frame();
            canvas::clear(BG);
            canvas::draw_rect(0.0, 0.0, w, h, paint(BG));
            if let Some(b) = s.clock_blob {
                canvas::draw_text_blob(b, w * 0.5 - clock_w, h * 0.42, paint(CLOCK_FG));
            }
            if let Some(b) = s.hint_blob {
                canvas::draw_text_blob(b, w * 0.5 - hint_w, h * 0.88, paint(HINT_FG));
            }
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

    fn on_pointer_event(_kind: PointerKind, _x: f32, _y: f32) {} // unlock gesture = M3
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_pointer_event_v2(_pid: u32, _kind: PointerKind, _x: f32, _y: f32, _pressure: f32) {}
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

impl FramePacingGuest for Lock {
    fn next_frame_delay() -> u32 {
        1000 // ~1 Hz so the clock stays current
    }
}

export!(Lock);
