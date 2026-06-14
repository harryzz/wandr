//! wandr.keyguard — the wandr lockscreen as a LIGHT Rust canvas guest (like
//! wandr.statusbar). A full-screen dark surface with a big centered clock and a
//! "swipe up to unlock" hint. Launched as a high-layer overlay
//! (`--standalone-overlay-lock`); the arbiter keyguard module shows it
//! (Role::Lockscreen) while locked. The unlock gesture (swipe → arbiter) is
//! wired in M3; for now it just renders.

wit_bindgen::generate!({
    world: "wandr:keyguard-app/keyguard-app",
    path: "wit",
    generate_all,
});

use std::cell::RefCell;

use crate::exports::wandr::ui_shell::frame_pacing::Guest as FramePacingGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::pointer_handler::{
    Guest as PointerGuest, Kind as PointerKind, PointerEvent,
};
use crate::wandr::chrome::now_playing as nowp;
use crate::wandr::chrome::status;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::types as wtypes;
use crate::wandr::keyguard::keyguard;

const BG: u32 = 0xFF0A0A12; // near-black lock background
const CLOCK_FG: u32 = 0xFFFFFFFF;
const HINT_FG: u32 = 0xFF9AA0B4;
const FAMILY: &[u8] = b"sans-serif";
const HINT: &str = "swipe up to unlock";
// Now-playing transport (M2).
const ACCENT: u32 = 0xFF4285F4;
const ICON: u32 = 0xFFFFFFFF;
const BAR_BG: u32 = 0xFF2A2A3C;

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    clock: String,
    clock_para: Option<Para>,
    hint_para: Option<Para>,
    // M3 — swipe-up unlock: remember the press y; a clear upward release unlocks.
    down_y: Option<f32>,
    // Highest point (lowest y) reached during the current drag — peak upward
    // travel, tracked via Move so a swipe is detected even when the Up event
    // reports an unreliable release coordinate.
    min_y: f32,
    // M2 — now-playing transport (cached from the last frame, for hit-testing).
    np_present: bool,
    np_playing: bool,
    np_dur: f64,
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
        filter: None,
    }
}

/// Laid-out paragraph (wasi:canvas/layout) — color baked, draws at a
/// baseline origin, REAL width (retires the 0.27-per-glyph centering hack).
struct Para {
    p: wlayout::Paragraph,
    baseline: f32,
    width: f32,
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
        baseline_shift: 0.0,
        decoration: None,
        shadows: Vec::new(),
        background: None,
    };
    // 0.0.2 setter-form builder; align defaults to start.
    let b = wlayout::ParagraphBuilder::new(&style);
    b.add_text(text);
    let p = wlayout::ParagraphBuilder::build(b);
    p.layout(1.0e6);
    let baseline = p.alphabetic_baseline();
    let width = p.max_intrinsic_width();
    Para { p, baseline, width }
}

fn draw_para(cv: &crate::wasi::canvas::draw::Canvas, pa: &Para, x: f32, baseline_y: f32) {
    pa.p.paint(cv, wtypes::Point { x, y: baseline_y - pa.baseline });
}

// ── Now-playing transport (M2) ──────────────────────────────────────────────
fn rect(x: f32, y: f32, w: f32, h: f32) -> wtypes::Rect {
    wtypes::Rect { x, y, width: w, height: h }
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> wtypes::RoundedRect {
    let c = wtypes::Point { x: r, y: r };
    wtypes::RoundedRect {
        rect: rect(x, y, w, h),
        top_left: c,
        top_right: c,
        bottom_right: c,
        bottom_left: c,
    }
}

/// Transport geometry — all derived from the surface dims (no hardcoding).
struct NpLayout {
    title_baseline: f32,
    artist_baseline: f32,
    bar: wtypes::Rect,
    prev_c: (f32, f32),
    play_c: (f32, f32),
    next_c: (f32, f32),
    btn_r: f32,
}

fn np_layout(w: f32, h: f32) -> NpLayout {
    let m = w * 0.10;
    let btn_r = w.min(h) * 0.058;
    let row_y = h * 0.77;
    let gap = btn_r * 2.9;
    NpLayout {
        title_baseline: h * 0.60,
        artist_baseline: h * 0.645,
        bar: rect(m, h * 0.685, w - 2.0 * m, h * 0.006),
        prev_c: (w * 0.5 - gap, row_y),
        play_c: (w * 0.5, row_y),
        next_c: (w * 0.5 + gap, row_y),
        btn_r,
    }
}

fn draw_play_pause(cv: &crate::wasi::canvas::draw::Canvas, c: (f32, f32), r: f32, playing: bool) {
    let (cx, cy) = c;
    cv.draw_oval(rect(cx - r, cy - r, r * 2.0, r * 2.0), &paint(ACCENT));
    if playing {
        let bw = r * 0.24;
        let bh = r * 0.84;
        cv.draw_rect(rect(cx - r * 0.38, cy - bh * 0.5, bw, bh), &paint(ICON));
        cv.draw_rect(rect(cx + r * 0.14, cy - bh * 0.5, bw, bh), &paint(ICON));
    } else {
        let x0 = cx - r * 0.30;
        let x1 = cx + r * 0.44;
        let y0 = cy - r * 0.46;
        let y1 = cy + r * 0.46;
        let path = format!("M {x0} {y0} L {x1} {cy} L {x0} {y1} Z");
        cv.draw_path(&path, wtypes::FillRule::Nonzero, &paint(ICON));
    }
}

fn draw_prev(cv: &crate::wasi::canvas::draw::Canvas, c: (f32, f32), r: f32) {
    let (cx, cy) = c;
    let s = r * 0.7;
    let path = format!(
        "M {} {} L {} {} L {} {} Z",
        cx + s * 0.5, cy - s * 0.6, cx - s * 0.4, cy, cx + s * 0.5, cy + s * 0.6,
    );
    cv.draw_path(&path, wtypes::FillRule::Nonzero, &paint(ICON));
    cv.draw_rect(rect(cx - s * 0.6, cy - s * 0.6, s * 0.18, s * 1.2), &paint(ICON));
}

fn draw_next(cv: &crate::wasi::canvas::draw::Canvas, c: (f32, f32), r: f32) {
    let (cx, cy) = c;
    let s = r * 0.7;
    let path = format!(
        "M {} {} L {} {} L {} {} Z",
        cx - s * 0.5, cy - s * 0.6, cx + s * 0.4, cy, cx - s * 0.5, cy + s * 0.6,
    );
    cv.draw_path(&path, wtypes::FillRule::Nonzero, &paint(ICON));
    cv.draw_rect(rect(cx + s * 0.42, cy - s * 0.6, s * 0.18, s * 1.2), &paint(ICON));
}

/// M3 — unlock on a clear upward swipe (Down then Up at least ~12% of the screen
/// higher). A tap (little movement) does NOT unlock, matching the "swipe up" hint.
fn gesture(kind: PointerKind, x: f32, y: f32) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        match kind {
            PointerKind::Down => {
                s.down_y = Some(y);
                s.min_y = y;
            }
            PointerKind::Move => {
                if s.down_y.is_some() && y < s.min_y {
                    s.min_y = y;
                }
            }
            PointerKind::Up => {
                let Some(dy) = s.down_y.take() else { return };
                // Peak upward travel during the drag OR the release delta — the
                // larger wins, so a swipe unlocks even if Up reports a snapped-back
                // coordinate. ~10% of screen height.
                let travel = (dy - y).max(dy - s.min_y);
                if travel >= s.h * 0.10 {
                    keyguard::unlock();
                    return;
                }
                // A tap (little movement) on the now-playing transport → send the
                // intent to the active session (the arbiter routes on-action).
                if s.np_present && travel < s.h * 0.04 {
                    let nl = np_layout(s.w, s.h);
                    let hit = |c: (f32, f32)| {
                        let (dx, dyy) = (x - c.0, y - c.1);
                        dx * dx + dyy * dyy <= nl.btn_r * nl.btn_r
                    };
                    if hit(nl.play_c) {
                        let act = if s.np_playing { nowp::Action::Pause } else { nowp::Action::Play };
                        nowp::send_action(act, None);
                    } else if hit(nl.prev_c) {
                        nowp::send_action(nowp::Action::PreviousTrack, None);
                    } else if hit(nl.next_c) {
                        nowp::send_action(nowp::Action::NextTrack, None);
                    } else if x >= nl.bar.x
                        && x <= nl.bar.x + nl.bar.width
                        && (y - nl.bar.y).abs() <= s.h * 0.04
                    {
                        let frac = ((x - nl.bar.x) / nl.bar.width).clamp(0.0, 1.0) as f64;
                        nowp::send_action(nowp::Action::SeekTo, Some(frac * s.np_dur));
                    }
                }
            }
            _ => {}
        }
    });
}

struct Lock;

impl FrameGuest for Lock {
    fn on_frame(_nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            let cv = wctx(|x| x.get_current_buffer());
            if s.w <= 0.0 {
                s.w = cv.width();
                s.h = cv.height();
            }
            let (w, h) = (s.w.max(1.0), s.h.max(1.0));
            // Big clock — rebuilt only when the minute changes.
            let clock = status::clock_text();
            if clock != s.clock || s.clock_para.is_none() {
                s.clock_para = Some(para(&clock, h * 0.13, 300, CLOCK_FG));
                s.clock = clock;
            }
            if s.hint_para.is_none() {
                s.hint_para = Some(para(HINT, h * 0.030, 500, HINT_FG));
            }

            cv.clear(BG);
            cv.draw_rect(wtypes::Rect { x: 0.0, y: 0.0, width: w, height: h }, &paint(BG));
            // Real centering — the paragraph carries its measured width.
            if let Some(p) = &s.clock_para {
                draw_para(&cv, p, w * 0.5 - p.width * 0.5, h * 0.42);
            }
            if let Some(p) = &s.hint_para {
                draw_para(&cv, p, w * 0.5 - p.width * 0.5, h * 0.88);
            }

            // ── Now-playing transport (M2) ──
            match nowp::get() {
                Some(np) => {
                    s.np_present = true;
                    s.np_playing = matches!(np.state, nowp::PlaybackState::Playing);
                    s.np_dur = np.duration_s;
                    let nl = np_layout(w, h);
                    let tp = para(&np.title, h * 0.034, 600, CLOCK_FG);
                    draw_para(&cv, &tp, w * 0.5 - tp.width * 0.5, nl.title_baseline);
                    if !np.artist.is_empty() {
                        let ap = para(&np.artist, h * 0.026, 400, HINT_FG);
                        draw_para(&cv, &ap, w * 0.5 - ap.width * 0.5, nl.artist_baseline);
                    }
                    let b = nl.bar;
                    cv.draw_rounded_rect(rrect(b.x, b.y, b.width, b.height, b.height * 0.5), &paint(BAR_BG));
                    let frac = if np.duration_s > 0.0 {
                        (np.position_s / np.duration_s).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    if frac > 0.0 {
                        cv.draw_rounded_rect(
                            rrect(b.x, b.y, b.width * frac, b.height, b.height * 0.5),
                            &paint(ACCENT),
                        );
                    }
                    draw_prev(&cv, nl.prev_c, nl.btn_r);
                    draw_play_pause(&cv, nl.play_c, nl.btn_r, s.np_playing);
                    draw_next(&cv, nl.next_c, nl.btn_r);
                }
                None => s.np_present = false,
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

}

impl PointerGuest for Lock {
    fn on_pointer(ev: PointerEvent) {
        gesture(ev.kind, ev.x, ev.y);
    }
}

impl FramePacingGuest for Lock {
    fn next_frame_delay() -> u32 {
        // ~1 Hz keeps the clock current; while a media session is showing, tick
        // faster so the scrubber + play/pause state stay live.
        STATE.with(|st| if st.borrow().np_present { 300 } else { 1000 })
    }
}

export!(Lock);
