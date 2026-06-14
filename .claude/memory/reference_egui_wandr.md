---
name: reference_egui_wandr
description: "egui on wandr: FINAL = belongs on wasi-gfx/wasi-webgpu, NOT wasi:canvas (draw-mesh is a skia-ism w/ no Canvas2D analog → scope creep). egui→egui_wgpu→wgpu→wasi-webgpu; the clean forward-compat plug = wandr implements wasi:webgpu HOST iface (no egui/canvas changes, no verb to pre-adopt — wasi-webgpu is the whole WebGPU API, no high-level draw-mesh fn). memo = docs/egui-wandr-feasibility.md"
metadata: 
  node_type: memory
  type: reference
  originSessionId: a79141a4-0e71-4555-ac42-9babd863d1e8
---

egui maps onto wasi:canvas through `draw-mesh` (vertex{pos,uv,color} +
indices + option<borrow<image>> → skia drawVertices) — its ENTIRE output
is textured triangle meshes + a TexturesDelta-managed atlas. Spike =
weekend-scale (wasm32-wasip2, slint-wandr toolchain). Gotchas for the
spike: epaint colors are PREMULTIPLIED gamma-space; mesh paint must set
anti-alias=false (egui feathers in tessellation — double-AA otherwise);
partial atlas updates → re-upload whole texture until `image.write-region`
(R2 lane). PaintCallback (custom GPU) = wasi-gfx side, out of scope.
Perf watch item: ~10⁴ vertices/frame ≈ 200 KB/frame lowering.
Memo: docs/egui-wandr-feasibility.md; deferral rows in
proposals/wasi-canvas/REDESIGN-0.0.2.md §7.

FINAL PLACEMENT (2026-06-14, supersedes the draw-mesh-in-canvas plan when
clean/standard is the priority): do NOT add draw-mesh to wasi:canvas — it has
NO W3C Canvas2D analog (it's an SkCanvas::drawVertices skia-ism) → GPU-layer
scope creep into the Canvas2D contract. egui's real backend is egui_wgpu →
wgpu, and wgpu already has a wasi-webgpu backend, so egui's clean home is
wasi-gfx/wasi-webgpu, not wasi:canvas. Verified wasi-webgpu WIT
(WebAssembly/wasi-gfx, webgpu.wit 1093 lines): it's a FAITHFUL MIRROR OF THE
WHOLE W3C WebGPU API (gpu/adapter/device/buffer/bind-group/pipeline/
command-encoder/render-pass-encoder/texture/queue) — there is NO single
high-level "draw-mesh" function to pre-adopt; draw entry = draw/draw-indexed
on render-pass-encoder via the full pipeline dance; present =
gpu-canvas-context.configure()+get-current-texture(). So there's NOTHING to
mirror at the verb level. The clean forward-compat "plug" is at the STACK
level: wandr implements the wasi:webgpu HOST interfaces (skia/EGL give the GPU
device) → egui-on-wgpu-on-wasi-webgpu runs with ZERO egui changes, ZERO
wasi:canvas changes, no speculative verb. Treat wasi:webgpu as a future
SECOND rendering lane implemented wholesale host-side. SwiftUI/Avalonia/
Slint/Flutter/Compose stay on wasi:canvas (pure Canvas2D), need no WebGPU.

Related: [[reference_slint_wasip2]], [[project_wasi_canvas_migration]],
[[reference_wasi_webgpu_gfx]], [[reference_avalonia_wandr]].
