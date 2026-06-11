# Qt on wandr — feasibility memo

> Written 2026-06-11, completing the guest-UI survey (after task 100/Slint
> shipped and the Avalonia memo, `docs/avalonia-wandr-feasibility.md`).
> Grounded in Qt's documented wasm architecture + the established Qt
> internals (QPaintEngine/QPA/scene graph); no source spike — the verdict
> doesn't hinge on details a spike would refine.
> Status: **analysis only — NOT recommended.**

## Verdict

**Architecturally Qt has the right seams, but the port is blocked one layer
below them: Qt has no wasm32-wasi port at all — its WebAssembly support is
Emscripten/browser-only — and Qt Quick's renderer is the wrong shape for a
skia-command stream.** Where Slint cost days and Avalonia is estimated in
weeks, Qt would cost **months**, most of it porting Qt itself to wasi
before a single wandr-specific line is written. And the strategic punchline
makes the exercise moot: **Slint is the Qt-shaped option wandr already
shipped** — founded by ex-Qt/Trolltech QML architects, with a DSL (.slint)
that is essentially QML — so the "Qt developer experience on wandr" box is
already ticked by task 100.

## The seams that WOULD fit (for the record)

Judged purely on the skia-wit-mapping checklist ("pluggable render
abstraction + self-shaped glyph text + portable platform layer"), Qt
qualifies on paper:

1. **QPaintEngine (the QPainter backend interface).** Custom paint engines
   are an established in-tree pattern — QPdfEngine and QSvgGenerator ARE
   custom paint engines that re-target QPainter's full op stream
   (rects/paths/images/glyph runs/clips/transforms/gradients). A
   `WandrPaintEngine` forwarding to `my:skiko-gfx/canvas` would map
   comparably to Slint's ItemRenderer: paths → SVG strings, gradients →
   shader ids, `drawTextItem` glyph runs → the task-100
   `create-typeface`/`draw-glyphs` verbs (Qt shapes text itself with its
   bundled HarfBuzz — glyph-level, same model as Slint/parley and
   Avalonia/HarfBuzzSharp).
2. **QPA (Qt Platform Abstraction).** Platform plugins are exactly the
   WindowAdapter/Platform seam — the browser "wasm" QPA plugin proves the
   pattern stretches to wasm-shaped hosts (event dispatcher, backingstore,
   input injection, IME via QPlatformInputContext).
3. **Fonts.** QPA font database is pluggable; embedded fonts or the
   `/system-fonts` preopen would feed it like the other ports.

## The blockers (why it still doesn't work)

1. **No wasi toolchain target — the showstopper.** Qt for WebAssembly is
   a cross-build for `wasm-emscripten`: it assumes Emscripten's JS glue,
   browser event loop integration, DOM/WebGL, Emscripten's pthread shim.
   wasm32-wasip2 (wasi-sdk/clang) is a *different platform* Qt has never
   been ported to — QtCore alone (event loop, QSocketNotifier, filesystem,
   process, locale, ICU-or-builtin) would need a new platform port plus a
   new QPA plugin with no JS to lean on. That's a Qt *platform port*
   (months, ongoing maintenance against a 25-year C++ codebase), not a
   renderer plug. Nobody upstream is working toward wasi (checked
   2026-06: Emscripten-only, browser-deployment focus).
2. **Qt Quick's scene graph is GPU-node-shaped, not canvas-shaped.** QSG
   renders geometry nodes + shader materials through RHI
   (OpenGL/Vulkan/Metal/D3D) — that's the wasi-webgpu/guest-owns-renderer
   model (see `reference_wasi_webgpu_gfx`), NOT a paint-command stream.
   Only the QPainter paths map onto `my:skiko-gfx`: Qt Widgets, the QSG
   *software adaptation* (flattens to raster — would forfeit host-GPU
   rendering), or `QQuickPaintedItem` islands. So even after a heroic wasi
   port, the modern half of Qt (QML/Quick with GPU materials) either
   degrades to software raster or waits for the Phase-2 wasi-gfx path.
3. **C++ component-model friction.** Doable (wit-bindgen has a C/C++
   generator; wasi-sdk + `wasm-tools component new` is established), but
   hand-wired — no componentize-style toolchain. C++ exceptions need
   wasm-EH (our wasmtime AOT flags already enable the exceptions proposal
   for Kotlin, so the device side is ready — small mercy).
4. **Footprint + licensing.** Qt browser-wasm builds run tens of MB before
   app code (worst of the four options); LGPLv3 static-linked into a
   component needs the relink-ability provision honored (or commercial).

## Where that leaves the guest-UI lineup

| | Compose (Kotlin) | dioxus-canvas (Rust) | Slint (Rust) | Avalonia (C#) | **Qt (C++)** |
|---|---|---|---|---|---|
| Status | shipping | production | shipped (task 100) | analysis: feasible | **analysis: not practical** |
| wasi toolchain | hand-rolled, shipping | native | native | preview (componentize-dotnet) | **nonexistent (Emscripten-only)** |
| Render seam fit | skiko (ours) | ours | ItemRenderer ✅ | IDrawingContextImpl ✅ | QPaintEngine ✅ but Qt Quick ✖ (GPU scene graph) |
| Effort | (paid) | (paid) | ~2 days (actual) | 2–4 weeks | **months (Qt-to-wasi port first)** |

**Recommendation: don't.** If the wish is "Qt-style development," Slint
already delivers it (QML-like DSL, same architects, native wasi, shipped on
wandr). If the wish is specifically the Qt Quick GPU pipeline, the honest
road is the Phase-2 wasi-webgpu/wasi-gfx guest-owns-renderer path, not
skia-command remoting. Revisit only if upstream Qt ever grows a wasi
platform port — that single fact flipping is the gate, and everything in
the "seams" section above becomes actionable the day it does.
