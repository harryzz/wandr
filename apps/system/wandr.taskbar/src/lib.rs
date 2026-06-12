//! wandr.taskbar — the wandr navigation bar as a LIGHT Rust canvas guest
//! (task 56). Runs on a thin always-visible bottom-strip overlay; draws
//! three Android-style buttons — Back (left triangle), Home (circle),
//! Recents (square) — via the canvas WIT and forwards taps to the arbiter
//! through the `my:skiko-gfx/launcher` nav verbs (go-back / go-home /
//! recents).
//!
//! The icons are drawn as shapes (draw-path / draw-oval / draw-rect), not
//! font glyphs: the device's sans-serif lacks the geometric-shapes Unicode
//! block (◁ ○ □), so glyphs render as tofu. Shapes are always crisp.
//!
//! No Kotlin/Compose → leak-immune + tiny, which matters for an always-on
//! chrome process. The layout is trivial (three equal thirds) so it's
//! recomputed inline each frame; a tapped button flashes briefly.

wit_bindgen::generate!({
    world: "wandr:taskbar-app/taskbar-app",
    path: "wit",
    generate_all,
});

use std::cell::RefCell;

use crate::exports::wandr::ui_shell::frame_pacing::Guest as FramePacingGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::pointer_handler::{
    Guest as PointerGuest, Kind as PointerKind, PointerEvent,
};
use crate::wandr::chrome::launcher;
use crate::wasi::canvas::draw as wdraw;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::types as wtypes;

const BG: u32 = 0xFF12121C; // opaque dark strip (matches the status bar)
const FG: u32 = 0xFFECECEC; // icon color
const FLASH: u32 = 0x40FFFFFF; // press-highlight pill (translucent white)
const FLASH_FRAMES: u64 = 8; // how long a tapped button stays lit

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    frame: u64,
    /// (button index, frame the press happened) — drives the flash pill.
    pressed: Option<(usize, u64)>,
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

fn rect(x: f32, y: f32, w: f32, h: f32) -> wtypes::Rect {
    wtypes::Rect { x, y, width: w, height: h }
}

/// Fire the nav action for a tapped button index (0=back, 1=home, 2=recents).
fn invoke(index: usize) {
    match index {
        0 => launcher::go_back(),
        1 => launcher::go_home(),
        2 => launcher::recents(),
        _ => {}
    }
}

/// Draw the icon for button `i` centered at (cx, cy) with extent `r`.
fn draw_icon(cv: &wdraw::Canvas, i: usize, cx: f32, cy: f32, r: f32) {
    let p = paint(FG);
    match i {
        // Back — left-pointing triangle.
        0 => {
            let svg = format!(
                "M {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1} Z",
                cx - r, cy, cx + r * 0.8, cy - r, cx + r * 0.8, cy + r,
            );
            cv.draw_path(&svg, wtypes::FillRule::Nonzero, &p);
        }
        // Home — circle.
        1 => cv.draw_oval(rect(cx - r, cy - r, 2.0 * r, 2.0 * r), &p),
        // Recents — square.
        2 => {
            let s = r * 1.7;
            cv.draw_rect(rect(cx - s * 0.5, cy - s * 0.5, s, s), &p);
        }
        _ => {}
    }
}

struct Taskbar;

impl FrameGuest for Taskbar {
    fn on_frame(_nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            let cv = wctx(|x| x.get_current_buffer());
            if s.w == 0.0 {
                s.w = cv.width();
                s.h = cv.height();
            }
            let w = s.w;
            let h = s.h;
            let third = w / 3.0;
            let cy = h * 0.5;
            let r = h * 0.22; // icon extent — ~0.44h tall, fits the strip
            let frame = s.frame;

            cv.clear(BG);
            cv.draw_rect(rect(0.0, 0.0, w, h), &paint(BG));

            // Flash pill behind a recently-tapped button.
            if let Some((idx, set_at)) = s.pressed {
                if frame.wrapping_sub(set_at) < FLASH_FRAMES {
                    let cx = third * (idx as f32 + 0.5);
                    let pw = third * 0.7;
                    let ph = h * 0.7;
                    cv.draw_rect(rect(cx - pw * 0.5, (h - ph) * 0.5, pw, ph), &paint(FLASH));
                } else {
                    s.pressed = None;
                }
            }

            for i in 0..3 {
                draw_icon(&cv, i, third * (i as f32 + 0.5), cy, r);
            }
            drop(cv);
            wctx(|x| x.present());

            s.frame = frame.wrapping_add(1);
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

impl PointerGuest for Taskbar {
    fn on_pointer(ev: PointerEvent) {
        if !matches!(ev.kind, PointerKind::Down) {
            return;
        }
        let x = ev.x;
        let index = STATE.with(|st| {
            let mut s = st.borrow_mut();
            if s.w <= 0.0 {
                return None;
            }
            let idx = ((x / (s.w / 3.0)) as usize).min(2);
            s.pressed = Some((idx, s.frame));
            Some(idx)
        });
        if let Some(idx) = index {
            invoke(idx);
        }
    }
}

/// Task 64 — the nav bar is static except for the brief tap-flash pill.
/// Idle otherwise; the host wakes us on input. The flash is frame-counted
/// (FLASH_FRAMES) so while a press is live we ask for the next frame
/// immediately (0) — `end-frame`'s vsync-blocking swap paces it to ~60 fps.
const IDLE: u32 = 60_000;

impl FramePacingGuest for Taskbar {
    fn next_frame_delay() -> u32 {
        STATE.with(|st| if st.borrow().pressed.is_some() { 0 } else { IDLE })
    }
}

export!(Taskbar);
