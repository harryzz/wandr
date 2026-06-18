# Task 114 — Swift CoreGraphics (OpenCoreGraphics) on `wasi:canvas`

> Scoped 2026-06-18. Outgrowth of the Swift feasibility re-analysis
> (`docs/swift-openswiftui-wandr-feasibility.md`). Goal: prove a **Swift guest
> draws on wandr** through `wasi:canvas`, by implementing the (currently empty)
> `CGContext` of **OpenCoreGraphics** over our canvas contract. This is the
> **canvas layer only** — a CoreGraphics-on-wandr drawing capability, *not*
> SwiftUI (that needs OpenRenderBox + OpenSwiftUI + text on top — explicitly out
> of scope, the follow-on wall).

## Why this, why now

The Swift **substrate** is the strongest of the gated languages — LLVM →
`wasm32-wasi`, standalone (no JS host), **no exnref/EH gate** (`throws` is
value-based). The **framework** is the wall. Between them sits one tractable,
in-our-control piece: the **CGContext drawing layer**. OpenCoreGraphics already
ships the CoreGraphics *type/geometry* layer (`CGPath` 252 LOC, `CGAffineTransform`,
`CGRect/Point/Size`, `CATransform3D`) but its **`CGContext` is an empty 16-line
stub** (`// Status: Empty`). So "make OpenCoreGraphics WASI-compatible to
`wasi:canvas`" = **implement that empty `CGContext` as calls into `wasi:canvas`** —
the same host-rendered-canvas consumer pattern we've shipped 5× (Compose, dioxus,
Slint, Avalonia, +the proposal). The `wasi:canvas` ↔ CGContext mapping is already
worked out (see the Swift doc) — fits with **3 minor gaps** (line dash;
offset+color drop shadow = the Flutter `drawShadow` deferral; alpha-mask/text-as-
clip). So the contract is ready; this task is implementation + the Swift toolchain
unknown.

## Prerequisite (the gate): Swift WASM SDK

Not installed in this environment. Phases 1+ need the Swift toolchain + the
**Swift SDK for WebAssembly** (`wasm32-unknown-wasi`). Install (user runs, heavy):

```
# swiftly (recommended) or swift.org toolchain, then the WASM SDK:
swift sdk install <swift-wasm-sdk-url>     # from swift.org / swiftwasm releases
swift build --swift-sdk wasm32-unknown-wasi
```

Until then, the no-Swift groundwork (WIT surface, wit-bindgen-c C bindings,
scaffold, CGContext design) is done in `repros/swift-canvas-spike/`.

## Phases (each a kill-gate)

- **P0 — toolchain binding surface (no Swift needed). DONE-able now.** A minimal
  custom WIT + `wit-bindgen c` generated headers — proves the C surface Swift will
  import via C-interop. Scaffold in `repros/swift-canvas-spike/`.

- **P1 — Swift custom-WIT round trip (the toolchain unknown).** A bare Swift
  `wasm32-wasi` module that, via `wit-bindgen c` + Swift C-interop (`@_cdecl`
  exports + calling the generated imports), **imports one host fn and exports
  one fn**, run through `wasm-tools component new --adapt …reactor.wasm`, called
  from a wasmtime component host. The Swift analog of `repros/java-wasm-spike`.
  Kills the "no public Swift custom-WIT precedent" unknown. **Gate:** if Swift
  C-interop can't express the canonical-ABI lowering cleanly, stop and report.

- **P2 — `CGContext` over `wasi:canvas`.** Fork/vendor OpenCoreGraphics; implement
  the empty `CGContext`:
  - state stack (`saveGState`/`restoreGState` → `canvas.save`/`restore`), CTM
    (`translate`/`scale`/`rotate`/`concat`).
  - current-path model: feed `CGPath` (exists) through a new `CGPath → SVG
    path-data` serializer → `canvas.draw-path`/`clip-path`.
  - fills/strokes/rects/ovals/clips; resolve CG graphics-state → a `wasi:canvas`
    `paint` value per draw (stroke cap/join/miter all already in `paint`).
  - color (ARGB; CMYK/Gray convert), blend, gradients (`graphics.linear/radial`),
    images (`decode-image`/`draw-image-rect`), offscreen (`new-offscreen`).
  - **Gaps:** dash (compute dashed geometry guest-side); offset+color shadow
    (draw-twice); mask-clip (`save-layer`+`dst-in`) — implement or defer.
  Acceptance: a Swift program issuing `CGContext` calls renders the expected
  frame on the **desktop dev loop** (task 101), then on **device**.

- **P3 — text.** CG glyph drawing / a minimal CoreText shim → `wasi:canvas`
  `text.glyphs.draw-glyphs` (+ `text.layout` for paragraphs). OpenCoreGraphics
  has no text yet; scope minimal (single-line glyph runs) or defer.

## Out of scope (the wall — explicitly NOT this task)

OpenRenderBox (display-list engine), OpenSwiftUI (framework + OpenAttributeGraph),
and full SwiftUI. This task stops at "Swift draws via CGContext on wandr." SwiftUI
is the follow-on, gated on those upstream projects maturing.

## Status

🟢 **P1 DONE (2026-06-18) — Swift is a working wandr custom-WIT component guest.**
Verified end to end (Swift 6.3.2 + `swift-6.3.2-RELEASE_wasm`, wasmtime 45):
Swift (`@_cdecl` export + C-interop imports over the `wit-bindgen c` surface) →
SwiftPM wasip1 **reactor** → `wasm-tools component new --adapt …reactor.wasm` →
valid WASI 0.2 component → wasmtime host (`repros/swift-canvas-spike/host/`)
provides WASI, implements `wandr:swift-spike/host {log, draw-rect}`, calls `run`;
Swift calls back with exact values. The "no public Swift custom-WIT precedent"
toolchain unknown is **killed**. Build facts (SwiftPM not raw swiftc; reactor via
`-Xclang-linker -mexec-model=reactor`; link the component-type `.o`;
`wasm32-unknown-wasip1`) captured in `repros/swift-canvas-spike/README.md` +
`build.sh`. **P0 done** earlier (WIT + verified wit-bindgen-c surface).

🟢 **P2.1 DONE (2026-06-18) — Swift drives REAL wasi:canvas.** The spike's `wit/`
imports `wasi:canvas/{types,draw,embedding}@0.0.2` (subset in `wit/deps/`);
`render` takes the embedding handoff and calls `clear`/`draw-rect`/`draw-path`
with `paint` records, all via Swift C-interop over the `wit-bindgen c` surface.
Compiles → `wasm-tools component new --adapt` → **valid component importing
`wasi:canvas/{types,draw,embedding}`, exporting `render`**. De-risk answered:
Swift handles wasi:canvas's rich ABI (flat `paint` struct, `rect`, resource
own/borrow handles), not just P1's scalars+string. (P1 runner preserved,
self-contained, in `host/` against `host/wit/`.)

**Next: P2.2 (render)** — add `wasi:input-handlers/frame-handler` (+ frame-pacing)
+ `package.toml`, run on **wandr-host** (skia wasi:canvas impl already exists,
`runtime/wandr-host/src/wasi_canvas_002_impl.rs`) on the desktop dev loop → real
pixels; then **P2.3** — implement OpenCoreGraphics's empty `CGContext` over these
calls (mapping in the feasibility doc).
