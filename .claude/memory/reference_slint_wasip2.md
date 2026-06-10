---
name: reference_slint_wasip2
description: "Slint on wandr is concretely feasible — wasip2 compiles (incl. the parley text stack); right seam = custom ItemRenderer over my:skiko-gfx (femtovg-style), NOT an impostor skia-safe; needs ~4 additive WIT verbs (typeface/draw-glyphs/shadow); effort ≈ 2-3 wandr-task-size"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**Slint on wandr guests — investigated 2026-06-10, spike completed same day
(probe: /tmp/slint-probe; source inventory: /tmp/slint @ master 1.17.0-dev).**

- ✅ **Compiles for wasm32-wasip2**: `slint = { default-features = false,
  features = ["compat-1-2", "renderer-software", "std"] }` → 4.4 MB component,
  ONLY `wasi:*` imports. The no_std combo does NOT build (browser-assuming
  wasm32 cfg paths). ✅ **The parley text stack also compiles**:
  `i-slint-core --features std,shared-parley` (parley 0.10 + fontique + skrifa
  + icu_normalizer) builds clean for wasm32-wasip2.
- **The old "text wall" is GONE on Slint master**: `renderer-skia` no longer
  uses SkParagraph — text is shaped GUEST-SIDE by parley
  (`i_slint_core::textlayout::sharedparley`); the renderer seam is the
  `GlyphRenderer` trait: it receives font-data blobs + glyph IDs + positions
  and draws via `canvas.draw_glyphs_at`. All text metrics
  (`text_size`/`char_size`/`font_metrics`/cursor mapping) are shared
  `sharedparley::*` helpers any renderer reuses wholesale. Glyph IDs stay
  consistent because host typeface is created from the SAME font bytes the
  guest shaped with (fonts: guest-embedded or fetched once via `assets.read`,
  registered with fontique — no system-font discovery on wasi).
- **Right seam = custom renderer crate** (`i-slint-renderer-wandr`)
  implementing `RendererSealed` + `ItemRenderer` (~25 high-level methods:
  draw_rectangle/border/image/text/path/box_shadow, clip/opacity/layer
  visits) + `GlyphRenderer`, exactly like `FemtoVGRenderer` (the proven
  non-skia template, internal/renderers/femtovg). The user's
  impostor-skia-safe idea (the skiko trick at the Rust level) WOULD work —
  Slint's skia canvas surface is small (~20 Canvas methods, Paint/Path/Image/
  gradients all map to existing WIT) — but it's strictly more work: you'd
  replicate skia-safe types/Handle semantics AND still need the same WIT
  additions. Direct ItemRenderer is the same mapping minus the type
  impersonation. i-slint-* internal crates are published but semver-unstable
  → pin exact version (Slint's own renderers do the same).
- **WIT gaps: CLOSED 2026-06-11** — the additive batch shipped (6 verbs:
  `create-typeface`/`drop-typeface`, `draw-glyphs` + `bc-` twin,
  `draw-shadow-rrect` + `bc-` twin), host impl in canvas_impl.rs
  (`guest_typefaces` map), WIT mirrored, host builds. Canonical contract +
  full union inventory (Compose/dioxus/Slint): **`docs/skia-wit-mapping.md`**.
  Everything else maps: gradients ✓ (linear/radial/sweep), images ✓,
  SVG-string paths ✓ (Slint paths are lyon → serialize), layers/opacity →
  save-layer + bc-* bitmap canvases ✓, clip ✓ (track clip stack guest-side
  like femtovg; no canvas query verbs needed). Colorize-by-gradient
  (image-filters) deliberately deferred.
- **Effort estimate**: renderer crate ≈ skia itemrenderer (1.3k lines) +
  renderer/lib glue (~500) + Platform/WindowAdapter wiring to wandr's
  input/frame-pacing/IME ≈ 2–3 wandr-task-units; WIT additions are a day
  (plus shared-WIT rebuild-all-consumers + zygote restart). Parley switch is
  master-only (1.17-dev) — build against git or wait for release.
- License: tri-license (GPLv3 / royalty-free / commercial).
- **Verdict vs dioxus-canvas**: now a real option, not a wall — but still not
  worth a migration for its own sake; dioxus-canvas already has
  input/IME/rotation/frame-pacing integrated. Adopt only if the Slint DSL /
  widget set / animations earn it; derisk with one test app
  (wandr.slint.test) implementing the renderer against the 4 new verbs.
