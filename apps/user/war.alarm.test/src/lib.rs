//! war.alarm.test — minimal test guest for the war:alarm timed-wake primitive
//! (Arbiter Inc. 3c). Imports `war:alarm/scheduler`, exports
//! `war:alarm/alarm-handler`. On its first frame it schedules a repeating alarm
//! (every 5 s); `on-alarm` bumps a counter drawn as a growing green bar (visible
//! proof). The host also logs "dispatched on-alarm". Killing the guest leaves the
//! alarm in the arbiter, which relaunches the guest at the next fire.

wit_bindgen::generate!({
    world: "alarm-test-app",
    path: "wit",
    generate_all,
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::frame_pacing::Guest as FramePacingGuest;
use crate::exports::my::skiko_gfx::renderer::{Guest as RendererGuest, KeyKind, PointerKind};
use crate::exports::war::alarm::alarm_handler::Guest as AlarmGuest;
use crate::exports::war::background::background::Guest as BackgroundGuest;
use crate::my::skiko_gfx::canvas::{
    self, BlendMode, ColorFilterKind, PaintAttrs, PaintStyle, StrokeCap, StrokeJoin,
};
use crate::war::alarm::scheduler;

const ALARM_ID: u64 = 1;
const PERIOD_MS: u64 = 5000;

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    count: u32,
    scheduled: bool,
    bg_count: u32,
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

struct Component;

impl RendererGuest for Component {
    fn render_frame(_nanos: u64) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            // Schedule the repeating alarm once (idempotent on id arbiter-side too).
            if !s.scheduled {
                scheduler::schedule(ALARM_ID, PERIOD_MS, PERIOD_MS);
                s.scheduled = true;
            }
            let (w, h) = (s.w.max(1.0), s.h.max(1.0));
            canvas::begin_frame();
            canvas::clear(0xFF12121C);
            // Growing bar: width tracks the alarm count (wraps to stay on-screen).
            let bar_w = ((s.count as f32) * 24.0) % (w - 40.0).max(24.0);
            canvas::draw_rect(20.0, h * 0.4, bar_w + 24.0, 60.0, paint(0xFF40C040));
            canvas::end_frame();
        });
    }

    fn on_resize(w: u32, h: u32) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.w = w as f32;
            s.h = h as f32;
        });
    }

    fn on_pointer_event(_kind: PointerKind, _x: f32, _y: f32) {}
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_pointer_event_v2(_pointer_id: u32, _kind: PointerKind, _x: f32, _y: f32, _pressure: f32) {}
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

impl FramePacingGuest for Component {
    fn next_frame_delay() -> u32 {
        // Idle ~1 Hz; the bar only changes on an alarm (every 5 s) anyway.
        1000
    }
}

impl AlarmGuest for Component {
    fn on_alarm(id: u64) {
        STATE.with(|s| s.borrow_mut().count += 1);
        let _ = id;
    }
}

impl BackgroundGuest for Component {
    fn bg_tick() -> u32 {
        // Real background work would pump a socket here; the test just bumps a
        // counter (the host logs each bg-tick, so logcat proves the pump fires
        // while backgrounded). Ask for ~1 Hz so the log is easy to read.
        STATE.with(|s| s.borrow_mut().bg_count += 1);
        1000
    }
}

export!(Component);
