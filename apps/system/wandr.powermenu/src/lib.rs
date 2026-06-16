//! wandr.powermenu — the power long-press menu (task 110). A LIGHT Rust canvas
//! guest (like wandr.keyguard): over the dimmed screen, a rounded dark card with
//! a 2×2 grid of circular icon buttons — Emergency / Lockdown / Power off /
//! Restart. The arbiter shows it (Role::Lockscreen) on a POWER long-press; a
//! button tap calls a `wandr:keyguard/keyguard` pm-* verb (host → arbiter),
//! which hides the menu and performs the action. A tap outside dismisses.

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

const SCRIM: u32 = 0xA6000000; // dim over the screen (wallpaper still visible)
const CARD: u32 = 0xF21D1D20; // rounded dark card
const CIRCLE: u32 = 0xFF2E2E31; // normal button circle
const CIRCLE_EMERGENCY: u32 = 0xFFF0655B; // Emergency = salmon/red
const ICON: u32 = 0xFFFFFFFF;
const LABEL: u32 = 0xFFE9ECF2;

// Row-major 2×2: Emergency, Lockdown, Power off, Restart.
const LABELS: [&str; 4] = ["Emergency", "Lockdown", "Power off", "Restart"];

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

fn fill(color: u32) -> wtypes::Paint<'static> {
    wtypes::Paint {
        style: wtypes::PaintStyle::Fill,
        color,
        alpha: 255,
        blend: wtypes::BlendMode::SrcOver,
        anti_alias: true,
        shader: None,
        stroke_width: 0.0,
        stroke_cap: wtypes::StrokeCap::Round,
        stroke_join: wtypes::StrokeJoin::Round,
        stroke_miter: 4.0,
        blur: None,
        filter: None,
    }
}
fn stroke(color: u32, width: f32) -> wtypes::Paint<'static> {
    let mut p = fill(color);
    p.style = wtypes::PaintStyle::Stroke;
    p.stroke_width = width;
    p
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> wtypes::Rect {
    wtypes::Rect { x, y, width: w, height: h }
}
fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> wtypes::RoundedRect {
    let c = wtypes::Point { x: r, y: r };
    wtypes::RoundedRect { rect: rect(x, y, w, h), top_left: c, top_right: c, bottom_right: c, bottom_left: c }
}
fn pt(x: f32, y: f32) -> wtypes::Point {
    wtypes::Point { x, y }
}
fn circle(cv: &Canvas, cx: f32, cy: f32, r: f32, color: u32) {
    cv.draw_oval(rect(cx - r, cy - r, r * 2.0, r * 2.0), &fill(color));
}
fn label_centered(cv: &Canvas, text: &str, size: f32, color: u32, cx: f32, baseline: f32) {
    let style = wlayout::TextStyle {
        family: "sans-serif".into(),
        size,
        weight: 500,
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
    let width = p.max_intrinsic_width();
    let base = p.alphabetic_baseline();
    p.paint(cv, pt(cx - width * 0.5, baseline - base));
}

/// The card rect (rounded dark container), centered horizontally, upper-middle.
fn card_rect(w: f32, h: f32) -> wtypes::Rect {
    let cw = (w * 0.80).min(h * 0.45);
    let ch = cw; // square 2×2
    rect((w - cw) * 0.5, (h - ch) * 0.42, cw, ch)
}
/// (quadrant rect, circle center, radius, label baseline) for button `i`.
fn item(w: f32, h: f32, i: usize) -> (wtypes::Rect, f32, f32, f32, f32) {
    let c = card_rect(w, h);
    let (qw, qh) = (c.width * 0.5, c.height * 0.5);
    let (col, row) = ((i % 2) as f32, (i / 2) as f32);
    let q = rect(c.x + col * qw, c.y + row * qh, qw, qh);
    let r = qw.min(qh) * 0.27;
    let ccx = q.x + qw * 0.5;
    let ccy = q.y + qh * 0.40;
    let label_base = q.y + qh * 0.80;
    (q, ccx, ccy, r, label_base)
}

// ── Icons (white, drawn from primitives) ──────────────────────────────────────
fn icon_emergency(cv: &Canvas, cx: f32, cy: f32, r: f32) {
    let p = stroke(ICON, r * 0.20);
    let l = r * 0.60;
    for deg in [90.0_f32, 30.0, 150.0] {
        let a = deg.to_radians();
        let (dx, dy) = (a.cos() * l, a.sin() * l);
        cv.draw_line(pt(cx - dx, cy - dy), pt(cx + dx, cy + dy), &p);
    }
}
fn icon_lock(cv: &Canvas, cx: f32, cy: f32, r: f32) {
    let bw = r * 1.05;
    let bh = r * 0.80;
    let by = cy - bh * 0.20;
    cv.draw_rounded_rect(rrect(cx - bw * 0.5, by, bw, bh, r * 0.18), &fill(ICON));
    // shackle — top half-circle, stroked, sitting on the body
    let sw = r * 0.66;
    cv.draw_arc(rect(cx - sw * 0.5, by - sw * 0.62, sw, sw), 180.0, 180.0, false, &stroke(ICON, r * 0.16));
}
fn icon_power(cv: &Canvas, cx: f32, cy: f32, r: f32) {
    let rr = r * 0.60;
    // ring with a gap at the top
    cv.draw_arc(rect(cx - rr, cy - rr, rr * 2.0, rr * 2.0), -55.0, 290.0, false, &stroke(ICON, r * 0.18));
    cv.draw_line(pt(cx, cy - rr * 1.15), pt(cx, cy - rr * 0.05), &stroke(ICON, r * 0.18));
}
fn icon_restart(cv: &Canvas, cx: f32, cy: f32, r: f32) {
    let rr = r * 0.60;
    cv.draw_arc(rect(cx - rr, cy - rr, rr * 2.0, rr * 2.0), -40.0, 300.0, false, &stroke(ICON, r * 0.18));
    // arrowhead at the arc's open (top) end, pointing up-left
    let ax = cx + rr * (-40.0_f32).to_radians().cos();
    let ay = cy + rr * (-40.0_f32).to_radians().sin();
    let s = r * 0.30;
    let path = format!(
        "M {} {} L {} {} L {} {} Z",
        ax + s * 0.1, ay - s, ax + s * 0.9, ay + s * 0.1, ax - s * 0.2, ay + s * 0.2
    );
    cv.draw_path(&path, wtypes::FillRule::Nonzero, &fill(ICON));
}

struct PowerMenu;

impl FrameGuest for PowerMenu {
    fn on_frame(_nanos: u64) {
        STATE.with(|st| {
            let s = st.borrow();
            let cv = wctx(|x| x.get_current_buffer());
            let (w, h) = (s.w.max(cv.width()).max(1.0), s.h.max(cv.height()).max(1.0));
            cv.clear(0);
            cv.draw_rect(rect(0.0, 0.0, w, h), &fill(SCRIM));
            let c = card_rect(w, h);
            cv.draw_rounded_rect(rrect(c.x, c.y, c.width, c.height, c.width * 0.07), &fill(CARD));
            for i in 0..4 {
                let (_q, ccx, ccy, r, lbase) = item(w, h, i);
                let col = if i == 0 { CIRCLE_EMERGENCY } else { CIRCLE };
                circle(&cv, ccx, ccy, r, col);
                match i {
                    0 => icon_emergency(&cv, ccx, ccy, r),
                    1 => icon_lock(&cv, ccx, ccy, r),
                    2 => icon_power(&cv, ccx, ccy, r),
                    _ => icon_restart(&cv, ccx, ccy, r),
                }
                label_centered(&cv, LABELS[i], h * 0.018, LABEL, ccx, lbase);
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
        for i in 0..4 {
            let (q, _cx, _cy, _r, _l) = item(w, h, i);
            if ev.x >= q.x && ev.x <= q.x + q.width && ev.y >= q.y && ev.y <= q.y + q.height {
                match i {
                    0 => control::pm_emergency(),
                    1 => control::pm_lock(),
                    2 => control::pm_power_off(),
                    _ => control::pm_restart(),
                }
                return;
            }
        }
        control::pm_dismiss(); // tapped outside the card
    }
}

impl FramePacingGuest for PowerMenu {
    fn next_frame_delay() -> u32 {
        500 // static menu; repaints on resize/show
    }
}

export!(PowerMenu);
