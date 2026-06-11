//! wandr.keyguard — the wandr lockscreen as a LIGHT Rust canvas guest (like
//! wandr.statusbar). A full-screen dark surface with a big centered clock and a
//! "swipe up to unlock" hint. Launched as a high-layer overlay
//! (`--standalone-overlay-lock`); the arbiter keyguard module shows it
//! (Role::Lockscreen) while locked. The unlock gesture (swipe → arbiter) is
//! wired in M3; for now it just renders.

wit_bindgen::generate!({
    world: "my:skiko-gfx/keyguard-app",
    path: ["../../../proposals/wasi-canvas/wit", "wit"],
    generate_all,
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::frame_pacing::Guest as FramePacingGuest;
use crate::exports::my::skiko_gfx::renderer::{Guest as RendererGuest, KeyKind, PointerKind};
use crate::my::skiko_gfx::status;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::types as wtypes;
use crate::wandr::keyguard::keyguard;

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
    clock_para: Option<Para>,
    hint_para: Option<Para>,
    // M3 — swipe-up unlock: remember the press y; a clear upward release unlocks.
    down_y: Option<f32>,
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
    };
    let b = wlayout::ParagraphBuilder::new(&style, wlayout::Align::Start);
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

/// M3 — unlock on a clear upward swipe (Down then Up at least ~12% of the screen
/// higher). A tap (little movement) does NOT unlock, matching the "swipe up" hint.
fn gesture(kind: PointerKind, y: f32) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        match kind {
            PointerKind::Down => s.down_y = Some(y),
            PointerKind::Up => {
                if let Some(dy) = s.down_y.take() {
                    if dy - y >= s.h * 0.12 {
                        keyguard::unlock();
                    }
                }
            }
            _ => {}
        }
    });
}

struct Lock;

impl RendererGuest for Lock {
    fn render_frame(_nanos: u64) {
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

    fn on_pointer_event(kind: PointerKind, _x: f32, y: f32) {
        gesture(kind, y);
    }
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_pointer_event_v2(_pid: u32, kind: PointerKind, _x: f32, y: f32, _pressure: f32) {
        gesture(kind, y);
    }
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

impl FramePacingGuest for Lock {
    fn next_frame_delay() -> u32 {
        1000 // ~1 Hz so the clock stays current
    }
}

export!(Lock);
