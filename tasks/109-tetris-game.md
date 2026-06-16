# Task 109 — Tetris (first wandr game)

> Scoped 2026-06-15. First game on wandr — proves the stack runs an interactive,
> continuously-animating, input-driven app, and exercises the `wasi:canvas` +
> `wasi:input-handlers` + frame-pacing loop as a *game loop* (not a widget tree).
> Decisions locked with the user: **pure `wasi:canvas` reactor** (no retained-mode
> framework) + **modern-guideline Tetris**.

## Why this shape

A game is immediate-mode by nature: a board redrawn every tick from game state,
not a declarative widget tree. The **chrome guests (keyguard / statusbar / taskbar)
are already pure `wasi:canvas` + `wasi:input-handlers` reactors** — that's the
template. Slint/dioxus would fight the game loop; raw canvas is lighter and the
right fit. Bonus: the **desktop dev loop** (task 101) runs the *same* wasm on
x86_64 with a HW keyboard (`WANDR_DESKTOP_SIZE=WxH wasm-android-host ui.wasm`),
so M1/M2 iterate fast on desktop and the identical binary deploys to the touch
device in M3.

## Where it lives

- `apps/user/wandr.tetris/` — new wandrpkg (`app_id = "wandr.tetris"`, label
  "Tetris", `orientation = portrait`, `max_fps = 60`, `background = false`).
- Pure Rust guest, `crate-type = ["cdylib"]`, built `cargo build --target
  wasm32-wasip2 --release` → `cp …/wandr_tetris.wasm components/ui.wasm` (the
  audio-player recipe).
- No new shared crate — game logic is self-contained Rust. No external deps
  required for M1–M3 (optional `rand`-free 7-bag uses an in-guest xorshift, as the
  audio player already does for shuffle).

## World (model on `wandr.keyguard/wit/world.wit`)

```wit
package wandr:tetris-app@0.1.0;
world tetris-app {
    import wasi:canvas/types@0.0.2;
    import wasi:canvas/draw@0.0.2;
    import wasi:canvas/layout@0.0.2;        // HUD text (score/level/lines)
    import wasi:canvas/embedding@0.0.2;
    export wasi:input-handlers/pointer-handler@0.0.2;  // touch (device)
    export wasi:input-handlers/key-handler@0.0.2;      // keys (desktop + HW kbd)
    export wasi:input-handlers/frame-handler@0.0.2;    // on-frame(nanos) = tick
    export wandr:ui-shell/frame-pacing@0.1.0;          // next-frame-delay
    export wandr:ui-shell/shell-events@0.1.0;          // on-lifecycle → pause on bg
}
```
Subset-copy the deps into `wit/deps/` (host provides supersets — the established
pattern). `/state` is preopened read-write for the high score.

## Binding rules (BINDING — from CLAUDE.md)

- **No hardcoding.** All geometry derives from `on-resize(width,height)`: cell
  size = `floor(min(w / COLS, playfield_h / ROWS))` after reserving HUD + (on
  device) the on-screen control band; board centered; HUD/next/hold boxes sized
  in cells. `COLS=10`, `ROWS=20` are the *rules of Tetris* (the one justified
  named constant set), not layout magic numbers.
- **Gravity is time-based**, accumulated from `on-frame(nanos)` deltas — never
  per-frame counts (frame rate varies desktop vs device). Frame-pacing returns a
  short delay while playing (smooth fall/animation), long when paused/game-over.

## Milestones

### M1 — playable core, keys only, on desktop (fast loop)
- Scaffold the guest + world + bindings; render: well border, locked cells
  (rounded rects, per-type color), active piece, basic HUD (score/lines).
- Mechanics: spawn, gravity (single fixed speed), left/right, soft drop, hard
  drop, **simple** rotation (no kicks yet), lock on landing, line clear (1–4),
  game over when spawn is blocked, restart.
- Input: keyboard only (Left/Right, Down soft, Up/X rotate, Space hard drop,
  R restart, P/Esc pause). W3C `code` tokens via `key-handler` (task 101 v3).
- **Verify on the desktop dev loop** (`WANDR_DESKTOP_SIZE`), not the device yet.

### M2 — modern guideline mechanics (still desktop)
- **7-bag** randomizer; **SRS** rotation with the standard **wall-kick** tables
  (incl. I-piece table); **ghost piece**; **hold** (one slot, locked until next
  lock); **next queue** (show 5); **lock delay** (~0.5 s, move-reset, capped).
- **Gravity curve** by level (guideline gravity per level; level-up every 10
  lines); soft-drop = faster gravity.
- **Scoring**: single/double/triple/tetris, **back-to-back** tetris, **combo**
  counter, soft/hard-drop points. (T-spin → M4.)
- **High score** persisted in `/state/highscore.json` (load on start, save on new
  best — same `/state` pattern as the audio player's config/EQ).

### M3 — device + touch + polish (user-verified)
- Deploy to Pixel 2 XL (`wandr-host --install` → `wandr-arbiter launch`).
- **Touch controls** (geometry derived from screen): on-screen button band
  (◀ ▶ rotate ⬇soft ⤓hard hold) hit-tested in `pointer-handler`, **plus** board
  gestures (tap board = rotate, swipe L/R = move, swipe down = soft, flick down =
  hard drop). Reliable buttons are primary; gestures are the nicety.
- **Pause on background** via `shell-events/on-lifecycle-changed` (Paused→pause,
  Resumed→keep paused until user resumes); frame-pacing goes idle when paused.
- Visual polish: standard tetromino palette (I cyan, O yellow, T purple, S green,
  Z red, J blue, L orange), grid lines, soft cell shadow, game-over overlay.
- **User visual verification** (`[[feedback_visual_verification]]`): playability,
  control feel, no jank; confirm on device.

### M4 — optional, after M3 verifies
- T-spin detection + scoring (3-corner rule) and back-to-back T-spin.
- Line-clear animation; level-up flash.
- **Sound via `wasi:audio`** — SFX (lock/clear/level) and/or music; the audio
  player already proves `wasi:audio` PCM works guest-side under `--no-art`.
- Settings: DAS/ARR tuning, ghost on/off (persisted in `/state`).

## Verification

- M1/M2: desktop dev loop — correctness of mechanics (kicks, scoring, 7-bag
  distribution), no panics, deterministic given a seed.
- M3: on device, `--no-art` (taps do NOT work via `adb input` — **real taps**);
  measure idle/play CPU (on-demand pacing should keep paused/idle cheap, like the
  ~0.7% Slint idle); user confirms feel.
- Checkpoint `.task-state` per milestone; add a STATUS.md row when M1 lands.

## Out of scope (record as findings, don't expand)
- Multiplayer / netplay; leaderboards beyond a local high score; replays.
- A reusable game-engine crate — keep it self-contained until a 2nd game asks.
- Non-Tetris game modes.

## Critical files / templates
- Template guest: `apps/system/wandr.keyguard/{src/lib.rs,wit/world.wit}` (pure
  canvas + input-handlers reactor; on_frame draws, pointer-handler hit-tests).
- Canvas verbs: `proposals/wasi-canvas/wit/` (`draw-rounded-rect`, `clear`,
  `draw-line`, `layout` text). Input: `proposals/wasi-input-handlers/wit/handlers.wit`.
- Packaging recipe: `apps/user/wandr.audio.player/{package.toml,Cargo.toml}` +
  `docs/build-pipeline.md`; desktop loop: `tasks/101-desktop-dev-loop.md`.
