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
    path: "wit",
    generate_all,
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::frame_pacing::Guest as FramePacingGuest;
use crate::exports::my::skiko_gfx::renderer::{Guest, KeyKind, PointerKind};
use crate::my::skiko_gfx::canvas::{
    self, BlendMode, ColorFilterKind, PaintAttrs, PaintStyle, StrokeCap, StrokeJoin,
};
use crate::my::skiko_gfx::status;
use crate::war::notify::notify_feed;

const BG: u32 = 0xFF12121C; // opaque dark strip
const FG: u32 = 0xFFECECEC;
// Task 64 — ms between status-bar frames. With on-demand rendering the
// host gates render-frame, so frame-count no longer maps to wall-clock;
// we ask for ~1 Hz and refresh the clock/battery on EVERY render instead
// of the old `frame % 60` gate (which would now fire once a minute).
const REFRESH_MS: u32 = 1000;

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    clock: String,
    battery: String,
    clock_blob: Option<u32>,
    batt_blob: Option<u32>,
    label_blob: Option<u32>,
    // M3b — notification badge.
    notif_label: String,
    notif_blob: Option<u32>,
    notif_count: u32,
    notif_nid: u64,
    badge_x0: f32,
    badge_x1: f32,
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

/// M3b — a tap inside the notification badge opens the most recent notification.
fn tap(kind: PointerKind, x: f32) {
    if !matches!(kind, PointerKind::Up) {
        return; // act on release, like a button
    }
    STATE.with(|st| {
        let s = st.borrow();
        if s.notif_count > 0 && x >= s.badge_x0 && x <= s.badge_x1 {
            notify_feed::click(s.notif_nid);
        }
    });
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
            // Refresh clock + battery every render (the host paces us at
            // ~1 Hz via frame-pacing); rebuild blobs only when the text
            // actually changes, so a no-change render stays a pure replay.
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
            // M3b — active notifications (queried from the arbiter via the host
            // each ~1 Hz render). Show a "● N" badge; tapping it opens the most
            // recent one (the arbiter foregrounds the owner + delivers the click).
            let actives = notify_feed::list_active();
            s.notif_count = actives.len() as u32;
            s.notif_nid = actives.last().map(|n| n.nid).unwrap_or(0);
            let label = if s.notif_count > 0 { format!("\u{25CF} {}", s.notif_count) } else { String::new() };
            if label != s.notif_label || (s.notif_count > 0 && s.notif_blob.is_none()) {
                if let Some(b) = s.notif_blob.take() { canvas::drop_text_blob(b); }
                s.notif_blob = if s.notif_count > 0 { Some(blob(&label, s.h * 0.40, 700)) } else { None };
                s.notif_label = label;
            }

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
            // Notification badge, left of the battery; remember its hit region.
            let badge_x = w - 300.0;
            if let Some(b) = s.notif_blob {
                canvas::draw_text_blob(b, badge_x, baseline, paint(0xFFE5894A));
                s.badge_x0 = badge_x - 24.0;
                s.badge_x1 = badge_x + 96.0;
            } else {
                s.badge_x0 = 0.0;
                s.badge_x1 = 0.0;
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

    fn on_pointer_event(kind: PointerKind, x: f32, _y: f32) {
        tap(kind, x);
    }
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_pointer_event_v2(_pid: u32, kind: PointerKind, x: f32, _y: f32, _pressure: f32) {
        tap(kind, x);
    }
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

impl FramePacingGuest for Bar {
    fn next_frame_delay() -> u32 {
        // ~1 Hz so the clock stays current; the host clamps to its IDLE_CAP.
        REFRESH_MS
    }
}

export!(Bar);
