---
name: reference_egui_wandr
description: "egui on wandr: FITS via the mesh lane — one additive verb (draw-mesh, the promoted draw-vertices deferral) + whole-atlas re-upload; candidate FIFTH reference consumer (new axis position: guest tessellates+feathers, host fills textured triangles); memo = docs/egui-wandr-feasibility.md"
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

Related: [[reference_slint_wasip2]], [[project_wasi_canvas_migration]].
