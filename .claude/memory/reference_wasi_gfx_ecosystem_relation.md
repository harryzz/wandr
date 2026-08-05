---
name: reference_wasi_gfx_ecosystem_relation
description: "How wandr's graphics/video WIT proposals relate to the upstream wasi-gfx org + wasi:webgpu — graphics-context is the shared socket; wandr ADDS consumers, does NOT own surface"
metadata: 
  node_type: memory
  type: reference
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-05T07:49:02.227Z
---

The unifying abstraction across all of wandr's graphics/media WIT is
**`wasi:graphics-context`** — the "swapchain socket": a surface exposes ONE context
(`get-context`), and a producer `connect`s to it and fills its buffers. Consumers
"differ only in who fills the buffer" (the ownership-axes model,
`contracts/proposals/wasi-surface/DESIGN.md`).

## Upstream (NOT wandr-owned)
- **wasi-gfx org** (https://github.com/wasi-gfx/wasi-gfx) owns **`surface`** (windows +
  input: pointer/keyboard/resize/frame) and **`frame-buffer`** (raw CPU pixel access).
  Both were originally in core WASI, then moved to this namespace to iterate
  "versioned like a library rather than a rigid standard." **`wasi:graphics-context`**
  lives here too as the shared socket.
- **`wasi:webgpu`** (https://github.com/WebAssembly/wasi-webgpu) is an official WASI
  spec, **Phase 2**, champions Mendy Berger + Sean Isom, portability Linux/Windows/
  macOS/Android/Web. It maps to the stable WebGPU web standard and is GPU-COMPUTE-
  focused — it **explicitly EXCLUDES windowing/display**, deferring that to wasi-gfx.
- ‼️ **`wasi:surface` is UPSTREAM, not wandr's.** The in-tree
  `proposals/wasi-surface/wit-sketch/surface.wit` is a **wandr PROPOSED CHANGE-SET as a
  validatable sketch** (capability-granted context via `get-context`, request/configure
  geometry, pull-profile events), claiming no version lineage. Ship-before-upstream
  fallback = `wandr:surface@0.0.1`. Any surface edit is a PROPOSAL to wasi-gfx, framed
  as such — never an "owned" change.

## wandr's ADDITIONS (new graphics-context consumers of the same socket)
- **`wasi:canvas`** — a Canvas2D producer (`[[reference_wasi_webgpu_gfx]]` notes it
  COMPLEMENTS webgpu, not competes).
- **`wasi:video-codec`** — the "media element": ONE package (decoder + encoder +
  the shared `frame`/VideoFrame), parallel to `wasi:audio-codec` and to WebCodecs.
  Decoder fills the connected surface host-side (decode-to-surface); encoder is pure
  codec (consumes a `frame`). Pixels never cross the sandbox. The factored form of the
  production-proven `wandr:video@0.1.0` (jellyfin player + Signal calls). (Superseded
  the earlier split `wasi:video-decoder`/`-encoder`/`-frame` drafts — consolidated
  2026-08-05.)

So the socket's producers are: **surface** (compositor-fill), **webgpu** (GPU-fill),
**frame-buffer** (CPU-pixel-fill) — upstream; **canvas** (Canvas2D), **video-codec**
decoder (host-codec-fill), **camera** viewfinder (capture-fill) — wandr additions.

## Practical implications for the extraction (task 120 follow-on)
- wandr's video factoring is "add two consumers to wasi-gfx's socket," not "invent a
  stack." Adoption trigger = the same as wasi:surface wiring (R3 coexistence: the fused
  `wandr:video` keeps shipping until then).
- **`wasi:camera` DRAFTED** (`contracts/proposals/wasi-camera/`, 2026-08-05) — the
  camera SOURCE, separate from the CODEC (`wasi:video-codec`). **W3C PRECEDENT:** the
  web platform separates capture from codec — `wasi:video-codec` mirrors WebCodecs
  (VideoDecoder/Encoder + VideoFrame in one spec), `wasi:camera` mirrors the W3C
  WebRTC-WG **Media Capture and Streams** (`getUserMedia`/`MediaStreamTrack`;
  `facingMode` == our `facing`), with **Image Capture** (torch/zoom/focus stills),
  **Screen Capture** (`getDisplayMedia` == a future screen-share source lane), and
  **MediaStreamTrack Insertable Streams** (`MediaStreamTrackProcessor` == the raw-frame
  bridge). So `getUserMedia → track-processor → VideoEncoder` is exactly
  `wasi:camera → wasi:video-codec`. Greenfield (no upstream wasi:camera); WebCodecs is
  tracked near wasi-webgpu, Media/Image/Screen Capture are WebRTC-WG. Camera shape:
  opaque host-held `frame` (shared, lives in `wasi:video-codec`; zero-copy + `read-rgba`
  opt-out), `list-cameras`/`open(facing)`, viewfinder `connect-preview(ctx)` = the fifth
  graphics-context producer, torch/zoom, `frame.rotation` (the encoder's old
  display-rotation, now a source property). The encoder was factored to a pure codec
  inside `wasi:video-codec` (`encode(frame)`; no camera/surface).
- Related: [[reference_wasi_webgpu_gfx]] (canvas vs webgpu), [[project_wandr_video_host]],
  [[project_wandr_call_video_track]].
