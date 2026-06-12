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

## Footprint / risks

- Guest size: MB-scale (pure Rust, no runtime) — the lightest candidate
  in the lineup after the chrome guests.
- Perf risk: vertex lists cross the wire every frame (immediate mode).
  A busy egui screen is ~10⁴ vertices ≈ 200 KB/frame of lowering — the
  spike's main measurement. Mitigations if needed: egui's own
  per-widget caching, or a retained `mesh` resource (R2 lane).
- API stability: egui moves fast (frequent minor releases, integration
  API occasionally shifts) — pin exactly, same policy as `i-slint-*`.
