# Converging wasi-gfx and the wandr canvas stack — both directions, one vocabulary

*Proposal notes, 2026-06-11. Companion to `docs/skiko-gfx-vs-wasi-gfx.md`
(the comparison) and `proposals/wasi-canvas/` (the draft). Goal: a single
standard-shaped story where guest-owned renderers (wasi-gfx) and
host-rendered canvas guests (Compose/Slint/dioxus) attach to the same
surface/windowing/event vocabulary — without forcing either side into the
other's control flow.*

## The four axes, and the reconciliation for each

### 1. Renderer ownership → canvas as a third graphics-context

wasi:surface's pivotal verb is `connect-graphics-context(ctx)`: a surface
is renderer-agnostic, and the context is pluggable — today webgpu-device
or frame-buffer. **Proposal: `canvas-context` becomes the third pluggable
context type.** Our `wasi:canvas/embedding` is already swapchain-shaped:

| wasi-gfx context | wasi:canvas/embedding today |
|---|---|
| acquire buffer / current texture | `begin-frame() -> canvas` |
| present | `end-frame()` |
| device/context handle | `get-graphics() -> graphics` |

An alignment pass (rename + field shapes, no semantic change) makes
`wasi:canvas` literally an implementor of the wasi-gfx graphics-context
contract. Then "which renderer" is a per-surface negotiation: a game
connects a webgpu context, a Compose app connects a canvas context, the
SURFACE LAYER NEITHER KNOWS NOR CARES. This is the single highest-leverage
change: it turns the two stacks from parallel universes into two backends
of one socket.

### 2. Window ownership → the Wayland answer (request vs configure)

wasi:surface 0.0.1 reads as "guest creates and sizes windows," which is
fatal for any compositor/window-server host (wandr's arbiter, but equally
any future multi-window wasi host). The precedent that resolves it is
xdg_shell: **clients request, compositors configure.**

Proposal: keep `surface(create-desc)` and `request-set-size`, but define
them as HINTS; the authoritative geometry always arrives via a
`configure`/resize event, and hosts may ignore requests entirely. Under
that reading wandr can implement wasi:surface honestly today:
`create` = register a window with the arbiter (sf_media child surface is
the existing primitive), `request-set-size` = advisory, configure events =
the arbiter's geometry pushes (exactly what `on-resize` carries now).
Roles/z-order stay host policy — same as every compositor.

### 3. Event delivery → one event vocabulary, two delivery profiles

The events themselves are already field-compatible (our handler records
were deliberately mirrored from wasi:surface's). Proposal: split the
PACKAGE from the DELIVERY:

- `wasi:input-events` (or folded into wasi:surface types): the records
  and enums, defined ONCE — pointer-event, key-event (W3C code + text),
  configure/resize, frame tick.
- **Pull profile** — wasi:surface pollables: for guests that own an event
  loop (Rust/C++ engines, command-style components).
- **Push profile** — exported handler interfaces (today's
  `wasi:input-handlers`): for host-driven reactor guests, which is the
  ONLY shape a Kotlin/Wasm component can physically consume (no blocking,
  no pollable-integrated coroutine executor; the app only runs inside
  export calls).

A guest declares its profile by what it exports/imports; a host can serve
both. Conversions are mechanical in both directions:

- A push-native host (wandr) offers the pull profile by queueing events
  per surface and exposing the queue via pollables — no architectural
  change, the standalone loop already drains per-frame.
- A pull-style API can be offered to guests ON TOP of push delivery by a
  guest-side adapter library: handler exports append to an internal
  queue; a non-blocking `poll_events()` drains it from inside `on-frame`.
  Code written winit-style runs unmodified as long as it tolerates
  "poll returns what's queued, ticks arrive via frames" — which is
  exactly winit's `about_to_wait` shape anyway. (This adapter is a
  ~200-line Rust crate; worth building as the proof.)

### 4. The platform remainder (IME, lifecycle, insets, theme)

Neither wasi-gfx nor wasi:surface covers IME, lifecycle, insets,
clipboard, or theme — and Compose-class apps need all of them. Proposal:
keep these as separate, optional interfaces layered NEXT TO the surface
(our my:skiko-gfx ime/lifecycle slices, eventually proposed as their own
small packages). Crucially, none of them belong INSIDE wasi:surface — a
game on a console host wants none of it. Optional-import layering keeps
the core surface universal.

## What this buys, concretely

- **A Compose app's world becomes expressible in standard terms**:
  `import wasi:surface` (read-geometry + configure) + `connect`ed
  `wasi:canvas` context + exported push-profile handlers + optional
  ime/lifecycle. Nothing about the app changes — only the names; the
  skiko binding maps 1:1. my:skiko-gfx shrinks to the not-yet-standard
  remainder and can eventually vanish.
- **A wasi-gfx app runs on wandr unchanged**: surface → sf_media child
  surface, frame-buffer/webgpu context → the task-93 present path,
  pull-profile events → host-side queue + pollables. The host gains a
  second guest class (engines that bring renderers) at marginal cost.
- **Wandr canvas guests become portable to any wasi-gfx host** that adds
  the (small, CPU-implementable) canvas-context: a desktop host can back
  it with Skia, tiny-skia, or even rasterize onto a frame-buffer —
  the same WIT contract regardless.
- **The standards pitch gets simpler**: not "here is a rival 2D stack"
  but two PRs against the wasi-gfx worldview — (a) canvas as a third
  graphics-context, (b) a push delivery profile for host-driven runtimes
  (precedent: wasi:http's incoming-handler is already push). Champions
  overlap; nothing is thrown away on either side.

## Suggested sequence (cheap → committal)

1. **Alignment pass** on `proposals/wasi-canvas/embedding` — reshape
   get-graphics/begin-frame/end-frame into a `canvas-context` resource
   matching wasi-gfx's context idiom. Additive; consumers re-generate
   bindings once (the wire_wasi_canvas!/slint seams localize it).
2. **Extract the shared event records** into one package consumed by both
   the handlers WIT and (later) pollable delivery. Pure refactor of
   proposals/wasi-input-handlers.
3. **Pull-profile spike on the host**: implement wasi:surface +
   frame-buffer for ONE Rust guest (a bouncing-square loop over
   pollables) on an sf_media child surface. Proves direction A end-to-end
   and produces the host's event-queue machinery.
4. **Guest-side adapter crate** (`surface-loop-over-handlers`): the
   winit-ish pull API over push delivery. Proves direction B and gives
   Rust guests a familiar API without giving up reactor hosting.
5. Only then: open the upstream conversation with all four artifacts in
   hand (working canvas-context, shared events, both profiles live on a
   real phone).

## DESIGNED (2026-06-12): proposals/wasi-surface/DESIGN-0.0.2.md

The socket model this document sketched is now a designed proposal —
`wasi:surface` + `wasi:graphics-context` 0.0.2 shapes
(wasm-tools-validated), the four producer connection idioms, the
fused-form equivalences (canvas embedding, video placement) with their
un-fuse lanes, overlap audit and the four-criteria acceptance check.
This document remains the reasoning record; the design doc is the
artifact.

## Upstream recheck (2026-06-12) — claims re-grounded against source

Upstream re-inventoried (it MOVED: surface/graphics-context/frame-buffer
now live in `wasi-gfx/wasi-gfx-runtime` `wit/deps/`, not in
WebAssembly/wasi-webgpu, which keeps only webgpu.wit). Three verdicts:

1. **canvas-context vs `wasi:graphics-context.context` — same idiom,
   one deliberate structural divergence.** Their context is
   renderer-agnostic: `get-current-buffer() -> abstract-buffer`, and the
   connected renderer API converts (`frame-buffer.buffer
   .from-graphics-buffer(abstract-buffer)`). Ours FUSES the conversion
   (`get-current-buffer() -> canvas`). Full alignment = the third-context
   shape (`canvas-device.connect-graphics-context` +
   `canvas.from-graphics-buffer`) — DEFERRED deliberately: upstream is
   visibly pre-stable (their `present` is marked "TODO: might want to
   remove", frame-event is an empty TODO record, and `context` has an
   AMBIENT constructor that violates WASI's own no-ambient-authority
   rule — our embedder-granted `get-context` is the cleaner capability
   story to converge TOWARD, not from).
2. **Event vocabulary — ours is strictly richer; convergence flows FROM
   wandr's records.** Upstream surface events: pointer = {x,y} ONLY (no
   multi-touch id, no buttons, no pressure, no scroll, no
   enter/leave!), key = a ~150-case enum of the same W3C code table our
   `code: string` carries, frame-event = empty. The
   wasi:input-handlers@0.0.2 records (six-consumer union) are the
   credible shared vocabulary for the push/pull split — the pitch
   strengthens: we bring the event model upstream lacks.
3. **The video claim, made precise.** "Signal's video path ≈ wasi-gfx"
   holds at the INFRASTRUCTURE level: the task-93 sf_media child
   SurfaceControl + BBQ producer is exactly what a wasi:surface
   implementation needs host-side (create-desc → child surface;
   request-set-size = advisory under the arbiter, matching the
   request/configure reading; pull-profile events = the queue+pollable
   work of step 3). It does NOT hold at the WIT level, and shouldn't:
   wandr:video is a codec contract. The clean observation the recheck
   adds: a video decoder is naturally ANOTHER graphics-context consumer
   (a fourth context type beside webgpu/frame-buffer/canvas — the
   "media element" of the socket), and wandr:video currently FUSES
   surface placement (set-rect/set-visible/set-rotation) into the
   decoder resource. Named decision: the fusion stays (shipped,
   live-call-verified; placement verbs are the arbiter-geometry
   shorthand) — the factoring lane (codec ↔ surface attachment) is
   recorded for IF wandr ever implements wasi:surface, at which point
   those three verbs would overlap with surface vocabulary and the
   decoder would instead connect to a child surface's context.

## Non-goals

- Porting Compose itself to webgpu/frame-buffer (gives up host Skia — the
  performance/memory architecture of the whole runtime).
- Letting guests author window geometry/roles (window-server invariant;
  resolved by the request/configure reading instead).
- Blocking poll loops in Kotlin guests (impossible: reactor model,
  single thread, coroutine pump rides export exits — see
  docs/kotlin-wasm-export-exit-pump-bug.md for how load-bearing that is).
