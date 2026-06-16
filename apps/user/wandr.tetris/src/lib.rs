//! wandr.tetris — the first wandr GAME (task 109, M1). A pure `wasi:canvas` +
//! `wasi:input-handlers` reactor run as a game loop: the board is redrawn every
//! tick from game state (no retained widget tree). Modelled on wandr.keyguard
//! (same canvas-context / paint / paragraph helpers).
//!
//! M1 = playable core, KEYS ONLY, verified on the desktop dev loop (task 101):
//! spawn, time-based gravity, move, soft/hard drop, simple rotation (SRS spawn
//! states, no wall-kicks yet), lock, line-clear (1–4), game-over, restart.
//! 7-bag/SRS-kicks/ghost/hold/scoring-curve/high-score = M2; device + touch = M3.

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
// Single fixed fall speed for M1 (gravity curve by level = M2). Time-based: the
// accumulator below counts real nanoseconds, never frames.
const FALL_NS: u64 = 800_000_000;

// ── Palette ──────────────────────────────────────────────────────────────────
const BG_APP: u32 = 0xFF0A0A12; // app background (matches the keyguard tone)
const WELL_BG: u32 = 0xFF14151E; // playfield interior
const GRID: u32 = 0xFF22232E; // cell grid lines
const BORDER: u32 = 0xFF3A3C4A; // well border
const HUD_FG: u32 = 0xFFFFFFFF;
const OVERLAY: u32 = 0xCC05060B; // dim scrim for pause / game-over
// Standard tetromino colors, indexed by PieceType (I,O,T,S,Z,J,L).
const COLORS: [u32; 7] = [
    0xFF00BCD4, // I cyan
    0xFFFFEB3B, // O yellow
    0xFF9C27B0, // T purple
    0xFF4CAF50, // S green
    0xFFF44336, // Z red
    0xFF2196F3, // J blue
    0xFFFF9800, // L orange
];

/// SRS spawn-state cell tables: SHAPES[kind][rot] = 4 (col,row) offsets within
/// the piece's local box. M1 rotation cycles rot 0→1→2→3 and reverts on
/// collision (no wall-kicks). I/O use a 4-wide box; the rest a 3-wide box.
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

struct State {
    board: Vec<Option<u8>>, // ROWS*COLS, row-major; Some(kind) = locked cell
    kind: u8,
    rot: u8,
    x: i32,
    y: i32,
    score: u32,
    lines: u32,
    paused: bool,
    game_over: bool,
    started: bool,
    rng: u64,
    accum_ns: u64,
    last_ns: u64,
    w: f32,
    h: f32,
    // Cached HUD paragraph, rebuilt only when score/lines change.
    hud: Option<(u32, u32, Para)>,
}

impl Default for State {
    fn default() -> Self {
        State {
            board: vec![None; ROWS * COLS],
            kind: 0,
            rot: 0,
            x: 3,
            y: 0,
            score: 0,
            lines: 0,
            paused: false,
            game_over: false,
            started: false,
            rng: 0,
            accum_ns: 0,
            last_ns: 0,
            w: 0.0,
            h: 0.0,
            hud: None,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

// One canvas-context per surface, lazily acquired (wasi:canvas idiom; same as
// the keyguard guest).
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

// ── Game logic (pure) ─────────────────────────────────────────────────────────
fn next_rng(s: &mut u64) -> u64 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *s = x;
    x
}

/// Would `kind`@`rot` at board cell (x,y) overlap a wall, the floor, or a locked
/// cell? Cells above the top (cy < 0) are allowed (spawn slack).
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

fn spawn(s: &mut State) {
    s.kind = (next_rng(&mut s.rng) % 7) as u8;
    s.rot = 0;
    s.x = 3;
    s.y = 0;
    if collides(&s.board, s.kind, s.rot, s.x, s.y) {
        s.game_over = true;
    }
}

/// Lock the active piece, clear full rows, score them, and spawn the next piece
/// (which may end the game). Standard line scores; the level curve is M2.
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
    s.lines += cleared;
    s.score += match cleared {
        1 => 100,
        2 => 300,
        3 => 500,
        4 => 800,
        _ => 0,
    };
    spawn(s);
}

/// Advance one row by gravity; lock if it can't fall.
fn step_down(s: &mut State) {
    if !collides(&s.board, s.kind, s.rot, s.x, s.y + 1) {
        s.y += 1;
    } else {
        lock_and_next(s);
    }
}

fn reset(s: &mut State) {
    s.board = vec![None; ROWS * COLS];
    s.score = 0;
    s.lines = 0;
    s.paused = false;
    s.game_over = false;
    s.accum_ns = 0;
    s.hud = None;
    spawn(s);
}

// ── Canvas helpers (lifted from the keyguard guest) ───────────────────────────
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

/// Laid-out paragraph (wasi:canvas/layout) — color baked, draws at a baseline
/// origin, carries its measured width for real centering.
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
    hud_baseline: f32,
    hud_size: f32,
    margin: f32,
}

fn layout(w: f32, h: f32) -> Layout {
    let margin = w * 0.03;
    let hud_h = h * 0.07; // top band for score/lines
    let avail_w = w - 2.0 * margin;
    let avail_h = h - hud_h - 2.0 * margin;
    let cell = (avail_w / COLS as f32).min(avail_h / ROWS as f32).floor().max(1.0);
    let board_w = cell * COLS as f32;
    let board_h = cell * ROWS as f32;
    let bx = (w - board_w) * 0.5;
    let by = hud_h + (h - hud_h - board_h) * 0.5;
    Layout {
        cell,
        bx,
        by,
        board_w,
        board_h,
        hud_baseline: hud_h * 0.72,
        hud_size: hud_h * 0.5,
        margin,
    }
}

fn draw_cell(cv: &Canvas, l: &Layout, cx: i32, cy: i32, color: u32) {
    let g = l.cell * 0.08;
    let x = l.bx + cx as f32 * l.cell + g;
    let y = l.by + cy as f32 * l.cell + g;
    let s = l.cell - 2.0 * g;
    cv.draw_rounded_rect(rrect(x, y, s, s, l.cell * 0.18), &paint(color));
}

// ── Rendering ─────────────────────────────────────────────────────────────────
fn render(s: &mut State, cv: &Canvas) {
    let (w, h) = (s.w.max(1.0), s.h.max(1.0));
    let l = layout(w, h);

    cv.clear(BG_APP);

    // Well interior + border.
    cv.draw_rect(rect(l.bx, l.by, l.board_w, l.board_h), &paint(WELL_BG));
    // Grid lines.
    let gp = stroke(GRID, 1.0);
    for c in 0..=COLS {
        let x = l.bx + c as f32 * l.cell;
        cv.draw_line(
            wtypes::Point { x, y: l.by },
            wtypes::Point { x, y: l.by + l.board_h },
            &gp,
        );
    }
    for r in 0..=ROWS {
        let y = l.by + r as f32 * l.cell;
        cv.draw_line(
            wtypes::Point { x: l.bx, y },
            wtypes::Point { x: l.bx + l.board_w, y },
            &gp,
        );
    }
    cv.draw_rect(rect(l.bx, l.by, l.board_w, l.board_h), &stroke(BORDER, 2.0));

    // Locked cells.
    for cy in 0..ROWS {
        for cx in 0..COLS {
            if let Some(t) = s.board[cy * COLS + cx] {
                draw_cell(cv, &l, cx as i32, cy as i32, COLORS[t as usize]);
            }
        }
    }

    // Active piece.
    if !s.game_over {
        for &(dx, dy) in &SHAPES[s.kind as usize][s.rot as usize] {
            let cx = s.x + dx as i32;
            let cy = s.y + dy as i32;
            if cy >= 0 {
                draw_cell(cv, &l, cx, cy, COLORS[s.kind as usize]);
            }
        }
    }

    // HUD (score / lines), cached until either value changes.
    let want = (s.score, s.lines);
    if s.hud.as_ref().map(|(sc, ln, _)| (*sc, *ln)) != Some(want) {
        let text = format!("SCORE {}     LINES {}", s.score, s.lines);
        s.hud = Some((s.score, s.lines, para(&text, l.hud_size, 600, HUD_FG)));
    }
    if let Some((_, _, p)) = &s.hud {
        draw_para(cv, p, l.margin, l.hud_baseline);
    }

    // Overlays.
    if s.game_over || s.paused {
        cv.draw_rect(rect(0.0, 0.0, w, h), &paint(OVERLAY));
        let title = if s.game_over { "GAME OVER" } else { "PAUSED" };
        let tp = para(title, h * 0.06, 700, HUD_FG);
        draw_para(cv, &tp, w * 0.5 - tp.width * 0.5, h * 0.46);
        let hint = if s.game_over { "press R to restart" } else { "press P to resume" };
        let hp = para(hint, h * 0.026, 400, 0xFFB8BCC8);
        draw_para(cv, &hp, w * 0.5 - hp.width * 0.5, h * 0.52);
    }
}

// ── Input ─────────────────────────────────────────────────────────────────────
fn handle_key(s: &mut State, code: &str) {
    // Restart and pause work regardless of state.
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
            if !collides(&s.board, s.kind, s.rot, s.x - 1, s.y) {
                s.x -= 1;
            }
        }
        "ArrowRight" => {
            if !collides(&s.board, s.kind, s.rot, s.x + 1, s.y) {
                s.x += 1;
            }
        }
        "ArrowDown" => {
            // Soft drop: one step now, gravity timer reset.
            step_down(s);
            s.accum_ns = 0;
        }
        "ArrowUp" | "KeyX" => {
            // Simple rotation (no wall-kicks yet — M2/SRS); revert on collision.
            let nr = (s.rot + 1) % 4;
            if !collides(&s.board, s.kind, nr, s.x, s.y) {
                s.rot = nr;
            }
        }
        "Space" => {
            while !collides(&s.board, s.kind, s.rot, s.x, s.y + 1) {
                s.y += 1;
            }
            lock_and_next(s);
            s.accum_ns = 0;
        }
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
            // First frame: seed RNG from the host clock and spawn.
            if !s.started {
                s.rng = nanos | 1;
                s.last_ns = nanos;
                spawn(&mut s);
                s.started = true;
            }

            // Time-based gravity (nanos deltas, never frame counts). Guard against
            // a non-monotonic or first-tick delta.
            if !s.paused && !s.game_over {
                let dt = nanos.saturating_sub(s.last_ns).min(FALL_NS * 4);
                s.accum_ns += dt;
                while s.accum_ns >= FALL_NS && !s.game_over {
                    s.accum_ns -= FALL_NS;
                    step_down(&mut s);
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
        // Act on key-down (incl. auto-repeat for move/soft-drop); ignore up.
        if !ev.down {
            return;
        }
        STATE.with(|st| handle_key(&mut st.borrow_mut(), &ev.code));
    }
}

impl PointerGuest for Tetris {
    // Touch arrives in M3; no-op for the keys-only M1.
    fn on_pointer(_ev: PointerEvent) {}
}

impl FramePacingGuest for Tetris {
    fn next_frame_delay() -> u32 {
        // ~60 Hz while a piece is falling; idle when paused / game-over (the host
        // clamps the long delay and re-wakes us on input).
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
    // Pause-on-background is wired in M3; M1 leaves this a no-op.
    fn on_lifecycle_changed(_new_state: crate::exports::wandr::ui_shell::shell_events::State) {}
}

export!(Tetris);
