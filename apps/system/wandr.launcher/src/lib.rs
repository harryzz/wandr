//! wandr.launcher — the wandr home screen as a LIGHT Rust canvas guest
//! (task 57). Exports `my:skiko-gfx/renderer` and draws the app grid via the
//! wasi:canvas draft (proposals/wasi-canvas) — no Kotlin/Compose, so no
//! continuation leak ([[feedback_indeterminate_progress_leak]]) and a tiny
//! working set, which matters for an always-running home process.
//!
//! Layout is built ONCE (when the app list + surface dims are known, and
//! again on resize) into a flat draw list + tile hit-rects; `render_frame`
//! just replays it. No per-frame allocation churn, no animation loop.

wit_bindgen::generate!({
    world: "my:skiko-gfx/launcher-app",
    path: ["../../../proposals/wasi-canvas/wit", "wit"],
    generate_all,
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::frame_pacing::Guest as FramePacingGuest;
use crate::exports::my::skiko_gfx::renderer::{Guest, KeyKind, PointerKind};
use crate::my::skiko_gfx::launcher;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::types as wtypes;

// ── State ───────────────────────────────────────────────────────────────

#[derive(Clone)]
enum DrawItem {
    /// Rounded-rect fill (letter tile).
    Tile { x: f32, y: f32, w: f32, h: f32, color: u32 },
    /// Laid-out paragraph (index into `State::paras`) at a top-left origin.
    Text { para: usize, x: f32, y: f32 },
}

struct HitRect { x: f32, y: f32, w: f32, h: f32, app_id: String }

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    loaded: bool,
    apps: Vec<(String, String)>, // (app-id, label)
    items: Vec<DrawItem>,
    hits: Vec<HitRect>,
    paras: Vec<Para>, // paragraph resources we own (rebuilt on relayout)
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

const BG: u32 = 0xFF1A1A2E;
const TILE_PALETTE: [u32; 8] = [
    0xFF4285F4, 0xFFEA4335, 0xFFFBBC05, 0xFF34A853,
    0xFFAB47BC, 0xFF00ACC1, 0xFFFF7043, 0xFF5C6BC0,
];

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

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> wtypes::RoundedRect {
    let c = wtypes::Point { x: r, y: r };
    wtypes::RoundedRect {
        rect: wtypes::Rect { x, y, width: w, height: h },
        top_left: c,
        top_right: c,
        bottom_right: c,
        bottom_left: c,
    }
}

fn tile_color(app_id: &str) -> u32 {
    let mut h: u32 = 0;
    for b in app_id.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    TILE_PALETTE[(h as usize) % TILE_PALETTE.len()]
}

/// Laid-out paragraph (wasi:canvas/layout) — color baked at build time;
/// real width/height metrics (retires the per-glyph-advance approximations).
struct Para {
    p: wlayout::Paragraph,
    width: f32,
    height: f32,
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
    let width = p.max_intrinsic_width();
    let height = p.height();
    let baseline = p.alphabetic_baseline();
    Para { p, width, height, baseline }
}

/// Parse the host's newline / TAB-delimited `list-apps` output, dropping
/// the launcher's own entry so it doesn't show a tile for itself.
fn load_apps() -> Vec<(String, String)> {
    let raw = launcher::list_apps();
    raw.lines()
        .filter(|l| !l.is_empty())
        .map(|l| match l.split_once('\t') {
            Some((id, label)) => (id.to_string(), label.to_string()),
            None => (l.to_string(), l.to_string()),
        })
        .filter(|(id, _)| id != "wandr.launcher")
        .collect()
}

/// Rebuild the draw list + hit-rects for the current dims + app list.
fn relayout(s: &mut State) {
    // Dropping the Para resources releases the host-side paragraphs.
    s.paras.clear();
    s.items.clear();
    s.hits.clear();

    let margin = 48.0_f32;
    // Status-bar / taskbar insets are now applied uniformly by the host
    // (task 56 — it shrinks the logical size + translates content into the
    // chrome gap), so the launcher lays out from its own logical (0,0); no
    // manual top inset here (would double-count with the host's).
    let top_inset = 0.0_f32;
    // Title — "Apps" header, 2× the original size.
    let title_size = 88.0_f32;
    let title = para("Apps", title_size, 600, 0xFFFFFFFF);
    // Baseline one em below the top edge so the larger glyphs aren't clipped
    // (same placement as before, but via the REAL baseline metric).
    let title_top = top_inset + title_size - title.baseline;
    let title_bottom = title_top + title.height;
    s.items.push(DrawItem::Text { para: s.paras.len(), x: margin, y: title_top });
    s.paras.push(title);

    // Grid. Tiles are 2× the original size (task 57 follow-up).
    let tile = 264.0_f32;
    let gap = 56.0_f32;
    // App-name label, 2× the original size; reserve room (em + descent pad)
    // below each tile so the taller text fits within the cell.
    let label_size = 56.0_f32;
    let label_h = label_size + 24.0_f32;
    let cell_w = tile + gap;
    let cell_h = tile + label_h + gap;
    let usable = (s.w - margin * 2.0).max(cell_w);
    let cols = ((usable + gap) / cell_w).floor().max(1.0) as usize;
    let top = title_bottom + gap;

    for (i, (id, label)) in s.apps.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = margin + col as f32 * cell_w;
        let y = top + row as f32 * cell_h;
        s.items.push(DrawItem::Tile { x, y, w: tile, h: tile, color: tile_color(id) });

        // Letter, truly centered on the tile (real paragraph metrics).
        let letter = label.chars().next().unwrap_or('?').to_uppercase().to_string();
        let lpa = para(&letter, 120.0, 600, 0xFFFFFFFF);
        s.items.push(DrawItem::Text {
            para: s.paras.len(),
            x: x + (tile - lpa.width) * 0.5,
            y: y + (tile - lpa.height) * 0.5,
        });
        s.paras.push(lpa);

        // Label under the tile, truncated to the tile width using REAL
        // measured widths (no glyph-advance approximation).
        let mut disp = label.clone();
        let mut tpa = para(&disp, label_size, 400, 0xFFE0E0E0);
        while tpa.width > tile && disp.chars().count() > 1 {
            let keep = disp.chars().count().saturating_sub(2);
            disp = disp.chars().take(keep).collect::<String>() + "…";
            tpa = para(&disp, label_size, 400, 0xFFE0E0E0);
        }
        s.items.push(DrawItem::Text { para: s.paras.len(), x, y: y + tile + (label_h - tpa.height) * 0.5 });
        s.paras.push(tpa);

        s.hits.push(HitRect { x, y, w: tile, h: tile + label_h, app_id: id.clone() });
    }
}

// ── Renderer export ──────────────────────────────────────────────────────

struct Launcher;

impl Guest for Launcher {
    fn render_frame(_nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            let cv = wctx(|x| x.get_current_buffer());
            if s.w == 0.0 {
                s.w = cv.width();
                s.h = cv.height();
            }
            if !s.loaded {
                s.apps = load_apps();
                s.loaded = true;
                relayout(&mut s);
            }
            cv.draw_rect(wtypes::Rect { x: 0.0, y: 0.0, width: s.w, height: s.h }, &paint(BG));
            for it in s.items.iter() {
                match *it {
                    DrawItem::Tile { x, y, w, h, color } => {
                        cv.draw_rounded_rect(rrect(x, y, w, h, 52.0), &paint(color));
                    }
                    DrawItem::Text { para, x, y } => {
                        s.paras[para].p.paint(&cv, wtypes::Point { x, y });
                    }
                }
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
            if s.loaded {
                relayout(&mut s);
            }
        });
    }

    fn on_pointer_event_v2(_pid: u32, kind: PointerKind, x: f32, y: f32, _pressure: f32) {
        if !matches!(kind, PointerKind::Down) {
            return;
        }
        let target = STATE.with(|st| {
            let s = st.borrow();
            s.hits
                .iter()
                .find(|r| x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h)
                .map(|r| r.app_id.clone())
        });
        if let Some(app_id) = target {
            launcher::launch_app(&app_id);
        }
    }

    // Unused inputs.
    fn on_pointer_event(_kind: PointerKind, _x: f32, _y: f32) {}
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    fn on_scheduled_callback(_callback_id: u32) {}
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

/// Task 64 — the home screen is fully static (layout built once, no
/// animation). Always idle; the host wakes us on input via its own
/// dirty-tracking, and clamps this to ~1 s so a stale frame can't persist.
const IDLE: u32 = 60_000;

impl FramePacingGuest for Launcher {
    fn next_frame_delay() -> u32 {
        IDLE
    }
}

export!(Launcher);
