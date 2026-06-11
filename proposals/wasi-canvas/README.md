# `wasi:canvas` — draft proposal (wandr, 2026-06-11)

A host-rendered 2D vector-canvas + text contract for WebAssembly
components: the **2D companion layer to wasi:webgpu/wasi-gfx** (the split
the web ships as Canvas2D vs WebGPU). Status: **wandr-local draft**, not
submitted anywhere, not wired into any build. `wasm-tools component wit
wit/` validates.

## Why this exists

WASI has a proposal for the GPU-driver layer (wasi:webgpu, guest-owned
renderer) and nothing for the 2D/UI layer. wandr ships that layer in
production as `my:skiko-gfx` — same guest binary rendering on an Android
phone (GPU) and a Linux desktop (CPU), with four UI frameworks' drawing
needs mapped empirically (`docs/skia-wit-mapping.md`). This draft is that
contract de-Skia'd, resource-ified and de-warted. Full reasoning:
`docs/skiko-gfx-vs-wasi-gfx.md`; break-analysis against wandr's shipped
model: `COMPATIBILITY.md` (verdict: coexistence, no hard breaks).

## Design rules

1. **Drawing only.** How a guest obtains a `canvas` and when it draws is
   the EMBEDDER's contract (wandr: host-driven `render-frame`; a wasi-gfx
   runtime: `wasi:surface` + graphics-context; a CLI tool: one-shot
   rasterize). No windowing, no input, no event loop — the red line that
   keeps both reactor-model and poll-loop runtimes hostable.
2. **Host-owned renderer, semantics over the wire.** Guests send
   device-independent draw commands; hosts rasterize GPU or CPU,
   unobservably. Heavy objects (image/shader/typeface/picture) are
   resources; per-draw styling (`paint`) is a value record carrying
   resource borrows (validated canonical-ABI-legal).
3. **Spec by reference to existing standards** where possible: SVG path
   grammar for paths, CSS compositing for blend modes, CSS border-radius
   corner order, W3C font-weight numbers.
4. **Two text layers, both real-world-proven:** `glyphs` (guest shapes —
   Slint/parley, Avalonia/HarfBuzz) is the portable floor; `layout`
   (host shapes — Compose-class managed runtimes that can't ship a
   shaper) is optional for embedders. This split is the draft's main
   novel claim, learned by shipping both.

## Deliberately excluded (and why)

- **Windowing/input/frame timing** — embedder contract (rule 1).
- **wandr's `WasiDrawable` / live-transform layers** — Compose-specific
  retained machinery; stays on `my:skiko-gfx`.
- **Pixel readback of the main canvas** — keeps the host's compositing
  private; offscreen canvases have `snapshot` instead.
- **Variable-font axes, path effects (dash), image filters on layers,
  runtime shaders (SkSL-class)** — real but deferred; additive later.
- **Damage/dirty rects** — an embedder/performance concern, not contract.

## Relationship to wandr's shipping interface

`my:skiko-gfx` keeps shipping unchanged (Compose/dioxus/Slint/chrome
consume it; the hand-written Kotlin binding stays). If/when this draft
gets a host implementation, it's a second `add_to_linker` over the same
`SkiaRenderer` — both packages served simultaneously; consumers migrate
per-consumer or never. First implementation candidate: a `slint-wandr`
variant, since its renderer already speaks the same shapes.

## Files

- `wit/canvas.wit` — `types` (paint/shader/image/geometry) + `draw`
  (`canvas` = the target, `graphics` = the embedder-granted creation
  factory: gradients/images/offscreen/recording).
- `wit/text.wit` — `glyphs` (typeface-from-bytes + positioned glyph runs)
  + optional `layout` (paragraph builder/metrics/hit-testing, clean
  `list<record>` returns).
- `COMPATIBILITY.md` — the hard-break analysis that gated this draft.

## Next steps (unscheduled)

1. Host implementation behind a feature flag (maps ~1:1 onto the existing
   `canvas_impl.rs` internals).
2. Port one renderer (slint-wandr) as the proving consumer; measure the
   resource-handle overhead vs u32 ids.
3. Let the surface harden; then consider the WASI phase-0 conversation,
   positioned as wasi-gfx's 2D companion (champion overlap natural).
