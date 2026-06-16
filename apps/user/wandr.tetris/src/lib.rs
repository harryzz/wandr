//! wandr.tetris — the first wandr GAME (task 109). A pure `wasi:canvas` +
//! `wasi:input-handlers` reactor run as a game loop: the board is redrawn every
//! tick from game state (no retained widget tree). Modelled on wandr.keyguard.
//!
//! M1 = playable core (keys-only, desktop-verified).
//! M2 = modern guideline mechanics: 7-bag randomizer + next-queue, SRS rotation
//! with wall-kicks (incl. the I table), ghost piece, hold, lock delay
//! (move-reset, capped), per-level gravity curve, guideline scoring
//! (back-to-back + combo + drop points), and a persisted high score.
//! Device + touch = M3; T-spins + sound = M4.

wit_bindgen::generate!({
    world: "wandr:tetris-app/tetris-app",
    path: "wit",
    generate_all,
});

use std::cell::RefCell;

use crate::exports::wandr::ui_shell::frame_pacing::Guest as FramePacingGuest;
use crate::exports::wandr::ui_shell::shell_events::Guest as ShellEventsGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::key_handler::{Guest as KeyGuest, KeyEvent};
use crate::exports::wasi::input_handlers::pointer_handler::{Guest as PointerGuest, PointerEvent};
use crate::wasi::canvas::draw::Canvas;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::types as wtypes;

// ── Rules of Tetris (the one justified named-constant set — NOT layout magic) ──
const COLS: usize = 10;
const ROWS: usize = 20;
const NEXT_SHOWN: usize = 5;
/// Lock delay (guideline ~0.5 s), with move/rotate reset capped to avoid stalling.
const LOCK_DELAY_NS: u64 = 500_000_000;
const MAX_LOCK_RESETS: u32 = 15;
/// High score persisted in the writable `/state` preopen (same pattern as the
/// audio player's config/EQ); best-effort (absent on the desktop dev loop).
const HS_FILE: &str = "/state/tetris-highscore.json";

// ── Palette ──────────────────────────────────────────────────────────────────
const BG_APP: u32 = 0xFF0A0A12;
const WELL_BG: u32 = 0xFF14151E;
const GRID: u32 = 0xFF22232E;
const BORDER: u32 = 0xFF3A3C4A;
const PANEL_BG: u32 = 0xFF12131C;
const HUD_FG: u32 = 0xFFFFFFFF;
const HUD_DIM: u32 = 0xFF9AA0B4;
const OVERLAY: u32 = 0xCC05060B;
/// Standard tetromino colors, indexed by PieceType (I,O,T,S,Z,J,L).
const COLORS: [u32; 7] = [
    0xFF00BCD4, // I cyan
    0xFFFFEB3B, // O yellow
    0xFF9C27B0, // T purple
    0xFF4CAF50, // S green
    0xFFF44336, // Z red
    0xFF2196F3, // J blue
    0xFFFF9800, // L orange
];

/// SRS state cell tables: SHAPES[kind][rot] = 4 (col,row) offsets within the
/// piece's local box. rot 0=spawn, 1=R (CW), 2=180, 3=L (CCW). I/O use a 4-wide
/// box; the rest a 3-wide box.
const SHAPES: [[[(i8, i8); 4]; 4]; 7] = [
    // I
    [
        [(0, 1), (1, 1), (2, 1), (3, 1)],
        [(2, 0), (2, 1), (2, 2), (2, 3)],
        [(0, 2), (1, 2), (2, 2), (3, 2)],
        [(1, 0), (1, 1), (1, 2), (1, 3)],
    ],
    // O (does not rotate)
    [
        [(1, 0), (2, 0), (1, 1), (2, 1)],
        [(1, 0), (2, 0), (1, 1), (2, 1)],
        [(1, 0), (2, 0), (1, 1), (2, 1)],
        [(1, 0), (2, 0), (1, 1), (2, 1)],
    ],
    // T
    [
        [(1, 0), (0, 1), (1, 1), (2, 1)],
        [(1, 0), (1, 1), (2, 1), (1, 2)],
        [(0, 1), (1, 1), (2, 1), (1, 2)],
        [(1, 0), (0, 1), (1, 1), (1, 2)],
    ],
    // S
    [
        [(1, 0), (2, 0), (0, 1), (1, 1)],
        [(1, 0), (1, 1), (2, 1), (2, 2)],
        [(1, 1), (2, 1), (0, 2), (1, 2)],
        [(0, 0), (0, 1), (1, 1), (1, 2)],
    ],
    // Z
    [
        [(0, 0), (1, 0), (1, 1), (2, 1)],
        [(2, 0), (1, 1), (2, 1), (1, 2)],
        [(0, 1), (1, 1), (1, 2), (2, 2)],
        [(1, 0), (0, 1), (1, 1), (0, 2)],
    ],
    // J
    [
        [(0, 0), (0, 1), (1, 1), (2, 1)],
        [(1, 0), (2, 0), (1, 1), (1, 2)],
        [(0, 1), (1, 1), (2, 1), (2, 2)],
        [(1, 0), (1, 1), (0, 2), (1, 2)],
    ],
    // L
    [
        [(2, 0), (0, 1), (1, 1), (2, 1)],
        [(1, 0), (1, 1), (1, 2), (2, 2)],
        [(0, 1), (1, 1), (2, 1), (0, 2)],
        [(0, 0), (1, 0), (1, 1), (1, 2)],
    ],
];

/// SRS wall-kick offsets in standard (x, y-UP) convention; the caller negates y
/// for this grid's row-DOWN coordinates. 5 candidates per transition, tried in
/// order. JLSTZ share one table; I has its own; O never rotates.
fn kicks(is_i: bool, from: u8, to: u8) -> [(i8, i8); 5] {
    if is_i {
        match (from, to) {
            (0, 1) => [(0, 0), (-2, 0), (1, 0), (-2, -1), (1, 2)],
            (1, 0) => [(0, 0), (2, 0), (-1, 0), (2, 1), (-1, -2)],
            (1, 2) => [(0, 0), (-1, 0), (2, 0), (-1, 2), (2, -1)],
            (2, 1) => [(0, 0), (1, 0), (-2, 0), (1, -2), (-2, 1)],
            (2, 3) => [(0, 0), (2, 0), (-1, 0), (2, 1), (-1, -2)],
            (3, 2) => [(0, 0), (-2, 0), (1, 0), (-2, -1), (1, 2)],
            (3, 0) => [(0, 0), (1, 0), (-2, 0), (1, -2), (-2, 1)],
            (0, 3) => [(0, 0), (-1, 0), (2, 0), (-1, 2), (2, -1)],
            _ => [(0, 0); 5],
        }
    } else {
        match (from, to) {
            (0, 1) => [(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)],
            (1, 0) => [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
            (1, 2) => [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
            (2, 1) => [(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)],
            (2, 3) => [(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)],
            (3, 2) => [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
            (3, 0) => [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
            (0, 3) => [(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)],
            _ => [(0, 0); 5],
        }
    }
}

struct State {
    board: Vec<Option<u8>>, // ROWS*COLS, row-major; Some(kind) = locked cell
    kind: u8,
    rot: u8,
    x: i32,
    y: i32,
    queue: Vec<u8>, // next pieces, front = next to spawn (7-bag filled)
    hold: Option<u8>,
    can_hold: bool,
    score: u32,
    lines: u32,
    level: u32,
    combo: i32,    // -1 = no chain; bonus from the 2nd consecutive clear
    b2b: bool,     // a difficult (tetris) clear is "active" for the next one
    high_score: u32,
    paused: bool,
    game_over: bool,
    started: bool,
    rng: u64,
    gravity_accum_ns: u64,
    last_ns: u64,
    soft_drop: bool,
    lock_accum_ns: u64,
    lock_resets: u32,
    w: f32,
    h: f32,
    hud: Option<(u32, u32, u32, u32, [Para; 2])>, // (score,hi,level,lines) -> 2 lines
}

impl Default for State {
    fn default() -> Self {
        State {
            board: vec![None; ROWS * COLS],
            kind: 0,
            rot: 0,
            x: 3,
            y: 0,
            queue: Vec::new(),
            hold: None,
            can_hold: true,
            score: 0,
            lines: 0,
            level: 1,
            combo: -1,
            b2b: false,
            high_score: 0,
            paused: false,
            game_over: false,
            started: false,
            rng: 0,
            gravity_accum_ns: 0,
            last_ns: 0,
            soft_drop: false,
            lock_accum_ns: 0,
            lock_resets: 0,
            w: 0.0,
            h: 0.0,
            hud: None,
        }
    }
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

// ── High score (best-effort /state persistence) ───────────────────────────────
fn load_high_score() -> u32 {
    let Ok(s) = std::fs::read_to_string(HS_FILE) else { return 0 };
    s.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
}
fn save_high_score(hs: u32) {
    let _ = std::fs::create_dir_all("/state");
    let _ = std::fs::write(HS_FILE, format!("{{\"high_score\": {hs}}}\n"));
}

// ── Game logic (pure) ─────────────────────────────────────────────────────────
fn next_rng(s: &mut u64) -> u64 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *s = x;
    x
}

fn collides(board: &[Option<u8>], kind: u8, rot: u8, x: i32, y: i32) -> bool {
    for &(dx, dy) in &SHAPES[kind as usize][rot as usize] {
        let cx = x + dx as i32;
        let cy = y + dy as i32;
        if cx < 0 || cx >= COLS as i32 || cy >= ROWS as i32 {
            return true;
        }
        if cy >= 0 && board[cy as usize * COLS + cx as usize].is_some() {
            return true;
        }
    }
    false
}

/// Refill the next-queue with shuffled 7-bags until it can always show NEXT_SHOWN
/// after a pop (Fisher–Yates via the in-guest xorshift — no `rand` dependency).
fn refill_queue(s: &mut State) {
    while s.queue.len() <= NEXT_SHOWN {
        let mut bag: [u8; 7] = [0, 1, 2, 3, 4, 5, 6];
        for i in (1..7).rev() {
            let j = (next_rng(&mut s.rng) % (i as u64 + 1)) as usize;
            bag.swap(i, j);
        }
        s.queue.extend_from_slice(&bag);
    }
}

fn next_kind(s: &mut State) -> u8 {
    refill_queue(s);
    s.queue.remove(0)
}

/// Place a specific piece at the top spawn position; resets per-piece lock state
/// and ends the game if the spawn cell is blocked.
fn spawn(s: &mut State, kind: u8) {
    s.kind = kind;
    s.rot = 0;
    s.x = 3;
    s.y = 0;
    s.gravity_accum_ns = 0;
    s.lock_accum_ns = 0;
    s.lock_resets = 0;
    if collides(&s.board, s.kind, s.rot, s.x, s.y) {
        s.game_over = true;
        if s.score > s.high_score {
            s.high_score = s.score;
            save_high_score(s.high_score);
        }
    }
}

fn grounded(s: &State) -> bool {
    collides(&s.board, s.kind, s.rot, s.x, s.y + 1)
}

/// A successful move/rotate while resting refreshes the lock timer, capped.
fn touch_lock_reset(s: &mut State) {
    if grounded(s) && s.lock_resets < MAX_LOCK_RESETS {
        s.lock_accum_ns = 0;
        s.lock_resets += 1;
    }
}

fn try_move(s: &mut State, dx: i32) -> bool {
    if !collides(&s.board, s.kind, s.rot, s.x + dx, s.y) {
        s.x += dx;
        touch_lock_reset(s);
        true
    } else {
        false
    }
}

/// SRS rotation with wall-kicks; reverts (returns false) if no candidate fits.
fn rotate(s: &mut State, cw: bool) {
    if s.kind == 1 {
        return; // O never rotates
    }
    let from = s.rot;
    let to = if cw { (from + 1) % 4 } else { (from + 3) % 4 };
    let is_i = s.kind == 0;
    for (kx, ky) in kicks(is_i, from, to) {
        let nx = s.x + kx as i32;
        let ny = s.y - ky as i32; // SRS y is UP; this grid's row is DOWN
        if !collides(&s.board, s.kind, to, nx, ny) {
            s.x = nx;
            s.y = ny;
            s.rot = to;
            touch_lock_reset(s);
            return;
        }
    }
}

/// Lock the active piece, clear full rows, apply guideline scoring (line values
/// ×level, back-to-back tetris ×1.5, combo bonus), then spawn the next piece.
fn lock_and_next(s: &mut State) {
    for &(dx, dy) in &SHAPES[s.kind as usize][s.rot as usize] {
        let cx = s.x + dx as i32;
        let cy = s.y + dy as i32;
        if cy >= 0 && cy < ROWS as i32 && cx >= 0 && cx < COLS as i32 {
            s.board[cy as usize * COLS + cx as usize] = Some(s.kind);
        }
    }
    // Compact non-full rows toward the bottom.
    let mut nb = vec![None; ROWS * COLS];
    let mut wy: i32 = ROWS as i32 - 1;
    let mut cleared = 0u32;
    for ry in (0..ROWS).rev() {
        let full = (0..COLS).all(|cx| s.board[ry * COLS + cx].is_some());
        if full {
            cleared += 1;
            continue;
        }
        for cx in 0..COLS {
            nb[wy as usize * COLS + cx] = s.board[ry * COLS + cx];
        }
        wy -= 1;
    }
    s.board = nb;

    if cleared > 0 {
        s.combo += 1;
        let base = match cleared {
            1 => 100,
            2 => 300,
            3 => 500,
            4 => 800,
            _ => 0,
        };
        let difficult = cleared == 4;
        let mut pts = base;
        if difficult && s.b2b {
            pts = pts * 3 / 2; // back-to-back tetris ×1.5
        }
        pts *= s.level;
        s.score += pts;
        if s.combo > 0 {
            s.score += 50 * s.combo as u32 * s.level;
        }
        s.b2b = difficult;
        s.lines += cleared;
        s.level = s.lines / 10 + 1;
    } else {
        s.combo = -1; // chain broken; b2b survives a no-clear placement
    }

    if s.score > s.high_score {
        s.high_score = s.score;
        save_high_score(s.high_score);
    }

    let k = next_kind(s);
    spawn(s, k);
    s.can_hold = true;
}

/// Hold: swap the active piece into the one-slot hold; locked until the next
/// natural lock (one hold per piece).
fn hold(s: &mut State) {
    if !s.can_hold {
        return;
    }
    let cur = s.kind;
    match s.hold.take() {
        Some(h) => {
            s.hold = Some(cur);
            spawn(s, h);
        }
        None => {
            s.hold = Some(cur);
            let k = next_kind(s);
            spawn(s, k);
        }
    }
    s.can_hold = false;
}

fn hard_drop(s: &mut State) {
    while !collides(&s.board, s.kind, s.rot, s.x, s.y + 1) {
        s.y += 1;
        s.score += 2; // hard-drop: 2 pts/cell
    }
    lock_and_next(s);
}

/// Landing row for the ghost piece (where a hard drop would lock).
fn ghost_y(s: &State) -> i32 {
    let mut gy = s.y;
    while !collides(&s.board, s.kind, s.rot, s.x, gy + 1) {
        gy += 1;
    }
    gy
}

/// Guideline gravity: seconds-per-cell by level, → ns. Soft drop is 20× faster.
fn gravity_ns(s: &State) -> u64 {
    let l = s.level as f64;
    let secs = (0.8 - (l - 1.0) * 0.007).max(0.05).powf(l - 1.0);
    let ns = (secs * 1.0e9) as u64;
    if s.soft_drop {
        (ns / 20).max(1_000_000)
    } else {
        ns.max(1_000_000)
    }
}

fn reset(s: &mut State) {
    let seed = s.rng | 1;
    let hs = s.high_score;
    *s = State::default();
    s.rng = seed;
    s.high_score = hs;
    s.started = false; // re-seeds + spawns on the next frame
}

// ── Canvas helpers (lifted from the keyguard guest) ───────────────────────────
fn paint_a(color: u32, alpha: u8) -> wtypes::Paint<'static> {
    wtypes::Paint {
        style: wtypes::PaintStyle::Fill,
        color,
        alpha,
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
fn paint(color: u32) -> wtypes::Paint<'static> {
    paint_a(color, 255)
}
fn stroke(color: u32, width: f32) -> wtypes::Paint<'static> {
    let mut p = paint(color);
    p.style = wtypes::PaintStyle::Stroke;
    p.stroke_width = width;
    p
}
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
    let b = wlayout::ParagraphBuilder::new(&style);
    b.add_text(text);
    let p = wlayout::ParagraphBuilder::build(b);
    p.layout(1.0e6);
    let baseline = p.alphabetic_baseline();
    let width = p.max_intrinsic_width();
    Para { p, baseline, width }
}
fn draw_para(cv: &Canvas, pa: &Para, x: f32, baseline_y: f32) {
    pa.p.paint(cv, wtypes::Point { x, y: baseline_y - pa.baseline });
}

// ── Layout (derived from surface dims — no hardcoding) ─────────────────────────
struct Layout {
    cell: f32,
    bx: f32,
    by: f32,
    board_w: f32,
    board_h: f32,
    margin: f32,
    top_h: f32,
}
fn layout(w: f32, h: f32) -> Layout {
    let margin = w * 0.02;
    let top_h = h * 0.20; // HUD text + hold/next previews
    let avail_h = h - top_h - margin;
    let cell = ((w - 2.0 * margin) / COLS as f32)
        .min(avail_h / ROWS as f32)
        .floor()
        .max(1.0);
    let board_w = cell * COLS as f32;
    let board_h = cell * ROWS as f32;
    let bx = (w - board_w) * 0.5;
    let by = top_h + (h - top_h - board_h) * 0.5;
    Layout { cell, bx, by, board_w, board_h, margin, top_h }
}

fn draw_cell_at(cv: &Canvas, x: f32, y: f32, size: f32, color: u32, alpha: u8) {
    let g = size * 0.08;
    cv.draw_rounded_rect(rrect(x + g, y + g, size - 2.0 * g, size - 2.0 * g, size * 0.18), &paint_a(color, alpha));
}
fn draw_cell(cv: &Canvas, l: &Layout, cx: i32, cy: i32, color: u32, alpha: u8) {
    draw_cell_at(cv, l.bx + cx as f32 * l.cell, l.by + cy as f32 * l.cell, l.cell, color, alpha);
}

/// Draw a piece (spawn orientation) centered in the given box — for hold/next.
fn draw_preview(cv: &Canvas, bx: f32, by: f32, bw: f32, bh: f32, kind: u8, alpha: u8) {
    let cells = &SHAPES[kind as usize][0];
    let (mut minc, mut maxc, mut minr, mut maxr) = (3i8, 0i8, 3i8, 0i8);
    for &(c, r) in cells {
        minc = minc.min(c);
        maxc = maxc.max(c);
        minr = minr.min(r);
        maxr = maxr.max(r);
    }
    let used_w = (maxc - minc + 1) as f32;
    let used_h = (maxr - minr + 1) as f32;
    let pcell = (bw / 4.0).min(bh / 4.0);
    let ox = bx + (bw - used_w * pcell) * 0.5;
    let oy = by + (bh - used_h * pcell) * 0.5;
    for &(c, r) in cells {
        draw_cell_at(cv, ox + (c - minc) as f32 * pcell, oy + (r - minr) as f32 * pcell, pcell, COLORS[kind as usize], alpha);
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────
fn render(s: &mut State, cv: &Canvas) {
    let (w, h) = (s.w.max(1.0), s.h.max(1.0));
    let l = layout(w, h);

    cv.clear(BG_APP);

    // ── HUD text (cached until any displayed value changes) ──
    let want = (s.score, s.high_score, s.level, s.lines);
    let dirty = match &s.hud {
        Some((sc, hi, lv, ln, _)) => (*sc, *hi, *lv, *ln) != want,
        None => true,
    };
    if dirty {
        let l1 = para(&format!("SCORE {}", s.score), h * 0.030, 700, HUD_FG);
        let l2 = para(
            &format!("HI {}    LV {}    LINES {}", s.high_score, s.level, s.lines),
            h * 0.022,
            500,
            HUD_DIM,
        );
        s.hud = Some((s.score, s.high_score, s.level, s.lines, [l1, l2]));
    }
    if let Some((_, _, _, _, lines)) = &s.hud {
        draw_para(cv, &lines[0], l.margin, h * 0.040);
        draw_para(cv, &lines[1], l.margin, h * 0.072);
    }

    // ── Hold (left) + Next queue (right), in the band below the text ──
    // Sizes are DERIVED so the two regions can never overlap: HOLD is a square
    // capped to a fraction of the width; NEXT auto-fits NEXT_SHOWN squares into
    // whatever width remains to the right of HOLD.
    let band_y = h * 0.095;
    let band_h = (l.top_h - band_y - l.margin).max(1.0);
    let label = |cv: &Canvas, x: f32, txt: &str| {
        draw_para(cv, &para(txt, h * 0.015, 600, HUD_DIM), x + 2.0, band_y - 2.0);
    };

    // HOLD — square, left.
    let hold_s = band_h.min(w * 0.16);
    let hold_y = band_y + (band_h - hold_s) * 0.5;
    label(cv, l.margin, "HOLD");
    cv.draw_rounded_rect(rrect(l.margin, hold_y, hold_s, hold_s, hold_s * 0.12), &paint(PANEL_BG));
    if let Some(hk) = s.hold {
        let a = if s.can_hold { 255 } else { 110 };
        draw_preview(cv, l.margin, hold_y, hold_s, hold_s, hk, a);
    }

    // NEXT — fit NEXT_SHOWN squares into the width right of HOLD.
    let gap = w * 0.012;
    let next_left = l.margin + hold_s + w * 0.05;
    let next_right = w - l.margin;
    let avail = (next_right - next_left).max(1.0);
    let nb = (((avail - (NEXT_SHOWN as f32 - 1.0) * gap) / NEXT_SHOWN as f32))
        .min(band_h)
        .max(2.0);
    let total = NEXT_SHOWN as f32 * nb + (NEXT_SHOWN as f32 - 1.0) * gap;
    let start_x = next_right - total;
    let nb_y = band_y + (band_h - nb) * 0.5;
    label(cv, start_x, "NEXT");
    for i in 0..NEXT_SHOWN {
        if let Some(&k) = s.queue.get(i) {
            let nx = start_x + i as f32 * (nb + gap);
            cv.draw_rounded_rect(rrect(nx, nb_y, nb, nb, nb * 0.12), &paint(PANEL_BG));
            draw_preview(cv, nx, nb_y, nb, nb, k, 255);
        }
    }

    // ── Playfield ──
    cv.draw_rect(rect(l.bx, l.by, l.board_w, l.board_h), &paint(WELL_BG));
    let gp = stroke(GRID, 1.0);
    for c in 0..=COLS {
        let x = l.bx + c as f32 * l.cell;
        cv.draw_line(wtypes::Point { x, y: l.by }, wtypes::Point { x, y: l.by + l.board_h }, &gp);
    }
    for r in 0..=ROWS {
        let y = l.by + r as f32 * l.cell;
        cv.draw_line(wtypes::Point { x: l.bx, y }, wtypes::Point { x: l.bx + l.board_w, y }, &gp);
    }
    cv.draw_rect(rect(l.bx, l.by, l.board_w, l.board_h), &stroke(BORDER, 2.0));

    // Locked cells.
    for cy in 0..ROWS {
        for cx in 0..COLS {
            if let Some(t) = s.board[cy * COLS + cx] {
                draw_cell(cv, &l, cx as i32, cy as i32, COLORS[t as usize], 255);
            }
        }
    }

    if !s.game_over {
        // Ghost piece (translucent landing preview).
        let gy = ghost_y(s);
        if gy != s.y {
            for &(dx, dy) in &SHAPES[s.kind as usize][s.rot as usize] {
                let cy = gy + dy as i32;
                if cy >= 0 {
                    draw_cell(cv, &l, s.x + dx as i32, cy, COLORS[s.kind as usize], 60);
                }
            }
        }
        // Active piece.
        for &(dx, dy) in &SHAPES[s.kind as usize][s.rot as usize] {
            let cy = s.y + dy as i32;
            if cy >= 0 {
                draw_cell(cv, &l, s.x + dx as i32, cy, COLORS[s.kind as usize], 255);
            }
        }
    }

    // ── Overlays ──
    if s.game_over || s.paused {
        cv.draw_rect(rect(0.0, 0.0, w, h), &paint(OVERLAY));
        let title = if s.game_over { "GAME OVER" } else { "PAUSED" };
        let tp = para(title, h * 0.06, 700, HUD_FG);
        draw_para(cv, &tp, w * 0.5 - tp.width * 0.5, h * 0.46);
        let hint = if s.game_over { "press R to restart" } else { "press P to resume" };
        let hp = para(hint, h * 0.026, 400, HUD_DIM);
        draw_para(cv, &hp, w * 0.5 - hp.width * 0.5, h * 0.52);
    }
}

// ── Input ─────────────────────────────────────────────────────────────────────
fn handle_key(s: &mut State, code: &str, down: bool) {
    if !down {
        if code == "ArrowDown" {
            s.soft_drop = false;
        }
        return;
    }
    match code {
        "KeyR" => {
            reset(s);
            return;
        }
        "KeyP" | "Escape" => {
            if !s.game_over {
                s.paused = !s.paused;
            }
            return;
        }
        _ => {}
    }
    if s.paused || s.game_over {
        return;
    }
    match code {
        "ArrowLeft" => {
            try_move(s, -1);
        }
        "ArrowRight" => {
            try_move(s, 1);
        }
        "ArrowDown" => {
            s.soft_drop = true;
        }
        "ArrowUp" | "KeyX" => rotate(s, true),
        "KeyZ" => rotate(s, false),
        "KeyC" | "ShiftLeft" | "ShiftRight" => hold(s),
        "Space" => hard_drop(s),
        _ => {}
    }
}

// ── Guest exports ─────────────────────────────────────────────────────────────
struct Tetris;

impl FrameGuest for Tetris {
    fn on_frame(nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            let cv = wctx(|x| x.get_current_buffer());
            if s.w <= 0.0 {
                s.w = cv.width();
                s.h = cv.height();
            }
            if !s.started {
                s.rng = nanos | 1;
                s.last_ns = nanos;
                s.high_score = load_high_score();
                let k = next_kind(&mut s);
                spawn(&mut s, k);
                s.can_hold = true;
                s.started = true;
            }

            if !s.paused && !s.game_over {
                let dt = nanos.saturating_sub(s.last_ns).min(LOCK_DELAY_NS * 4);
                if grounded(&s) {
                    // Resting: count down the lock delay.
                    s.lock_accum_ns += dt;
                    if s.lock_accum_ns >= LOCK_DELAY_NS {
                        lock_and_next(&mut s);
                    }
                } else {
                    // Falling: apply gravity (time-based; soft drop = faster).
                    s.gravity_accum_ns += dt;
                    let interval = gravity_ns(&s);
                    while s.gravity_accum_ns >= interval {
                        s.gravity_accum_ns -= interval;
                        if !collides(&s.board, s.kind, s.rot, s.x, s.y + 1) {
                            s.y += 1;
                            if s.soft_drop {
                                s.score += 1; // soft-drop: 1 pt/cell
                            }
                        } else {
                            break;
                        }
                    }
                    // Descended → the lock window restarts at the new low.
                    s.lock_accum_ns = 0;
                    s.lock_resets = 0;
                }
            }
            s.last_ns = nanos;

            render(&mut s, &cv);
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

impl KeyGuest for Tetris {
    fn on_key(ev: KeyEvent) {
        STATE.with(|st| handle_key(&mut st.borrow_mut(), &ev.code, ev.down));
    }
}

impl PointerGuest for Tetris {
    fn on_pointer(_ev: PointerEvent) {} // touch arrives in M3
}

impl FramePacingGuest for Tetris {
    fn next_frame_delay() -> u32 {
        STATE.with(|st| {
            let s = st.borrow();
            if s.paused || s.game_over {
                500
            } else {
                16
            }
        })
    }
}

impl ShellEventsGuest for Tetris {
    fn on_scheduled_callback(_id: u32) {}
    fn on_lifecycle_changed(_new_state: crate::exports::wandr::ui_shell::shell_events::State) {}
}

export!(Tetris);
