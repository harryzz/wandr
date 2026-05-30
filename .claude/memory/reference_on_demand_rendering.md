---
name: reference-on-demand-rendering
description: "High CPU root cause — guests render 60fps unconditionally; fix is on-demand rendering (task 64 steps 1-3 DONE 2026-05-30; Compose step 4 deferred)"
metadata: 
  node_type: memory
  type: reference
  originSessionId: d33723e5-8289-42cf-b090-a027d6a8e217
---

**Root cause of high idle CPU on device (found 2026-05-30):** the host render
loop (`runtime/wart-host/src/standalone.rs`) calls `call_render_frame` +
`eglSwapBuffers` **every 16 ms unconditionally** — a game-engine loop, not the
on-demand model UI toolkits use. Each surface is its own process with its own
60 fps loop, so static chrome wastes CPU re-rasterizing unchanged frames:
launcher ~23%, taskbar ~15%, hidden IME ~30%, status bar ~20%, idle (NOT
animating) dioxus ~55% → ~140% user + 57% sys across 5 processes. Measure with
`adb shell top -b -n 2 -d 3 -o PID,%CPU,RES,CMD | grep wart-host`.

**Best practice (Android/Compose), confirmed by research:** render only when
content invalidated, an animation wants the next frame, a timer is due, or input
arrived — else sleep. Android requests vsync on-demand (static app → no frames →
0 CPU); `Choreographer.postFrameCallback` registers ONE frame (re-post to keep
animating); Compose's desktop renderer renders only when
`ComposeScene.hasInvalidations()`; `delay()`/cursor-blink schedule a single timed
wake, not 60 fps.

**Fix = task 64** (`tasks/64-on-demand-rendering.md`): OPTIONAL WIT interface
`frame-pacing { next-frame-delay() -> u32 }` (ms until next wanted frame:
0=animating, N=timed wake, large=idle) + a probe-only `frame-pacing-world`. Host
binds it `.ok()` like `ime-events` (`InstantiatedApp.frame_pacing`); the
`standalone.rs` loop keeps the cheap ~60 Hz input/IME/scheduler poll but gates the
expensive `render_frame`+swap on `dirty || now>=next_render_at || frame<3`. Guest
sets the deadline (clamped IDLE_CAP=1000ms); no export ⇒ POLL_MS=16 (legacy 60fps,
no breaking change).

**STEPS 1–3 DONE + device-verified 2026-05-30** (this is what shipped):
- Step 1 host: `frame_pacing_bindings` bindgen module in `lib.rs`,
  `InstantiatedApp.frame_pacing` probe in `app_loader.rs`, dirty-gate loop in
  `standalone.rs` (`dirty=true` set from every event source; removed the old
  `t0`/`frame_target` per-frame sleep).
- Step 2 Rust canvas guests: launcher=IDLE(60000), taskbar=`0` while
  `pressed.is_some()` else IDLE, statusbar=1000 **and clock refresh moved off
  `frame%60` to PER-RENDER** (frame-count no longer maps to wall-clock once gated
  — the trap). Add `interface frame-pacing` + `export frame-pacing` to each
  trimmed `wit/*.wit`; impl the second `Guest` trait + same `export!`.
- Step 3 dioxus: `DomRenderer::next_frame_delay()` = `if self.dirty {0} else
  {60000}` (event-driven, no async/timers); demo exports it.
- Results: launcher 23→5%, statusbar 20→8%, taskbar 15→6%, dioxus 55→6%; Compose
  wart-app unchanged 45%/60fps (None path = regression gate passed). Logcat proof:
  `loader: app exports my:skiko-gfx/frame-pacing — on-demand rendering enabled`.
  Residual ~5–8%/proc is the 60 Hz cheap poll (epoll refinement out of scope).

**FOLLOW-UP SHIPPED — per-app FPS cap (device-verified 2026-05-30), HOST-ONLY:**
complementary to frame-pacing (cap = "no faster than X"; idle-skip = "nothing
when static"). Enforced entirely in `standalone.rs` → caps EVERY app
(Compose/dioxus/canvas) with ZERO app/library code, incl. the legacy None path
(so it throttles Compose NOW, before step 4). Resolution: `WART_MAX_FPS` env >
`package.toml max_fps` (read like `orientation` via `LoadedApp::max_fps()`, NOT in
cache-key) > 60. Gate: `frame<3 || now>=next_render_at || (dirty && (now-last_render)>=frame_interval)`;
post-render delay `max(guest_delay, frame_interval=1000/fps)`; input still polled
at POLL_MS=16. Scales: Compose wart-app 45%→37%(30fps)→26%(15fps). Caveat: Pixel
2 XL already renders Compose at only ~40 fps (~25 ms/frame), so 30-cap trims only
~25% — cap helps fast-but-frequent renderers; idle-skip is the bigger Compose win.
**Architecture rule (user-confirmed):** FPS cap = host-only; idle-skip signal must
live in the rendering LIBRARY not per-app — Compose→skiko generated bindings (step
4 = all Compose apps free), dioxus→`crates/dioxus-canvas` (brains there; collapse
the demo's forwarding export into a `dioxus_canvas::export_app!()` macro). Only the
hand-rolled launcher/statusbar/taskbar are legitimately per-app.

**CAP BUG FIXED 2026-05-30 (don't reintroduce):** the fps cap must NOT gate
`dirty`/input renders. First cut gated input with `cap_ok = (now-last_render) >=
frame_interval`; a quick tap (down-render, then up <frame_interval later — and the
dioxus `click` fires on UP) had its up-render skipped → deferred to the idle
deadline → **~1 s tap lag** (user-reported on the dioxus tabs at 30 fps). Fix in
`standalone.rs`: gate = `frame<3 || now>=next_render_at || dirty` (dirty always
renders); the cap floors only the TIMED/animation cadence via `next_render_at =
now + max(guest_delay, frame_interval)`. Consequence: for an event-driven guest
(dioxus) the cap is ~no-op (all its renders are input-driven → uncapped → smooth
scroll at input rate); the cap meaningfully limits only unconditional/animating
guests (Compose None-path). So `max_fps` on a reactive guest does little — it's a
Compose/animation knob.

**STEP 4 DONE — Compose frame-pacing (device-verified 2026-05-30, mechanism):**
idle wart-app 45%→~7%, hidden IME 30%→~8.5% — every guest (Rust + Compose) now in
the ~7–11% cheap-poll band. **No skiko change needed** (logic is in Compose + the
app export glue): `WasiFrameDispatcher.nextDeadlineMillis(now)` (0 if work
queued/due, else nearest delay/blink deadline, else MAX) + a top-level
`nextFrameDelayMillis()` in each guest's `RealComposeApp.kt` (0 if
`scene.hasInvalidations()`, else dispatcher deadline, else IDLE=100000) + a
`@WasmExport("my:skiko-gfx/frame-pacing@0.1.0#next-frame-delay")` in
`RendererExports.kt` + `export my:skiko-gfx/frame-pacing` in each `.wit` world —
in BOTH wart-app and war.ime.keyboard. Rebuild = `:compose:ui:ui`
republish only (additive method) + recompile each guest. The `flush()`-heartbeat
gotcha is honored (deadline included). **Build trap that ate hours:** the guests
were linking discarded out-of-tree fat bundles, not the in-tree modules — see
[[reference-compose-wasi-consumption]]. VISUAL CHECK still owed to the user
(animations / cursor-blink / IME typing) per [[feedback-visual-verification]].
**Gotcha:** reinstalling a PREHEATED system-app (IME) then launching crashes the
host (SIGSEGV in `get_exported_func`) on the stale preheated Component — restart
the zygote/stack after reinstalling a preloaded app (preload registry doesn't
invalidate on reinstall; follow-up). Still open: a *backgrounded* Compose app with
a focused editor keeps blinking the cursor (~11%) — frame-pacing got it from 45%,
but a visibility/role gate (skip render when backgrounded) would finish it.
