---
name: reference_slint_wasip2
description: Slint 1.16 COMPILES to wasm32-wasip2 (std + renderer-software + compat-1-2, clean wasi:* imports) — but its skia backend is native skia-safe (can't target our WIT); integration = custom Platform + ItemRenderer; text pipeline is the wall
metadata:
  type: reference
---

**Slint on wandr guests — investigated 2026-06-10 (probe: /tmp/slint-probe).**

- ✅ **Compiles for wasm32-wasip2**: `slint = { default-features = false,
  features = ["compat-1-2", "renderer-software", "std"] }` → 4.4 MB component
  with ONLY `wasi:*` imports (wasm-bindgen/web-sys are in the dep tree from
  `target_arch=wasm32` browser branches but nothing browser-specific links).
  The no_std combo (`unsafe-single-threaded`+`libm`, no `std`) does NOT build
  — i-slint-core's wasm32 cfg paths assume the browser there.
- ❌ **Slint's `renderer-skia` is NOT retargetable at our WIT**: it binds
  skia-safe natively (C++ Skia in-process, real GPU/raster surface). Our
  `my:skiko-gfx/canvas` is skia-SHAPED COMMANDS over the component boundary —
  different thing. Slint's pluggable seam is the **`ItemRenderer` trait**
  (what the software renderer implements).
- Two integration shapes if ever wanted:
  (A) software renderer → pixel buffer → new blit/shared-mem WIT (+damage
  rects). Simple, but CPU-renders in wasm + ~16 MB/frame at panel res —
  fights the idle-CPU work and host-GPU model.
  (B) custom `Platform` + `ItemRenderer` → canvas WIT verbs (the dioxus-canvas
  sink equivalent). Right shape; the WALL is TEXT: Slint shapes/lays out text
  in-guest (glyph-level, fonts in-guest) vs wandr's host-owns-fonts rule
  (`measure-text`/text blobs host-side, [[feedback_android_fonts]]).
- License: tri-license (GPLv3 / royalty-free desktop-mobile / commercial).
- **Verdict vs dioxus-canvas: not better for wandr.** Slint's wins (DSL,
  widgets, animations, no async runtime — single-threaded fits wasip2) don't
  outweigh: text-pipeline redesign, losing host-GPU rendering or building a
  full ItemRenderer bridge, and re-integrating input/IME/rotation/frame-pacing
  that dioxus-canvas already has. If the DSL appeal grows, derisk with a
  bounded route-(B) spike on one test app, text first.
