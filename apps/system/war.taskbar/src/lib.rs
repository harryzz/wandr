//! war.taskbar — the wart navigation bar as a LIGHT Rust canvas guest
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
    world: "taskbar-app",
    path: "wit/taskbar.wit",
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::renderer::{Guest, KeyKind, PointerKind};
use crate::my::skiko_gfx::canvas::{
    self, BlendMode, ColorFilterKind, PaintAttrs, PaintStyle, StrokeCap, StrokeJoin,
};
use crate::my::skiko_gfx::launcher;

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
fn draw_icon(i: usize, cx: f32, cy: f32, r: f32) {
    let p = paint(FG);
    match i {
        // Back — left-pointing triangle.
        0 => {
            let svg = format!(
                "M {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1} Z",
                cx - r, cy, cx + r * 0.8, cy - r, cx + r * 0.8, cy + r,
            );
            canvas::draw_path(svg.as_bytes(), p);
        }
        // Home — circle.
        1 => canvas::draw_oval(cx - r, cy - r, 2.0 * r, 2.0 * r, p),
        // Recents — square.
        2 => {
            let s = r * 1.7;
            canvas::draw_rect(cx - s * 0.5, cy - s * 0.5, s, s, p);
        }
        _ => {}
    }
}

struct Taskbar;

impl Guest for Taskbar {
    fn render_frame(_nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            if s.w == 0.0 {
                s.w = canvas::surface_width() as f32;
                s.h = canvas::surface_height() as f32;
            }
            let w = s.w;
            let h = s.h;
            let third = w / 3.0;
            let cy = h * 0.5;
            let r = h * 0.22; // icon extent — ~0.44h tall, fits the strip
            let frame = s.frame;

            canvas::begin_frame();
            canvas::clear(BG);
            canvas::draw_rect(0.0, 0.0, w, h, paint(BG));

            // Flash pill behind a recently-tapped button.
            if let Some((idx, set_at)) = s.pressed {
                if frame.wrapping_sub(set_at) < FLASH_FRAMES {
                    let cx = third * (idx as f32 + 0.5);
                    let pw = third * 0.7;
                    let ph = h * 0.7;
                    canvas::draw_rect(cx - pw * 0.5, (h - ph) * 0.5, pw, ph, paint(FLASH));
                } else {
                    s.pressed = None;
                }
            }

            for i in 0..3 {
                draw_icon(i, third * (i as f32 + 0.5), cy, r);
            }
            canvas::end_frame();

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

    fn on_pointer_event_v2(_pid: u32, kind: PointerKind, x: f32, _y: f32, _pressure: f32) {
        if !matches!(kind, PointerKind::Down) {
            return;
        }
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

    // Unused inputs.
    fn on_pointer_event(_kind: PointerKind, _x: f32, _y: f32) {}
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

export!(Taskbar);
