---
name: reference_wasi_gfx_ecosystem_relation
description: "How wandr's graphics/video WIT relate to upstream wasi-gfx. UPDATED 2026-08: graphics-context DEPRECATED → per-pairing surface-* worlds @ wasi-gfx:surface@0.2.0. SETTLED MODEL: every producer gets 2 forms — reactor `embedding` (SHIPS, wasi-gfx-FREE, host-assigned surface) + optional `surface-*` pairing (portability, imports wasi-gfx:surface); video-codec/camera must ADD the missing reactor twin; placement verbs live there; drop graphics-context@0.0.2."
metadata: 
  node_type: memory
  type: reference
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-06T05:38:44.922Z
---

wandr ADDS graphics/media producers on top of the upstream wasi-gfx surface stack;
it does NOT own surface. ‼️ The old framing "`wasi:graphics-context` is THE shared
swapchain socket" is OBSOLETE as of 2026-08 — see the status re-check below.

## ‼️ UPSTREAM STATUS RE-CHECK (2026-08-05) — the design moved
Sources: wasi-gfx.dev "Future of wasi-gfx" blog, WebAssembly/wasi-webgpu issue #55
(OPEN), github.com/wasi-gfx/wasi-gfx (`packages/surface/`), wasi-gfx-runtime.

- **`wasi:graphics-context` is DEPRECATED / removed from the design.** Per issue #55
  (Luke Wagner's proposal, now implemented in WIT): the ONE generic graphics-context
  "socket" is replaced by **per-pairing CONNECTION WORLDS** — one WIT per (windowing
  system × graphics API): `surface-webgpu`, `surface-frame-buffer` exist today; the
  pattern generalizes. Each pairing = a `context` resource with
  **`constructor(surface: borrow<surface>)` + `get-current-buffer()` + `present()`**.
  Lets each API carry its own params and avoids locking swapchain/present into a
  shared type. Issue #55 still OPEN → not fully frozen.
- **Namespace moved: `wasi:surface`/`wasi:frame-buffer` → `wasi-gfx:surface@0.2.0` /
  `wasi-gfx:frame-buffer@0.2.0`** (out of core `wasi:`, into the `wasi-gfx:`
  namespace). Phase 2, shipping as **preview**; runtime = `wasi-gfx-runtime`. Base
  surface is now pure I/O + **async STREAMS**: `constructor(create-desc{width?,
  height?})` + `on-resize`/`on-frame`/`on-pointer-{up,down,move}`/`on-key-{up,down}`
  → `stream<_>`. Still in flux (a `frame-event` field is literally marked
  `/// TODO: doesn't mean anything`). The stream model aligns with wandr's wasip3
  async ([[project_task115_wasip3_async]]).
- **Only `wasi:webgpu` stays in the official `wasi:` namespace** (`wasi-webgpu@
  0.3.0-rc.2`) — it hit W3C Candidate Recommendation, the stable industry standard.
  surface/frame-buffer explicitly are NOT trying to be rigid standards.

## Impact on wandr's proposals — THREE points

**1. The old "graphics-context = shared socket" model is dead; align to
`wasi-gfx:surface@0.2.0` per-pairing worlds.** Our in-tree `wasi:surface` sketch
(`proposals/wasi-surface/`) and DESIGN.md "graphics-context socket / ownership-axes"
framing are obsolete. Reframe as CONSUMING/aligning to `wasi-gfx:surface@0.2.0`
(namespace `wasi-gfx:`, not `wasi:`), with decode/draw producers as `surface-*`
pairing worlds rather than graphics-context `connect(ctx)`.

**2. Our WIRED canvas is UNAFFECTED — and already matches the new idiom.**
`wasi:canvas` has TWO handoffs: the FUSED **`embedding.canvas-context`** (`get-context`
→ `graphics()`/`get-current-buffer()`/`present()`) is what actually SHIPS (host
`wasi_canvas_impl.rs`, slint-wandr, media-engine) — it is a self-contained host-driven
reactor handoff that does **NOT import `wasi:graphics-context`**, so the upstream
deprecation does not touch it; canvas keeps working as-is. And it maps 1:1 onto
`get-current-buffer`/`present` — i.e. it is STRUCTURALLY the new upstream
`surface-frame-buffer` context. We designed toward the pattern upstream just
standardized on. (Only the UNWIRED `connection.wit` — see point 3 — carries the stale
dep.)

**3. ‼️ CORRECTED 2026-08-06 (the "uniform two-form" below was REFUTED).**
Source-grounded verification (3 agents) found the two-form template does NOT apply
uniformly: it holds ONLY for **guest-pump** producers (canvas), NOT for **host-fill**
producers (video/camera). Upstream `wasi-gfx:surface@0.2.0` is guest-fills-only, so a
`surface-*` pairing for host-fill video/camera would be a wandr extension MISLABELED as
"wasi-gfx-aligned" — do NOT build it. The correct model for video/camera is the
IMPORT-LAYERING one: extract codec BASICS → `wasi:video-codec` (no surface), and
`wandr:video` (the EMBEDDER) imports it + keeps host-fill decode-to-surface as a wandr
stack decision. See **[[project_video_wit_extraction_embedder]]** for the full corrected
plan + next-session tasks. The text below is kept for history but is superseded for
video/camera.

--- (superseded for video/camera; still valid for canvas) ---
Every wandr graphics/media producer gets **canvas's two-form template**, uniformly:

- **Reactor `embedding` form = the SHIPPING form, wasi-gfx-FREE.** Host assigns the
  surface; the guest gets its context (canvas `get-context()`) and never creates a
  surface — because wandr is a MANAGED embedder (arbiter owns roles/geometry/stacking),
  unlike upstream's guest-CREATED surfaces (there is NO upstream analog, so this form
  stays regardless of upstream). Self-contained: imports NO wasi-gfx; placement is plain
  records. Canvas already ships exactly this (`embedding.wit`, no graphics dep).
- **`surface-*` pairing form = OPTIONAL portability lane, imports `wasi-gfx:surface@0.2.0`.**
  `context::constructor(borrow<surface>)` for a guest targeting a GENERIC wasi-gfx host.
  Kept OUT of the wandr shipping world (a separate side-world the wandr host does NOT
  load) → costs nothing at runtime, chases no 0.2.0-preview moving target, yet preserves
  the "runs on any wasi-gfx host / upstreamable" identity. Dropping it entirely would
  make wandr a PRIVATE graphics stack — rejected.

| producer | reactor `embedding` (ships, no dep) | `surface-*` pairing (optional, wasi-gfx dep) |
|---|---|---|
| canvas | `embedding.canvas-context` ✅ exists | `connection.wit` → `surface-canvas` |
| video-codec | ‼️ MISSING — must ADD (the twin production `wandr:video` already IS) | `connect(ctx)` → `surface-video-codec` |
| camera | ‼️ MISSING — must ADD | `connect-preview(ctx)` → `surface-camera` |

- **The asymmetry to fix:** the factored `wasi:video-codec`/`wasi:camera` kept ONLY the
  explicit-surface form and DROPPED the reactor twin — even though the reactor form is
  what production `wandr:video` ships (host-assigned CHILD surface + `set-rect`/`z-layer`,
  host composites). The video reactor form is even LIGHTER than canvas's (host FILLS via
  the decoder; the guest controls only WHEN via `decoded-frame.present(at-ns)` and WHERE
  via a placement request — no get-current-buffer/present bracket).
- **Placement verbs are NOT "unnecessary surface".** `set-rect`/`set-visible`/`z-layer`
  belong in the reactor form as host-HONORED REQUESTS ("guest requests, embedder
  configures" — the z-layer ownership note), distinct from guest surface-OWNERSHIP. The
  necessary-and-sufficient audit that deleted them was right for the PAIRING form, wrong
  as the ONLY form. Restore them in the reactor `embedding` form.
- **DROP the deprecated `wasi:graphics-context@0.0.2`** vendored dep + every `connect(ctx)`
  built on it; the reactor forms need nothing external. (`wasi:eme` for DRM is our OWN
  draft, not wasi-gfx — unaffected.)

Net: everything the wandr host RUNS is wasi-gfx-free; wasi-gfx lives only in optional
side-worlds. ORTHOGONAL to the production `wandr:video` cleanup ("Scope A"), which has no
graphics-context dep. Building the real surface foundation ("Scope C") chases a
0.2.0-preview target with an OPEN design question — keep the fused `wandr:video` shipping
(R3 coexistence) rather than rush it.

## wandr's ADDITIONS (producers on the upstream surface stack)
- **`wasi:canvas`** — Canvas2D producer (COMPLEMENTS webgpu, [[reference_wasi_webgpu_gfx]]).
- **`wasi:video-codec`** — ONE package (decoder + encoder + shared `frame`/VideoFrame),
  parallel to `wasi:audio-codec` + WebCodecs. Decoder fills the connected surface
  host-side (decode-to-surface); encoder = pure codec (consumes a `frame`). Pixels never
  cross. Factored form of production `wandr:video@0.1.0` (jellyfin + Signal). Superseded
  the split video-decoder/-encoder/-frame drafts (consolidated 2026-08-05).
- **`wasi:camera`** — the SOURCE, separate from the codec (W3C precedent: capture ≠
  codec). Mirrors W3C Media Capture and Streams (`getUserMedia`/`MediaStreamTrack`;
  `facingMode`==`facing`) + Image Capture (torch/zoom) + MediaStreamTrack Insertable
  Streams (the raw-frame bridge). `getUserMedia → track-processor → VideoEncoder` ==
  `wasi:camera → wasi:video-codec`. Greenfield (no upstream wasi:camera). Opaque
  host-held `frame` (shared, lives in video-codec; zero-copy + `read-rgba` opt-out),
  `list-cameras`/`open(facing)`, viewfinder preview producer, torch/zoom,
  `frame.rotation` (source CVO).

Producers: **surface** (compositor), **webgpu** (GPU), **frame-buffer** (CPU pixels) —
upstream; **canvas**, **video-codec** decoder, **camera** viewfinder — wandr. Each wandr
producer ships a reactor `embedding` form (wasi-gfx-free) AND an optional `surface-*`
pairing (imports `wasi-gfx:surface@0.2.0`) — see the settled model in point 3.

Related: [[reference_wasi_webgpu_gfx]] · [[project_wandr_video_host]] ·
[[project_wandr_call_video_track]] · [[feedback_wit_changes_need_approval]] ·
[[project_task115_wasip3_async]].
