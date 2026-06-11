---
name: reference_qt_wandr
description: "Qt on wandr = NOT practical — Qt has NO wasi port (Emscripten/browser-only, checked 2026-06) so months of Qt-platform-porting precede any renderer work; Qt Quick scene graph is GPU-node-shaped (wrong for skiko-gfx); Slint IS the Qt-shaped option already shipped; memo = docs/qt-wandr-feasibility.md"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**Qt on wandr — analyzed 2026-06-11 (no spike; verdict doesn't hinge on
one). Full memo: `docs/qt-wandr-feasibility.md`.**

- The seams WOULD fit on paper: QPaintEngine is a proven custom-backend
  plug (QPdfEngine/QSvgGenerator are in-tree paint engines), Qt shapes its
  own glyphs (bundled HarfBuzz → drawTextItem ≈ the task-100 draw-glyphs
  model), QPA platform plugins = the WindowAdapter seam (browser wasm QPA
  proves it stretches to wasm).
- **Blocker 1 (showstopper): no wasm32-wasi target exists** — Qt's wasm is
  a `wasm-emscripten` cross-build assuming JS glue/DOM/WebGL; a wasi port
  = a new QtCore platform + JS-less QPA = months, unmaintained by us
  forever. Gate to revisit: upstream Qt growing a wasi port.
- **Blocker 2: Qt Quick ≠ canvas.** QSG renders geometry nodes + shader
  materials via RHI — that's the [[reference_wasi_webgpu_gfx]]
  guest-owns-renderer shape, not a paint stream; only QPainter paths
  (Widgets / QSG software adaptation / QQuickPaintedItem) map to
  skiko-gfx, forfeiting host-GPU for Quick content.
- Minor notes: C++ components are hand-wired but fine (wit-bindgen C
  generator; our wasmtime AOT flags already enable wasm-EH); tens-of-MB
  footprint; LGPL relink provision for static components.
- **Recommendation: don't.** "Qt developer experience" is already served
  by Slint (ex-Qt/Trolltech architects, .slint ≈ QML) — see
  [[reference_slint_wasip2]]; the four-way lineup table lives in the memo.
