//! wandr.statusbar — the wandr system status bar as a LIGHT Rust canvas
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
    world: "my:skiko-gfx/statusbar-app",
    path: ["../../../proposals/wasi-canvas/wit", "wit"],
    generate_all,
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::frame_pacing::Guest as FramePacingGuest;
use crate::exports::my::skiko_gfx::renderer::{Guest, KeyKind, PointerKind};
use crate::my::skiko_gfx::status;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::types as wtypes;
use crate::wandr::notify::notify_feed;

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
    clock_para: Option<Para>,
    batt_para: Option<Para>,
    label_para: Option<Para>,
    // M3b — notification badge.
    notif_label: String,
    notif_para: Option<Para>,
    notif_count: u32,
    notif_nid: u64,
    badge_x0: f32,
    badge_x1: f32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

// wasi:canvas canvas-context (wasi-gfx graphics-context idiom): one per
// surface, lazily acquired; frames bracket via get-current-buffer/present.
thread_local! {
    static WCTX: RefCell<Option<wembed::CanvasContext>> = const { RefCell::new(None) };
}
fn wctx<R>(f: impl FnOnce(&wembed::CanvasContext) -> R) -> R {
    WCTX.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(wembed::get_context());
        }
        f(c.borrow().as_ref().unwrap())
    })
}

fn paint(color: u32) -> wtypes::Paint<'static> {
    wtypes::Paint {
        style: wtypes::PaintStyle::Fill,
        color,
        alpha: 255,
        blend: wtypes::BlendMode::SrcOver,
        anti_alias: true,
        shader: None,
        stroke_width: 0.0,
        stroke_cap: wtypes::StrokeCap::Butt,
        stroke_join: wtypes::StrokeJoin::Miter,
        stroke_miter: 4.0,
        blur: None,
    }
}

/// A laid-out host paragraph (wasi:canvas/layout) standing in for the old
/// text blobs. Color bakes at build; draws at a BASELINE origin like
/// blobs did (paragraphs paint at top-left: top = baseline_y - baseline).
struct Para {
    p: wlayout::Paragraph,
    baseline: f32,
}

fn para(text: &str, size: f32, weight: u32, color: u32) -> Para {
    let style = wlayout::TextStyle {
        family: "sans-serif".into(),
        size,
        weight,
        italic: false,
        color,
        letter_spacing: 0.0,
        line_height: 0.0,
    };
    let b = wlayout::ParagraphBuilder::new(&style, wlayout::Align::Start);
    b.add_text(text);
    let p = wlayout::ParagraphBuilder::build(b);
    p.layout(1.0e6);
    let baseline = p.alphabetic_baseline();
    Para { p, baseline }
}

fn draw_para(cv: &crate::wasi::canvas::draw::Canvas, pa: &Para, x: f32, baseline_y: f32) {
    pa.p.paint(cv, wtypes::Point { x, y: baseline_y - pa.baseline });
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
            let cv = wctx(|x| x.get_current_buffer());
            if s.w == 0.0 {
                s.w = cv.width();
                s.h = cv.height();
            }
            // One-time static label. Font sizes are proportional to the
            // bar height (WANDR_STATUSBAR_PX) so they scale with it.
            if s.label_para.is_none() {
                s.label_para = Some(para("wandr", s.h * 0.32, 600, 0xFF8AB4F8));
            }
            // Refresh clock + battery every render (the host paces us at
            // ~1 Hz via frame-pacing); rebuild blobs only when the text
            // actually changes, so a no-change render stays a pure replay.
            let clock = status::clock_text();
            if clock != s.clock || s.clock_para.is_none() {
                s.clock_para = Some(para(&clock, s.h * 0.42, 600, FG));
                s.clock = clock;
            }
            let battery = status::battery_text();
            if battery != s.battery || s.batt_para.is_none() {
                s.batt_para = Some(para(&battery, s.h * 0.32, 400, FG));
                s.battery = battery;
            }
            // M3b — active notifications (queried from the arbiter via the host
            // each ~1 Hz render). Show a "● N" badge; tapping it opens the most
            // recent one (the arbiter foregrounds the owner + delivers the click).
            let actives = notify_feed::list_active();
            s.notif_count = actives.len() as u32;
            s.notif_nid = actives.last().map(|n| n.nid).unwrap_or(0);
            let label = if s.notif_count > 0 { format!("\u{25CF} {}", s.notif_count) } else { String::new() };
            if label != s.notif_label || (s.notif_count > 0 && s.notif_para.is_none()) {
                s.notif_para = if s.notif_count > 0 { Some(para(&label, s.h * 0.40, 700, 0xFFE5894A)) } else { None };
                s.notif_label = label;
            }

            let w = s.w;
            let h = s.h;
            // ~vertically centered for the proportional font sizes.
            let baseline = h * 0.64;
            cv.clear(BG);
            cv.draw_rect(wtypes::Rect { x: 0.0, y: 0.0, width: w, height: h }, &paint(BG));
            if let Some(p) = &s.label_para { draw_para(&cv, p, 40.0, baseline); }
            // Clock centered.
            if let Some(p) = &s.clock_para { draw_para(&cv, p, w * 0.5 - 48.0, baseline); }
            // Battery right-aligned-ish.
            if let Some(p) = &s.batt_para { draw_para(&cv, p, w - 160.0, baseline); }
            // Notification badge, left of the battery; remember its hit region.
            let badge_x = w - 300.0;
            if s.notif_para.is_some() {
                if let Some(p) = &s.notif_para { draw_para(&cv, p, badge_x, baseline); }
                s.badge_x0 = badge_x - 24.0;
                s.badge_x1 = badge_x + 96.0;
            } else {
                s.badge_x0 = 0.0;
                s.badge_x1 = 0.0;
            }
            drop(cv);
            wctx(|x| x.present());
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
