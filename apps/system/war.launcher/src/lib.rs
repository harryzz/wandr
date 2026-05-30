//! war.launcher — the wart home screen as a LIGHT Rust canvas guest
//! (task 57). Exports `my:skiko-gfx/renderer` and draws the app grid
//! directly via the canvas WIT — no Kotlin/Compose, so no continuation
//! leak ([[feedback_indeterminate_progress_leak]]) and a tiny working
//! set, which matters for an always-running home process.
//!
//! Layout is built ONCE (when the app list + surface dims are known, and
//! again on resize) into a flat draw list + tile hit-rects; `render_frame`
//! just replays it. No per-frame allocation churn, no animation loop.

wit_bindgen::generate!({
    world: "launcher-app",
    path: "wit/launcher.wit",
});

use std::cell::RefCell;

use crate::exports::my::skiko_gfx::frame_pacing::Guest as FramePacingGuest;
use crate::exports::my::skiko_gfx::renderer::{Guest, KeyKind, PointerKind};
use crate::my::skiko_gfx::canvas::{
    self, BlendMode, ColorFilterKind, PaintAttrs, PaintStyle, StrokeCap, StrokeJoin,
};
use crate::my::skiko_gfx::launcher;

// ── State ───────────────────────────────────────────────────────────────

#[derive(Clone)]
enum DrawItem {
    /// Rounded-rect fill (letter tile).
    Tile { x: f32, y: f32, w: f32, h: f32, color: u32 },
    /// Text blob (host-owned id) at a baseline origin.
    Text { id: u32, x: f32, y: f32, color: u32 },
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
    blobs: Vec<u32>, // host text-blob ids we own (dropped on relayout)
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

const BG: u32 = 0xFF1A1A2E;
const TILE_PALETTE: [u32; 8] = [
    0xFF4285F4, 0xFFEA4335, 0xFFFBBC05, 0xFF34A853,
    0xFFAB47BC, 0xFF00ACC1, 0xFFFF7043, 0xFF5C6BC0,
];

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

fn tile_color(app_id: &str) -> u32 {
    let mut h: u32 = 0;
    for b in app_id.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    TILE_PALETTE[(h as usize) % TILE_PALETTE.len()]
}

const FAMILY: &[u8] = b"sans-serif";

fn new_blob(text: &str, size: f32, weight: u32) -> u32 {
    canvas::create_text_blob(text.as_bytes(), FAMILY, size, weight, false)
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
        .filter(|(id, _)| id != "war.launcher")
        .collect()
}

/// Rebuild the draw list + hit-rects for the current dims + app list.
fn relayout(s: &mut State) {
    // Drop the host text blobs from the previous layout.
    for id in s.blobs.drain(..) {
        canvas::drop_text_blob(id);
    }
    s.items.clear();
    s.hits.clear();

    let margin = 48.0_f32;
    // Status-bar / taskbar insets are now applied uniformly by the host
    // (task 56 — it shrinks the logical size + translates content into the
    // chrome gap), so the launcher lays out from its own logical (0,0); no
    // manual top inset here (would double-count with the host's).
    let top_inset = 0.0_f32;
    // Title.
    let title = new_blob("Apps", 44.0, 600);
    s.blobs.push(title);
    s.items.push(DrawItem::Text { id: title, x: margin, y: top_inset + 56.0, color: 0xFFFFFFFF });

    // Grid. Tiles are 2× the original size (task 57 follow-up).
    let tile = 264.0_f32;
    let gap = 56.0_f32;
    let label_h = 56.0_f32;
    let cell_w = tile + gap;
    let cell_h = tile + label_h + gap;
    let usable = (s.w - margin * 2.0).max(cell_w);
    let cols = ((usable + gap) / cell_w).floor().max(1.0) as usize;
    let top = top_inset + 116.0_f32;

    for (i, (id, label)) in s.apps.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = margin + col as f32 * cell_w;
        let y = top + row as f32 * cell_h;
        s.items.push(DrawItem::Tile { x, y, w: tile, h: tile, color: tile_color(id) });

        // Letter, centered-ish on the tile (baseline approx, scaled 2×).
        let letter = label.chars().next().unwrap_or('?').to_uppercase().to_string();
        let lblob = new_blob(&letter, 120.0, 600);
        s.blobs.push(lblob);
        s.items.push(DrawItem::Text { id: lblob, x: x + tile * 0.5 - 36.0, y: y + tile * 0.5 + 44.0, color: 0xFFFFFFFF });

        // Label under the tile (truncated).
        let mut disp = label.clone();
        if disp.chars().count() > 14 {
            disp = disp.chars().take(13).collect::<String>() + "…";
        }
        let tblob = new_blob(&disp, 28.0, 400);
        s.blobs.push(tblob);
        s.items.push(DrawItem::Text { id: tblob, x, y: y + tile + 36.0, color: 0xFFE0E0E0 });

        s.hits.push(HitRect { x, y, w: tile, h: tile + label_h, app_id: id.clone() });
    }
}

// ── Renderer export ──────────────────────────────────────────────────────

struct Launcher;

impl Guest for Launcher {
    fn render_frame(_nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            if s.w == 0.0 {
                s.w = canvas::surface_width() as f32;
                s.h = canvas::surface_height() as f32;
            }
            if !s.loaded {
                s.apps = load_apps();
                s.loaded = true;
                relayout(&mut s);
            }
            canvas::begin_frame();
            canvas::clear(BG);
            // Snapshot the items to avoid borrowing `s` across the draw
            // calls (the canvas imports don't touch STATE, but keep it
            // clean).
            let items = s.items.clone();
            for it in &items {
                match *it {
                    DrawItem::Tile { x, y, w, h, color } => {
                        canvas::draw_rrect(x, y, w, h, 52.0, 52.0, paint(color));
                    }
                    DrawItem::Text { id, x, y, color } => {
                        canvas::draw_text_blob(id, x, y, paint(color));
                    }
                }
            }
            canvas::end_frame();
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
