//! wandr.alarm.test — minimal test guest for the wandr:alarm timed-wake primitive
//! (Arbiter Inc. 3c). Imports `wandr:alarm/scheduler`, exports
//! `wandr:alarm/alarm-handler`. On its first frame it schedules a repeating alarm
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
use crate::exports::wandr::alarm::alarm_handler::Guest as AlarmGuest;
use crate::exports::wandr::audio_focus::focus_handler::{FocusChange, Guest as FocusHandlerGuest};
use crate::exports::wandr::background::background::Guest as BackgroundGuest;
use crate::exports::wandr::notify::notify_handler::Guest as NotifyHandlerGuest;
use crate::wandr::audio_focus::focus::{self, FocusKind};
use crate::wandr::notify::notifier;
use crate::my::skiko_gfx::canvas::{
    self, BlendMode, ColorFilterKind, PaintAttrs, PaintStyle, StrokeCap, StrokeJoin,
};
use crate::wandr::alarm::scheduler;

const ALARM_ID: u64 = 1;
const PERIOD_MS: u64 = 5000;

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    count: u32,
    scheduled: bool,
    bg_count: u32,
    notified: bool,
    clicked: u32,
    // wandr-arbiter-audio M2 — focus state. `focus_requested` guards the one-shot
    // request; `focus_change` is the last on-focus-changed code (0..3) the
    // arbiter pushed, colouring the bar as visible proof.
    focus_requested: bool,
    focus_change: u32,
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
            // M3 — raise one notification on first frame (host logs the post;
            // tapping it delivers on-notification-click below).
            if !s.notified {
                notifier::post(1, "Alarm Test", "tap me");
                s.notified = true;
            }
            // wandr-arbiter-audio M2 — request permanent audio focus once.
            if !s.focus_requested {
                let r = focus::request(FocusKind::Gain);
                s.focus_requested = true;
                let _ = r; // host logs the granted/failed result
            }
            let (w, h) = (s.w.max(1.0), s.h.max(1.0));
            canvas::begin_frame();
            canvas::clear(0xFF12121C);
            // Growing bar: width tracks the alarm count (wraps to stay on-screen).
            // Colour tracks the audio-focus state — green=owner/gain, amber=duck,
            // grey=loss/loss-transient — visible proof of on-focus-changed.
            let bar_color = match s.focus_change {
                0 | 1 => 0xFF707070, // loss / loss-transient
                2     => 0xFFD0A030, // duck
                _     => 0xFF40C040, // gain (owner)
            };
            let bar_w = ((s.count as f32) * 24.0) % (w - 40.0).max(24.0);
            canvas::draw_rect(20.0, h * 0.4, bar_w + 24.0, 60.0, paint(bar_color));
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

impl NotifyHandlerGuest for Component {
    fn on_notification_click(id: u64) {
        STATE.with(|s| s.borrow_mut().clicked += 1);
        let _ = id;
    }
}

impl FocusHandlerGuest for Component {
    fn on_focus_changed(change: FocusChange) {
        // Record the change so render_frame recolours the bar (visible proof).
        let code = match change {
            FocusChange::Loss          => 0,
            FocusChange::LossTransient => 1,
            FocusChange::Duck          => 2,
            FocusChange::Gain          => 3,
        };
        STATE.with(|s| s.borrow_mut().focus_change = code);
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
