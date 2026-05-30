# Task 64 — On-demand rendering (stop the 60 fps idle CPU burn)

🟢 **Steps 1–3 device-verified 2026-05-30.** Host gate + the three Rust canvas
guests + dioxus all on-demand; Compose (step 4) deferred (heavy republish).
Spun out of a CPU investigation. Self-contained for a fresh session.

## Results (steps 1–3, device-verified 2026-05-30)

Implemented the optional `frame-pacing` WIT interface + a `frame-pacing-world`
probe world (host binds it `.ok()` exactly like `ime-events`). The standalone
loop (`standalone.rs`) keeps the cheap ~60 Hz input/IME/scheduler poll but gates
the expensive `render_frame` + buffer swap behind `dirty || now >= next_render_at
|| frame < 3`; a guest exporting `frame-pacing` drives `next_render_at` (clamped
to `IDLE_CAP=1000` ms), one that doesn't falls back to `POLL_MS=16` (legacy 60 fps
— **no breaking change**). Probe confirmed live in logcat: `loader: app exports
my:skiko-gfx/frame-pacing — on-demand rendering enabled`.

Idle CPU on Pixel 2 XL (`top -b`, foreground guest + bars):

| Process | Baseline | After |
|---|---|---|
| war.launcher | 23% | ~5% |
| war.statusbar | 20% | ~8% |
| war.taskbar | 15% | ~6% |
| war.dioxus.demo (idle) | 55% | ~6% |
| **Compose wart-app (regression gate)** | ~45% | **~45% (unchanged — None path)** |

The full chrome stack idle dropped from ~140% user to ~25% total. Residual ~5–8%
per process is the documented 60 Hz cheap-poll; the `poll()`/epoll refinement to
reach ~0% is still out of scope (see Gotchas → Later refinement).

Per-guest pacing: **launcher** → IDLE (static); **taskbar** → `0` while the tap-
flash is live (`STATE.pressed.is_some()`), else IDLE — vsync-blocking `end-frame`
paces the 8-frame flash to ~60 fps; **statusbar** → 1000, and its clock/battery
refresh was moved off the `frame % 60` gate to **per-render** (frame-count no
longer maps to wall-clock once gated — the key trap); **dioxus** → `0` while the
`DomRenderer.dirty` flag is set (event-driven, no async/timers), else IDLE.

Observation (not fixed, out of scope): a *backgrounded* Compose app (None path)
keeps rendering 60 fps (~45%) even when its SF surface is hidden. Pre-existing,
not a task-64 regression. Step 4 (Compose frame-pacing) and/or a visibility-gate
(skip render when `set_visible(false)`) would address it — noted for follow-up.

## Follow-up: per-app FPS cap (device-verified 2026-05-30)

A complementary lever to frame-pacing. Frame-pacing skips rendering when content
is *static*; the FPS cap limits the rate when content *is* changing — and,
crucially, it is enforced **entirely host-side**, so it caps **every** app
(Compose / dioxus / canvas) with **zero app- or library-side code**. It also
works on the legacy/None path, so it throttles **Compose apps now**, before
step 4.

Mechanism: `standalone.rs` resolves a `target_fps` (env `WART_MAX_FPS` > the
installed `package.toml` `max_fps` > 60) into a `frame_interval = 1000/fps` floor
on the render rate. The render gate becomes `frame < 3 || now >= next_render_at ||
(dirty && cap_ok)` where `cap_ok = (now - last_render) >= frame_interval`, and the
post-render delay is `max(guest_delay, frame_interval)`. Input is still polled at
`POLL_MS=16`, so touch latency is unchanged. `max_fps` is read like `orientation`
(`LoadedApp::max_fps()` off the install dir's `package.toml`; NOT in the AOT
cache-key). Files: `app_loader.rs` (`max_fps()`), `standalone.rs` (resolve +
`frame_interval` + `last_render` + gate). No `wit/` or guest changes.

Device results (Pixel 2 XL): manifest path confirmed (launcher no field →
`render cap 60 fps`; dioxus `max_fps=30` → `render cap 30 fps`). Compose wart-app
scales **45% (uncapped, ~40 fps-bound) → 37% (30 fps) → 26% (15 fps)**. Caveat:
the device already renders Compose at only ~40 fps (a frame is ~25 ms), so a
30 fps cap trims only ~25%; the cap helps most for fast-but-frequent renderers,
while the bigger Compose idle win remains step 4 (idle-skip). `war.dioxus.demo`
ships `max_fps = 30` as a sensible default for a reactive UI.

## Architecture note: where on-demand logic belongs

The **FPS cap** is host-only (host owns loop timing) — no app/library code. The
**idle-skip** signal (`am-I-idle?`) must originate in the rendering layer, so it
belongs in the shared libraries, not per app: **Compose → skiko** (`RendererImpl`
`hasInvalidations()`; the `frame-pacing` export goes in skiko's *generated*
bindings, so every Compose app inherits it free — that's step 4), **dioxus →
`crates/dioxus-canvas`** (the `next_frame_delay()` brains already live there; the
~6-line forwarding export in `war.dioxus.demo` should be collapsed into a
`dioxus_canvas::export_app!()` macro). Only the hand-rolled `war.launcher` /
`war.statusbar` / `war.taskbar` guests are genuinely per-app (no shared library).

### Follow-on: `dioxus_canvas::launch!` — dioxus guest = pure dioxus + one line

Acting on the rule above (apps depend only on the library; no backend leak), the
dioxus guest boilerplate was collapsed into a library macro
(`crates/dioxus-canvas/src/launch.rs`, device-verified mechanism 2026-05-30). A
guest is now its components + `dioxus_canvas::launch!(app)` (optional `pre_frame:
|r| ...`), depending **only on `dioxus` + `dioxus-canvas`** — no `.wit`, no
`wit_bindgen::generate!`, no `HostSink`, no `export!`, no wit-bindgen dep. The
library stays WIT-agnostic; the macro expands in the guest cdylib (where
component exports must live) and emits one `generate!` over the full
`my:skiko-gfx` world + the `CanvasSink` adapter + `measure_text`/`editor_*`
helpers + the renderer/frame-pacing Guest impls. wit-bindgen bumped 0.46→0.57.1;
the guest reaches it via a re-export + `runtime_path`. `war.dioxus.demo` is now
pure dioxus. Details + traps (single-generate!-only; delete the stale `wit/`
dir; `pub_export_macro`) in [[reference_dioxus_taffy_rust_ui]].

---

## Original scope

## Problem (measured on device 2026-05-30)

Every guest renders at **60 fps unconditionally**, even when its content is
static. With the full chrome stack running (5 processes), idle CPU is ~140%
user + 57% sys:

| Process | CPU | State |
|---|---|---|
| war.dioxus.demo (fg) | ~55% | **not animating** — pure render-loop tax |
| war.ime.keyboard | ~30% | **hidden / idle** |
| war.launcher | ~23% | **static** home screen |
| war.statusbar | ~20% | ~1 Hz content (clock) |
| war.taskbar | ~15% | **static** |

Root cause: `runtime/wart-host/src/standalone.rs` render loop calls
`call_render_frame` + `eglSwapBuffers` every 16 ms regardless of whether
anything changed. Not a task-62/63 regression — the loop always rendered
unconditionally; it became costly as each chrome surface got its own process +
60 fps loop. The dioxus app at 55% **while not animating** confirms it's general,
not animation-specific.

## Best practice (how Android / Compose do it)

On-demand, invalidation-driven, vsync-paced — the opposite of a fixed loop:

- **Vsync is requested on demand.** A static app requests no vsync → the system
  generates no frames for it → its thread sleeps (≈0 CPU).
- **`Choreographer.postFrameCallback` registers one frame.** Continuous
  animation = re-post each frame; animation ends → stop posting → idle. Draw is
  posted only on `invalidate()` / `requestLayout()`.
- **Compose** recomposes only on observed `State` change; `withFrameNanos` ticks
  only while there are awaiters. The CMP desktop renderer renders only when
  `ComposeScene.hasInvalidations()` is true. `delay()` / cursor blink schedule a
  single timed wake, not 60 fps.

Rule: render only when **(a)** content invalidated, **(b)** an animation wants the
next frame, **(c)** a timer is due, or **(d)** input arrived — else sleep. See
sources in the chat thread (Android Choreographer / Compose phases docs).

## Design

**Signal (per framework): "ms until the next frame you want."** A new OPTIONAL
WIT interface the host probes for; guests adopt it one at a time (no breaking
change to `render-frame`, so Compose guests keep working at 60 fps until their
step lands):

```wit
// wit/skiko-gfx.wit — new interface, NOT a required world export.
interface frame-pacing {
    /// Called by the host right after render-frame. Returns ms until the
    /// next frame the guest wants: 0 = animating / more work this frame,
    /// a finite N = a timed wake (delay()/cursor blink), large = idle.
    next-frame-delay: func() -> u32;
}
```

The guest computes it from post-render state:
- **Compose** (skiko): `0` if `ComposeScene.hasInvalidations()`; else the nearest
  `WasiFrameDispatcher` pending `delay`/awaiter deadline (ms from now); else IDLE.
- **dioxus** (`crates/dioxus-canvas`): `0` if the `VirtualDom` has pending work /
  dirty scopes; else IDLE. (Reactive → idle between events.)
- **canvas guests** (launcher/taskbar): IDLE (static). **status bar**: ~1000
  (clock refresh).

**Host loop** (`standalone.rs`): keep a *cheap* per-iteration input/event poll for
responsiveness, but gate the expensive `render_frame`+swap:
- `dirty` this iteration = any input dispatched, IME-inbound event, lifecycle /
  screen / orientation change, or scheduled-callback fired.
- After a render, if the instance exports `frame-pacing`, call `next-frame-delay`
  → `d`; set `next_render_at = now + clamp(d, 0, IDLE_CAP)` (IDLE_CAP ≈ 1000 ms).
  If the guest does NOT export it → `next_render_at = now + 16 ms` (legacy 60 fps,
  no regression).
- Render when `dirty || now >= next_render_at || first few frames`.
- Sleep `min(next_render_at - now, POLL_MS)` with `POLL_MS ≈ 16` so idle input
  latency stays ≤16 ms while the expensive render is skipped. The residual idle
  cost is just the cheap poll (~1–3%), down from 15–55%.

**Probing for the optional export** (host): use the low-level
`Instance::get_func(&mut store, "my:skiko-gfx/frame-pacing@0.1.0#next-frame-delay")`
once after instantiation; `Some` ⇒ throttle, `None` ⇒ legacy 60 fps. Cache the
typed func.

## Execution order (start here — Rust first, Compose last)

The Compose path needs the heavy skiko republish + 11 compose-*-wasi rebuilds +
wart-app/IME recompile (see `feedback_rebuild_compose_after_skiko`), so do it
last. The Rust guests are quick and deliver the biggest chunk (launcher + bars +
dioxus ≈ the bulk of the waste).

1. **Host scaffold + WIT** — add the `frame-pacing` interface to the canonical
   `wit/skiko-gfx.wit` (definition only; not required by any world yet). Restructure
   the `standalone.rs` loop: dirty-tracking + `next_render_at` gate + optional
   `frame-pacing` probe. With no guest exporting it yet, behavior is unchanged
   (60 fps) — verify no regression. Also handle the `run_once.rs` / `lib.rs`
   call sites (they can keep calling render-frame directly; throttle only the
   standalone loop).
2. **Canvas guests** (`apps/system/war.launcher`, `war.statusbar`, `war.taskbar`):
   add `frame-pacing` to each trimmed WIT (`*/wit/*.wit`) + world export, implement
   `next_frame_delay()` (launcher/taskbar → IDLE; statusbar → 1000). Rebuild via
   `build-system-warpkgs.sh` (Rust wasm32-wasip2, fast). **Device-verify CPU drop**
   for these three (expect ~58% → ~3% combined).
3. **dioxus** (`crates/dioxus-canvas` + `apps/user/war.dioxus.demo`): add
   `frame-pacing` to `war.dioxus.demo/wit/skiko-gfx.wit` + world; `dioxus-canvas`
   exposes a `next_frame_delay()` from VirtualDom pending state; demo's
   `render_frame` sibling exports it. Rebuild. **Device-verify** dioxus idle CPU
   drops (~55% → low single digits) and interaction still 60 fps.
4. **Compose** (skiko + compose-wasi) — LAST, heavy: `RendererImpl`/`SkiaLayerWasi`
   compute the delay from `ComposeScene.hasInvalidations()` + `WasiFrameDispatcher`
   pending deadline; add the `frame-pacing` export to the generated bindings +
   `skiko-gfx.wit` deps mirrors (wart-app + IME). Republish skiko, rebuild the 11
   compose-*-wasi modules, recompile wart-app + IME. **Device-verify** idle wart-app
   + hidden IME drop to near-0 while animations/cursor-blink still work.

## Surface (exact files)

- **WIT** (add `frame-pacing`): `wit/skiko-gfx.wit` (canonical) + mirror to
  `external/skiko/skiko/wit/skiko-gfx.wit`, `apps/user/wart-app/wit/deps/skiko-gfx/`,
  `apps/system/war.ime.keyboard/wit/deps/skiko-gfx/`; trimmed:
  `apps/system/war.{launcher,statusbar,taskbar}/wit/*.wit`,
  `apps/user/war.dioxus.demo/wit/skiko-gfx.wit`. (See the WIT-sync rule in CLAUDE.md.)
- **Host**: `runtime/wart-host/src/standalone.rs` (loop @ ~707–883, the
  `call_render_frame` @ 843 + a second @ ~906; `frame_target`/sleep @ ~879).
  Call sites also in `lib.rs:340` (NativeActivity) + `run_once.rs` (one-shot — leave).
- **Canvas guests**: `apps/system/war.launcher/src/lib.rs:163`,
  `war.statusbar/src/lib.rs:71`, `war.taskbar/src/lib.rs:100`.
- **dioxus**: `crates/dioxus-canvas/src/lib.rs:187` (`render_frame`),
  `apps/user/war.dioxus.demo/src/lib.rs:138`.
- **Compose**: `external/skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/wasi/RendererImpl.kt`,
  `.../SkiaLayerWasi.kt`, `.../generated/` exports;
  `external/compose-multiplatform-core/.../WasiFrameDispatcher.kt` (expose pending-deadline).

## Gotchas

- **`flush()` is the Compose heartbeat** (`WasiFrameDispatcher.kt` header): timed
  wakes fire on `flush()` inside `render_frame`. The `next-frame-delay` MUST
  include the nearest pending `delay`/awaiter deadline, or `delay()`/cursor-blink
  would never fire once idle. Don't just return invalidations.
- **Optional, not breaking**: keep `render-frame` signature unchanged; the new
  interface is a separate optional export so step order works without breaking
  un-migrated guests.
- **Input latency**: the cheap poll loop must keep running ~60 fps (POLL_MS≈16);
  only the render is gated. Don't sleep long between input polls.
- **Orientation lock file read** (task 63) currently runs every frame for
  overlays — fine to leave, or gate behind dirty/heartbeat (minor).
- **Later refinement** (closest to Android): replace the cheap poll with a real
  `poll()`/epoll on the input-channel fd + a timerfd so a fully idle process is
  ~0% (not a light poll). Out of scope for v1.

## Verify

`adb shell top -b -n 2 -d 3 -o PID,%CPU,RES,CMD | grep wart-host` before/after each
step. Targets: static launcher/taskbar ≈0–2%, status bar ≈1–2%, hidden IME ≈0%,
idle dioxus/wart-app ≈ low single digits; interaction stays 60 fps; Compose
cursor-blink + animations still work; status-bar clock still updates ~1 Hz.
