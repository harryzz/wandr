# Task 72 — Background CPU floor (DESIGN/BACKLOG)

**Status:** DESIGN / BACKLOG (filed 2026-05-31). PROFILED 2026-05-31 — the
original premise (below) was **wrong**, see "Profiling result".

## Profiling result (2026-05-31) — it's NOT the TLS connection

A backgrounded fullscreen app sits at ~4–5.5% CPU with its surface hidden. The
attribution (proc `utime+stime` deltas over 12 s):

- **The launcher backgrounded — same host loop, NO engine — costs ~4.0%.** So the
  floor is the **host render loop**, app-agnostic, not the Signal connection.
- **The Signal engine (live TLS websocket) adds only ~1.6%** (`signal-bg 5.6% −
  launcher-bg 4.0%`). Real but not the floor.
- Within the host loop, the **60 Hz input poll** (`sf.poll_input` →
  native libgui `InputConsumer::consume`, `POLL_MS=16`) was a suspect; gating it
  to `BG_POLL_MS=200` when backgrounded (shipped, see below) cut launcher-bg
  4.0% → ~3.3% — so the input poll is only **~0.7%**, NOT the whole 4%.
- **~3.3% remains unattributed.** Leading suspect: the loop still RENDERS at the
  guest's idle cadence while backgrounded (the render gate isn't visibility-aware
  — `standalone.rs` `frame < 3 || now >= next_render_at || dirty`), and a
  hand-rolled renderer (the launcher) repaints + swaps even though hidden. A
  dioxus guest now skips its paint when paused (`commit e1dbed11`), but the
  launcher's own renderer does not. **Next:** instrument the loop stages
  (poll_input / render_frame / sf swap) with timing logs, or per-thread sample,
  to attribute the rest — then likely slow the background RENDER cadence for all
  renderers (a host-side `BG_RENDER_MS` floor on `next_render_at` when the role
  is Background), which the `BG_POLL_MS` input gate alone can't do because
  `next_render_at` (idle ~1/s) currently dominates the `nap`.

(original premise, now disproven — kept for context):

## Evidence (device-measured this session, proc `utime+stime` delta)

- Foreground, busy→idle: **14% → 7%** after adaptive idle frame pacing
  (`commit e25e83fd`) — render rate was the big foreground lever (8 fps → ~2 fps).
- Backgrounded: **7.1% → 5.6%** after lifecycle-aware throttle + skipping the
  paint while hidden (`commits e1dbed11` + `ebcd4ff5`).
- **Skipping the paint entirely only saved ~0.3%** (5.9% → 5.6%). So the repaint
  was never the dominant background cost — the remaining ~5.5% is the engine/
  connection. At a ~1/s background poll that's ~55 ms CPU *per poll*, which is
  suspiciously high for a non-blocking `poll` + idle socket → worth profiling
  (it may be wall-clock-bound TLS/keepalive work, or the step loop doing more
  than a single non-blocking sweep).

## Already shipped (render side — do NOT redo)

- Adaptive foreground idle cadence (`war.signal` pre_frame: 120→250→500 ms via an
  `IDLE_FRAMES` counter reset on engine activity). See [[reference_dioxus_taffy_rust_ui]].
- `dioxus-canvas` lifecycle plumbing: `on-lifecycle-changed` → `lifecycle_paused`
  (`is_paused()`); **`render_frame` early-returns when paused** (no relayout /
  draw-op replay / `eglSwapBuffers` to a hidden buffer); `pre_frame` still runs so
  a polling guest keeps ticking.
- `war.signal` throttles its poll cadence when `is_paused()` (host clamps to its
  `IDLE_CAP_MS = 1000` → ~1/s background poll).

## Why the host can't currently go below ~1/s in background

`runtime/wart-host/src/standalone.rs`: the render gate is
`frame < 3 || now >= next_render_at || dirty` — **not gated on visibility**. The
next-frame delay is `guest_delay.min(IDLE_CAP_MS).max(frame_interval)`, and
`IDLE_CAP_MS = 1000`, so even a guest asking for 2 s is rendered (→ `pre_frame`
pumped) at least 1/s. With the paint now skipped while paused, that 1/s call is
cheap on the render side, but it still pumps the engine 1/s.

## Candidate approaches (decide before implementing)

1. **Profile the per-poll engine cost first.** ~55 ms/poll at 1/s is the headline
   number to explain. Is it the `wasi:tls` read, prost decode of server
   keepalives, the step executor's poll sweep, or busy-work in `wstd`/the shim?
   Cheapest path to a real win is finding out where the 5.5% goes. (Tools:
   per-thread `top -H`, sampling, or temporary timing logs around `step()`.)
2. **Slow the background poll below 1/s.** Make `IDLE_CAP_MS` role-aware (e.g.
   2–5 s when the foreground app is backgrounded / a background pool member) so
   the engine polls less often when hidden. Tradeoff: incoming-message latency in
   the background rises to the cap; fine for a backgrounded chat. Host change in
   `standalone.rs`; pairs with the guest already asking for a slow cadence.
3. **Lengthen the websocket keepalive when backgrounded.** If the cost is
   keepalive churn, a longer interval cuts it. Engine/transport change.
4. **Suspend the socket when backgrounded + wake on a push.** The "proper"
   battery answer, but this stack has **no FCM/push channel** (Signal normally
   uses FCM). Would need a wake mechanism (a system push, a periodic alarm, or a
   lightweight always-on listener shared across apps). Biggest scope; likely a
   separate roadmap item.
5. **Host-side: skip the engine pump too when truly idle + backgrounded** — but
   that stops receiving, so only viable with (4)'s wake path. Not standalone.

## Recommendation

Start with **(1) profile**, then **(2) role-aware `IDLE_CAP_MS`** as the low-risk
win (slower background poll). (3) if keepalive is implicated. (4) is the real
battery fix but needs a push/wake story this stack doesn't have yet — treat as a
roadmap item, not a quick task.

## Relationship to other work

- Built on the Signal client ([[project_signal_resume_point]],
  [[project_signal_client_architecture]], [[project_wart_step_executor]]).
- Render-side levers + the "whole screen repaints every frame even when not dirty"
  note live in [[reference_dioxus_taffy_rust_ui]] and `tasks/64-on-demand-rendering.md`.
- The hybrid zygote/background-pool model is [[project_app_lifecycle_and_packaging]].
