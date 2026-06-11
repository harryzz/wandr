---
name: reference_avalonia_wandr
description: "Avalonia (C#/.NET) CAN be ported to wandr the same way as Slint — pluggable IPlatformRenderInterface + self-shaped glyph text map onto my:skiko-gfx with ZERO new WIT verbs; risk is toolchain (componentize-dotnet preview) + footprint, not rendering; full memo = docs/avalonia-wandr-feasibility.md"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**Avalonia on wandr — analyzed 2026-06-11 (no spike run). Full memo:
`docs/avalonia-wandr-feasibility.md`; source inventory at /tmp/avalonia.**

- Same shape as Slint: Skia-backed UI framework with a pluggable render
  abstraction (`IPlatformRenderInterface`/`IDrawingContextImpl`; in-tree
  proof = 618-line `HeadlessPlatformRenderInterface`, zero Skia) and
  GUEST-shaped glyph-level text (`ITextShaperImpl` via the standalone
  `Avalonia.HarfBuzz` package → `DrawGlyphRun`) → the task-100
  `create-typeface`/`draw-glyphs` verbs fit unchanged.
- **Zero new WIT verbs for a v1 port** — full draw-op mapping in the memo;
  only deferred gaps are PushOpacityMask + PushEffect (blur-on-content
  needs the image-filter-on-layer verb the WIT still lacks).
- Risk lives in the RUNTIME, not rendering: componentize-dotnet
  (NativeAOT-LLVM, preview ~0.6–0.7, historically Windows-only builds —
  verify), harfbuzz must link as wasm32-wasi native dep (hard prereq, no
  managed shaper exists), footprint ~30–80 MB wasm vs Slint's 8.7 MB.
  NativeAOT = own GC in linear memory → NO wasm-gc, none of the Kotlin
  adapter pain. Platform interfaces are [NotClientImplementable]/unstable →
  pin exact version like [[reference_slint_wasip2]].
- Effort ≈ 2–4 weeks (vs Slint's 2 days); derisking spike #1 = a bare C#
  guest exporting our `renderer` world via componentize-dotnet (~a day,
  kills the biggest unknown). Wait for a concrete need.
