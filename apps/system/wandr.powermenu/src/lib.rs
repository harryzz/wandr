//! wandr.powermenu — the power long-press menu (task 110). A LIGHT Rust canvas
//! guest (like wandr.keyguard): a dim full-screen overlay with four buttons —
//! Lock / Power off / Restart / Emergency. The arbiter shows it (Role::Lockscreen)
//! on a POWER long-press and demotes the app behind it; a button tap calls a
//! `wandr:keyguard/keyguard` pm-* verb (host → arbiter), which hides the menu and
//! performs the action. A tap outside the buttons dismisses.

wit_bindgen::generate!({
    world: "wandr:powermenu-app/powermenu-app",
    path: "wit",
    generate_all,
});

use std::cell::RefCell;

use crate::exports::wandr::ui_shell::frame_pacing::Guest as FramePacingGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::pointer_handler::{
    Guest as PointerGuest, Kind as PointerKind, PointerEvent,
};
use crate::wandr::keyguard::keyguard as control;
use crate::wasi::canvas::draw::Canvas;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::types as wtypes;

const SCRIM: u32 = 0xCC05060B; // dim over the app behind
const BTN_BG: u32 = 0xFF1E2230;
const BTN_FG: u32 = 0xFFFFFFFF;
const EMERGENCY_BG: u32 = 0xFF7A1E22; // red-ish
const TITLE_FG: u32 = 0xFFB8BCC8;

/// The four actions, top to bottom.
const ITEMS: [&str; 4] = ["Lock", "Power off", "Restart", "Emergency"];

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
}
thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}
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
fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> wtypes::RoundedRect {
    let c = wtypes::Point { x: r, y: r };
    wtypes::RoundedRect { rect: rect(x, y, w, h), top_left: c, top_right: c, bottom_right: c, bottom_left: c }
}
fn para(text: &str, size: f32, weight: u32, color: u32) -> (wlayout::Paragraph, f32, f32) {
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
    let b = wlayout::ParagraphBuilder::new(&style);
    b.add_text(text);
    let p = wlayout::ParagraphBuilder::build(b);
    p.layout(1.0e6);
    let baseline = p.alphabetic_baseline();
    let width = p.max_intrinsic_width();
    (p, baseline, width)
}
fn draw_centered(cv: &Canvas, text: &str, size: f32, weight: u32, color: u32, cx: f32, baseline_y: f32) {
    let (p, baseline, width) = para(text, size, weight, color);
    p.paint(cv, wtypes::Point { x: cx - width * 0.5, y: baseline_y - baseline });
}

/// Button `i`'s rect — a centered vertical stack, derived from the surface dims.
fn item_rect(w: f32, h: f32, i: usize) -> wtypes::Rect {
    let bw = (w * 0.66).min(h * 0.5);
    let bh = h * 0.09;
    let gap = h * 0.025;
    let n = ITEMS.len() as f32;
    let total = n * bh + (n - 1.0) * gap;
    let x = (w - bw) * 0.5;
    let y0 = (h - total) * 0.5;
    rect(x, y0 + i as f32 * (bh + gap), bw, bh)
}

struct PowerMenu;

impl FrameGuest for PowerMenu {
    fn on_frame(_nanos: u64) {
        STATE.with(|st| {
            let s = st.borrow();
            let cv = wctx(|x| x.get_current_buffer());
            let (w, h) = (s.w.max(cv.width()).max(1.0), s.h.max(cv.height()).max(1.0));
            cv.clear(0);
            cv.draw_rect(rect(0.0, 0.0, w, h), &paint(SCRIM));
            draw_centered(&cv, "Power", h * 0.03, 600, TITLE_FG, w * 0.5, item_rect(w, h, 0).y - h * 0.03);
            for (i, label) in ITEMS.iter().enumerate() {
                let r = item_rect(w, h, i);
                let bg = if *label == "Emergency" { EMERGENCY_BG } else { BTN_BG };
                cv.draw_rounded_rect(rrect(r.x, r.y, r.width, r.height, r.height * 0.22), &paint(bg));
                draw_centered(&cv, label, h * 0.028, 600, BTN_FG, r.x + r.width * 0.5, r.y + r.height * 0.62);
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

impl PointerGuest for PowerMenu {
    fn on_pointer(ev: PointerEvent) {
        if !matches!(ev.kind, PointerKind::Down) {
            return;
        }
        let (w, h) = STATE.with(|st| {
            let s = st.borrow();
            (s.w.max(1.0), s.h.max(1.0))
        });
        for i in 0..ITEMS.len() {
            let r = item_rect(w, h, i);
            if ev.x >= r.x && ev.x <= r.x + r.width && ev.y >= r.y && ev.y <= r.y + r.height {
                match i {
                    0 => control::pm_lock(),
                    1 => control::pm_power_off(),
                    2 => control::pm_restart(),
                    _ => control::pm_emergency(),
                }
                return;
            }
        }
        // Tapped outside the buttons → dismiss.
        control::pm_dismiss();
    }
}

impl FramePacingGuest for PowerMenu {
    fn next_frame_delay() -> u32 {
        500 // static menu; only repaints on resize/show
    }
}

export!(PowerMenu);
