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

## Next steps

1. ~~Host implementation behind a feature flag~~ **DONE 2026-06-11**:
   `wandr-host --features wasi-canvas` implements `types` + `draw` +
   `glyphs` (the `canvas-host` bindgen world) over the same SkiaRenderer —
   `runtime/wandr-host/src/wasi_canvas_impl.rs`, linked at both
   instantiate sites, unit-tested on the offscreen path. `layout` host
   support landed 2026-06-11 with the dioxus-canvas migration (stage 1):
   skparagraph-backed paragraph/paragraph-builder resources reusing the
   renderer's font collection + family-alias typeface resolution; the
   `wasi-canvas` cargo feature is now DEFAULT-ON (additive linker
   entries). Notes: bindgen needs
   `imports: { default: trappable }`; skia Surface/PictureRecorder need
   an `unsafe impl Send` on the canvas resource (single-threaded store,
   the video.rs justification).
2. ~~Port one renderer (slint-wandr) as the proving consumer~~ **DONE
   2026-06-11**: slint-wandr now draws EXCLUSIVELY through wasi:canvas
   (my:skiko-gfx keeps only window-metrics + IME; the guest imports
   `wasi:canvas/{types,draw,glyphs,embedding}`). The embedding hook
   became the non-normative `embedding` interface (get-graphics +
   begin-frame→canvas / end-frame), exercised per frame. The port
   UPGRADED fidelity: per-corner radii (max-corner approximation
   retired), real fill rules, blur-on-paint shadows, owned shader/image/
   typeface resources that drop themselves. Verified live on the desktop
   host (`--features wasi-canvas`, CPU raster) AND on the Pixel 2 XL
   under --no-art (GPU GL path, AOT cwasm, host serving BOTH canvas
   packages at once) — same wasm binary in both places. Frame times
   same order as the u32-id port. The 'grey controls' delta root-caused +
   fixed: the OLD pipeline REPLACED color alpha with paint alpha
   (rendering fluent's semi-transparent light fills as opaque white);
   the draft pipeline multiplies faithfully, which exposed that Slint
   was drawing the LIGHT palette over wandr's dark chrome. Fix =
   slint-wandr wires the host theme (my:skiko-gfx/theme.get-night-mode)
   into SlintContext::set_color_scheme → fluent dark — i.e. the proving
   consumer caught a real fidelity bug in the legacy pipeline, working
   as intended.
3. ~~Migrate the production guests~~ **DONE 2026-06-11** — every
   non-Kotlin guest now draws through the draft:
   - **dioxus-canvas** grew a second backend (`launch_wasi_canvas!`,
     `wire_wasi_canvas!`): CanvasSink over wasi:canvas + layout-built
     paragraphs; wandr.dioxus.demo, taskmanager, connectivity.test and
     **Signal** all migrated + device-verified (history intact).
   - **System chrome** (launcher / statusbar / taskbar / keyguard —
     the hand-rolled `wit_bindgen::generate!` guests) ported off the
     legacy canvas; text-blobs → `layout` paragraphs with REAL metrics
     (true centering / measured-width truncation replaced the per-glyph
     advance guesses). Device-verified --no-art: boot-to-home, lock +
     swipe-up unlock, tile-tap launch, taskbar home.
   The legacy `my:skiko-gfx` canvas now has exactly one consumer class
   left: Kotlin/Compose (the hand-maintained skiko binding).
4. **Kotlin/Compose migration (task 102) — staged; Stage 1 DONE
   2026-06-11.** Stage 1 proved the path with `apps/user/wandr.ktcanvas.test`:
   a bare Kotlin guest whose bindings are GENERATED by the JetBrains Kotlin
   wit-bindgen fork (github.com/Kotlin/wit-bindgen branch `kotlin`, rev
   6b9cb12) — full wasi:canvas import + my:skiko-gfx/renderer export from
   one generator pass, zero hand-written ABI code, desktop + Pixel-2-XL
   verified (spilled paint blobs, shader borrows in records, gradient stop
   lists, SVG-path strings, list<record> line-metrics lifts, resource drop
   churn all correct). **Stage 2 DONE 2026-06-11**: Compose runs with
   geometry/transforms/clips/gradient-shaders on wasi:canvas (skiko
   `generated/wasicanvas/` bindings behind `WasiCanvasBackend.enabled`),
   desktop + device verified incl. scroll — the RenderNode question
   resolved cleanly because legacy and wasi verbs share the host's
   `renderer.canvas()`, so live-transform drawable recordings capture wasi
   draws; per-paint fallback covers color-filter and image-shader draws.
   **Stage 3 DONE 2026-06-12**: Compose text runs on `layout`, and the
   draft absorbed the one batched break Compose surfaced: paint grew
   `filter: option<color-filter>` (blend(color, mode) | invert),
   line-metrics became the full 13-field editor shape, paragraph gained
   ideographic-baseline + line-count, and rects-for-range was replaced
   by `selection-boxes` (height/width box styles + per-box direction).
   Host + every consumer rebuilt and device-verified in one event.
   Remaining: (4) images/bitmap-canvas → wasi (atomic Image migration +
   color-filter adoption for tinted icons), then cut the Kotlin worlds
   over fully — my:skiko-gfx keeps the non-canvas remainder
   (window/IME/lifecycle + the WasiDrawable live-transform layers,
   deliberately out of wasi:canvas).
5. Let the surface harden; then consider the WASI phase-0 conversation,
   positioned as wasi-gfx's 2D companion (champion overlap natural).
