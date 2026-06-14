# egui on wandr — mapping memo

> Written 2026-06-12 (task 102, after the 0.0.2 redesign). Sources: egui
> integration docs (Mesh/TexturesDelta/RawInput, verified against
> docs.rs/egui this date), epaint README, the egui_glow reference painter.
> Companion to `proposals/wasi-canvas/REDESIGN-0.0.2.md` §1b/§7 — egui is
> the candidate FIFTH reference consumer, valuable precisely because it
> occupies an axis position the four reference libraries don't.

## Verdict

**Fits, via the mesh lane — one additive verb away.** egui is a mature,
pure-Rust, immediate-mode UI whose entire rendering output is *textured
triangle meshes*, not canvas shapes. It maps onto wasi:canvas through a
single promoted deferral (`draw-mesh`, the `draw-vertices` row of the
0.0.2 deferral table) plus a texture-update convenience that can be
deferred for a spike. Toolchain cost ≈ zero: the same
`wasm32-wasip2` + `wit_bindgen` path slint-wandr ships on. A spike is a
weekend-scale guest crate, not a campaign — the cheapest possible test of
a genuinely new consumer class.

## Why egui is a NEW axis position

| Axis | egui's position |
|---|---|
| Text shaping | guest — but NOT the Slint model: egui rasterizes glyphs into its own **font atlas texture** and emits textured quads. It imports neither `glyphs` nor `layout`. |
| Retained scene | none — pure immediate mode, full re-tessellation per frame (the reactor render-frame model fits natively; `frame-pacing` ← `needs_repaint`) |
| Rasterization | SPLIT: egui tessellates + anti-aliases (feathering) in-guest; the host only fills textured triangles. A position between "host rasterizes shapes" and "guest delivers pixels". |

No reference library exercises "mesh + atlas" — which is exactly why a
fifth consumer here stress-tests the contract somewhere new.

## Integration shape (from the canonical loop)

```
host events → RawInput → ctx.run(ui) → FullOutput
  ├─ TexturesDelta.set/free  → image resources (graphics factory)
  └─ tessellate(shapes)      → ClippedPrimitive[] → per primitive:
        save; clip-rect; draw-mesh(verts, indices, texture); restore
present (canvas-context)
```

## Mapping table

| egui output | wasi:canvas 0.0.2 home | Status |
|---|---|---|
| `Mesh { vertices(pos,uv,color), indices, texture_id }` | **`draw-mesh`** — the promoted `draw-vertices` deferral (additive, R2): `record vertex { pos: point, uv: point, color: color }`, `draw-mesh: func(canvas, vertices: list<vertex>, indices: list<u32>, texture: option<borrow<image>>)` → skia `drawVertices` + image shader | 🆕 the one promotion |
| `ClippedPrimitive.clip_rect` | `save` + `clip-rect` + `restore` per primitive | ✅ |
| `TexturesDelta.set` (full image; `ImageData::Color`/`Font`) | `graphics.image-from-rgba8` (font atlas is alpha-only → expand to RGBA guest-side) | ✅ |
| `TexturesDelta.set` with `pos` offset (PARTIAL atlas update) | image resources are immutable → spike re-uploads the whole texture per delta (the font atlas stabilizes after warm-up frames); proper lane: additive `image.write-region` method (R2) | ⚠ workable / named lane |
| `TexturesDelta.free` | `image` resource drop | ✅ |
| `PaintCallback` (custom GPU passes, 3D viewports) | out of scope for wasi:canvas — that's the wasi-gfx side of the surface socket | 🚫 named exclusion |
| `RawInput` (pointer/key/touch/modifiers) | embedder contract — wandr's pointer/key handlers map 1:1 | ✅ |
| repaint scheduling (`needs_repaint`) | `frame-pacing` | ✅ |
| IME / text events | basic insertion via key events; full IME = the wandr IME slice | ✅ baseline |

## Contract findings (fed back into REDESIGN-0.0.2 §7)

1. **`draw-mesh` passes R5 as a primitive**: not semantically derivable
   (no existing verb renders arbitrary textured triangles; emulating via
   per-triangle clip-path + image-pattern is absurd wire cost — fails
   test 3 by orders of magnitude). egui is the consumer that legitimately
   promotes the deferral the moment a spike ships.
2. **`image.write-region`** joins the deferral table with egui as its
   promoting consumer (font-atlas incremental rasterization).
3. Rendering correctness notes for the spike: epaint colors are
   **premultiplied** Color32 in gamma space (convert once guest-side);
   egui does its own anti-aliasing via tessellation feathering — the
   mesh paint must set `anti-alias = false` and leave feathering ON, or
   edges double-blur.

## Spike order (when wanted)

1. Bare guest crate (`apps/user/wandr.egui.test`): egui demo lib +
   RawInput wiring + whole-texture uploads + meshes drawn as **untextured
   fallback** (draw-path per triangle batch) just to prove the loop —
   ugly but zero contract changes.
2. Add `draw-mesh` to the draft (additive) + host impl (skia
   drawVertices) → real rendering, measure frame cost vs the 60 fps
   budget.
3. Only if atlas re-upload dominates: `image.write-region`.

## In-depth verification (2026-06-14) — is the path clean?

Re-checked the claim against the actual artifacts (contract WIT, host impl,
egui rendering core). **Clean = bounded, verified, no hidden blockers — but
NOT drop-in: one additive verb + its host impl + a perf spike.**

- **Toolchain: clean, zero risk.** Pure Rust → `wasm32-wasip2` +
  `wit_bindgen`, the identical shipped path of `crates/slint-wandr` and
  `crates/dioxus-canvas`. No WasmGC, no adapter fork, no corelib pins.
- **Supporting verbs: present AND host-implemented (verified).** Confirmed
  in `proposals/wasi-canvas/wit/canvas.wit` and live in
  `runtime/wandr-host/src/wasi_canvas_002_impl.rs` (skia_safe):
  `image-from-rgba8`, `clip-rect`, `draw-path` (untextured fallback),
  `image` + `image-pattern`/`sampling`. So egui's entire loop EXCEPT the
  mesh draw runs on today's contract+host.
- **The one gap: `draw-mesh`** — confirmed NOT in the WIT and NOT in the
  host. It's the documented R2 additive deferral (egui = the promoting
  consumer; shape pre-validated by wasm-tools; promotion specs
  index-validation → trap). The egui→skia mapping is clean:
  `drawVertices(Vertices::new_copy(TriangleList, pos, texs=uv, colors,
  indices), BlendMode::Modulate, paint{shader = image-pattern(atlas, local
  matrix = scale(texW,texH)), anti_alias=false})`. Two host details, both
  known/expressible: (a) skia `texs` are in SHADER space, not normalized →
  the atlas shader needs the scale-to-texel local matrix; (b) per-vertex
  color × texture = `BlendMode::Modulate`.
- **One spec wrinkle to pin at promotion (the only "not fully clean"
  point):** `draw-mesh(…, texture: option<image>)` bakes egui's
  modulate + linear-sampling + AA-off conventions into the host impl. Fine
  for egui, but it makes the verb egui-shaped; a second mesh consumer
  (Flutter `drawVertices` is noted to "ride" this promotion) might need a
  blend/sampling param. Resolve the verb's generality once, at promotion,
  to avoid a later breaking re-spec.
- **Idle cost now solved upstream:** the on-demand rendering shipped for
  avalonia-wandr (incremental + skip-present-when-idle, `MarkDrawn`) is the
  same mechanism egui's `needs_repaint`→frame-pacing wants, so an idle egui
  guest would be ~free. Only ACTIVE-frame vertex wire cost remains the
  measurement (immediate mode: busy screen ≈ 10⁴ verts ≈ ~200 KB/frame).
- **No hidden blockers:** the sole unmappable feature, `PaintCallback`
  (custom GPU / 3D), is a genuine named exclusion (wasi-gfx side), needed
  by no standard widget.

**Net:** the path is clean and free of surprises; the work is exactly
"add `draw-mesh` (additive) + host skia `drawVertices` + a spike to measure
per-frame vertex cost," with the verb-generality decision the one thing to
get right up front. Unchanged from the original verdict — now confirmed
against the real contract/host, not just analysis.

## Design Q&A (2026-06-14) — guest-side emulation + wire cost

**Can `draw-mesh` be emulated guest-side with existing verbs? No.** Two
routes, both rejected: (a) decompose to vector verbs — breaks on per-vertex
GOURAUD color/alpha (egui's AA = per-vertex alpha feathering; flat fills
can't interpolate color across a triangle → no AA), and textured triangles
(text) would need one clip+image+restore PER triangle (~10⁴/frame, absurd);
(b) guest software-rasterizes the whole frame → `image-from-rgba8` +
`draw-image` — fully expressible and correct, but uploads a ~15 MB full-frame
bitmap per repaint (vs ~200 KB of verts) and moves raster off the host GPU
to guest CPU. So `draw-mesh` (host `drawVertices`) is genuinely the only
clean way to get gouraud color + texture modulation + AA in one GPU call —
confirms the R5 "primitive, not derivable" finding against egui's real AA.

**Active-frame wire cost — Avalonia approach, does clean spec win?** Idle:
yes, identically — `needs_repaint`→frame-pacing→skip frame = zero (the
on-demand mechanism shipped for avalonia-wandr). Active: Avalonia's win is
RETAINED+dirty-tracked deltas; egui is IMMEDIATE (full re-tessellation, no
deltas) so that win does NOT transfer — and need not, because the boundary
is **in-process**: a `draw-mesh` call hands the host a ptr+len into guest
linear memory (bulk memcpy, not IPC). ~200 KB/repaint = microseconds. Clean
spec wins: (1) flat/parallel buffers (`positions/uvs: list<point>`,
`colors`/`indices: list<u32>`) matching skia `SkVertices::MakeCopy`'s
separate arrays → host near-zero-copy + bulk-memcpy lowering (egui
de-interleaves guest-side, one cheap pass); (2) frame-pacing for idle. A
retained `mesh` resource (true Avalonia retention) is a poor fit (egui has
no stable per-frame mesh identity) — skip. Net: the wire is a cheap memcpy;
the only real per-frame cost is host GPU rasterization, identical for any
consumer. The "wire cost = main risk" framing softens to a non-issue.

## Contract-cleanliness reconsideration (2026-06-14) — draw-mesh vs wasi-gfx

Pressed on "is the interface clean / standard / non-duplicating?", the
draw-mesh-in-wasi:canvas plan does NOT pass the *standard* bar, and the
recommendation flips:

- draw-mesh is non-duplicating ✓ and reusable ✓, BUT it has **no W3C
  Canvas2D analog** (Canvas2D has no `drawVertices`); it's a skia extension
  (`SkCanvas::drawVertices`). wasi:canvas is positioned as the **Canvas2D**
  companion to wasi-gfx (the web's Canvas2D-vs-WebGPU split), so a
  gouraud-textured-triangle verb is GPU-layer scope creep into the 2D layer.
- egui is fundamentally a **GPU mesh renderer** — its real backend is
  `egui_wgpu` (egui → wgpu → WebGPU), and **wasi-gfx/wasi-webgpu** exists to
  run exactly that as a component. So the clean, WASI-standard placement is
  **egui → wasi-gfx, NOT draw-mesh in wasi:canvas.** egui then *validates*
  the wasi-canvas ↔ wasi-gfx boundary (lands on the GPU side) instead of
  stretching the 2D contract. Keeps both layers clean + non-overlapping.
- **Recommendation (priority = clean/standard, not effort): do NOT add
  draw-mesh.** Keep wasi:canvas pure Canvas2D (Compose/Slint/Avalonia/
  Flutter-dart:ui/SwiftUI). egui waits on wandr wiring wasi-gfx (Phase 2,
  unreleased — bigger than one verb, but the boundary-correct path).
- Pragmatic counter, recorded but not chosen: skia (a 2D lib) does carry
  drawVertices, and egui's core needs only textured-triangle *fill*, not the
  full WebGPU pipeline — so draw-mesh is one verb vs all of wasi-webgpu. A
  defensible INTERIM iff egui is urgent and wasi-gfx absent, but explicitly a
  skia-extension lane that migrates to wasi-gfx. Earlier sections (mapping
  table, "one additive verb") describe THIS interim; the standard answer
  above supersedes them when cleanliness is the priority.
- SwiftUI/OpenSwiftUI checked at the same time: a standard Canvas2D consumer
  (RenderBox shapes/text/images) — needs **no** canvas-WIT changes; its GPU
  effects (Shader/MeshGradient) are wasi-gfx, like egui's PaintCallback.

## wasi-webgpu forward-compat check (2026-06-14) — no function to pre-adopt

Asked whether wasi-webgpu exposes a function we could mirror NOW (same
name/flow/path) in wasi:canvas, so that when the spec lands we just "plug in"
wasi-webgpu. Verified against the real WIT (`WebAssembly/wasi-gfx`, WebGPU
half now at the `wasi-webgpu` repo: `wit/webgpu.wit`, 1093 lines, +
`imports.wit`). **Answer: no — there is no single function to mirror.**

- wasi:webgpu@0.0.1 is a **faithful, mechanical mirror of the entire W3C
  WebGPU API** — `gpu`, `gpu-adapter`, `gpu-device`, `gpu-buffer`,
  `gpu-bind-group`, `gpu-pipeline-layout`, `gpu-render-pipeline`,
  `gpu-command-encoder`, `gpu-render-pass-encoder`, `gpu-texture`,
  `gpu-queue`. There is **no high-level "draw a mesh" verb**. The draw entry
  points are `draw` / `draw-indexed` on `gpu-render-pass-encoder`, reached
  only through the full pipeline dance: `create-render-pipeline` →
  `begin-render-pass` → `set-pipeline` → `set-vertex-buffer` →
  `set-bind-group` → `draw` → `end` → `queue.submit`. Presentation mirrors
  the web swapchain exactly: `gpu-canvas-context.configure()` +
  `get-current-texture()` per frame.
- The CPU-side `wasi:surface` / `wasi:frame-buffer` interfaces the governance
  blog mentioned are **not in the repo yet** — only the WebGPU interface is
  published.
- So the unit of adoption is the **whole device/pipeline/render-pass object
  graph**, not one function. Pre-declaring `draw`/`draw-indexed` in
  wasi:canvas would mean dragging in pipelines, shader modules, bind groups,
  vertex buffers and a swapchain — i.e. re-implementing WebGPU inside the
  Canvas2D interface. That is exactly the duplication + scope-creep the
  clean/standard priority rules out. There is nothing to alias at the verb
  level.

**The clean forward-compatible plug lives at the STACK level, not the verb
level.** egui renders through `egui_wgpu` → **wgpu**, and wgpu already has a
wasi-webgpu backend. So an egui guest's clean target is just "wgpu," and the
"plug when the spec arrives" is: **wandr implements the `wasi:webgpu` host
interfaces** (skia/EGL already provide the GPU device underneath). The day
that's wired, egui-on-wgpu-on-wasi-webgpu runs with **zero changes to egui,
zero changes to wasi:canvas, and no speculative verb invented today** — you
adopt the real standard interface verbatim rather than carrying a hand-rolled
lookalike you'd later have to reconcile/deprecate. This strictly confirms the
prior section's reversal: keep wasi:canvas pure Canvas2D, do NOT add
draw-mesh, treat wasi:webgpu as a future second rendering lane implemented
**wholesale (host-side)** when the spec lands. SwiftUI/Avalonia/Slint/
Flutter/Compose stay on wasi:canvas and need no WebGPU.

## Footprint / risks

- Guest size: MB-scale (pure Rust, no runtime) — the lightest candidate
  in the lineup after the chrome guests.
- Perf risk: vertex lists cross the wire every frame (immediate mode).
  A busy egui screen is ~10⁴ vertices ≈ 200 KB/frame of lowering — the
  spike's main measurement. Mitigations if needed: egui's own
  per-widget caching, or a retained `mesh` resource (R2 lane).
- API stability: egui moves fast (frequent minor releases, integration
  API occasionally shifts) — pin exactly, same policy as `i-slint-*`.
