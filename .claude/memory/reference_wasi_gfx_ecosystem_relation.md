---
name: reference_wasi_gfx_ecosystem_relation
description: "How wandr's graphics/video WIT proposals relate to the upstream wasi-gfx org + wasi:webgpu — graphics-context is the shared socket; wandr ADDS consumers, does NOT own surface"
metadata: 
  node_type: memory
  type: reference
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-05T06:29:06.677Z
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
- **`wasi:video-decoder`** / **`wasi:video-encoder`** — the "media element": a HW codec
  fills (decoder) / captures-and-encodes (encoder) into the connected surface's
  buffers, pixels never crossing the sandbox. These are the factored halves of the
  production-proven `wandr:video@0.1.0` (jellyfin player + Signal calls).

So the socket's producers are: **surface** (compositor-fill), **webgpu** (GPU-fill),
**frame-buffer** (CPU-pixel-fill) — upstream; **canvas** (Canvas2D), **video-decoder**
(host-codec-fill), **video-encoder/camera** (capture-fill) — wandr additions.

## Practical implications for the extraction (task 120 follow-on)
- wandr's video factoring is "add two consumers to wasi-gfx's socket," not "invent a
  stack." Adoption trigger = the same as wasi:surface wiring (R3 coexistence: the fused
  `wandr:video` keeps shipping until then).
- Camera source is a future `wasi:camera` (the encoder's `facing`/`source-camera` +
  `display-rotation` factor out there).
- Related: [[reference_wasi_webgpu_gfx]] (canvas vs webgpu), [[project_wandr_video_host]],
  [[project_wandr_call_video_track]].
