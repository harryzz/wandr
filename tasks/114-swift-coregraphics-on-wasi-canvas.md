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

🟢 **P2.2 (2026-06-18) — Swift renders on wandr-host.** `wit/` exports the reactor
surface (`wasi:input-handlers/frame-handler` + `pointer-handler` +
`wandr:ui-shell/frame-pacing`), imports `wasi:canvas`; Swift `on_frame` does the
embedding handoff + `clear`/`draw-rect`/`draw-path`/`present`. `package.toml` =
`wandr.swift.canvas.test`; `build.sh` → `components/ui.wasm`. ✅ **DEVICE-VERIFIED 2026-06-18 (Pixel 2 XL)** — installed + launched; logs show
`eglSwapBuffers` → `rendered frame 0/1/2`, no traps; `screencap` confirms dark bg +
blue filled rect + green stroked triangle = what `on_frame` draws. First Swift
guest rendering on wandr. (WSLg desktop also runs it but weston crashes in its
bundled libpixman 0.43.2 — WSLg bug, not the guest.)
Gotcha fixed: caching the `get-context` `own` handle + re-borrow traps
(`unknown handle index 0`); acquire fresh each frame.

🟢 **P2.3 STARTED + DEVICE-VERIFIED (2026-06-18)** — Swift draws via
**CoreGraphics**. `Sources/CoreGraphicsWasi/` implements OpenCoreGraphics's empty
`CGContext` over `wasi:canvas`: state stack, CTM (translate/scale/rotate), current
path → SVG path-data (move/addLine/addQuadCurve/addCurve/addRect/close), fill/stroke
color + line width → `paint`, and fill/stroke/fillPath/strokePath/clear. `on_frame`
now uses the CoreGraphics API only (no raw wasi:canvas). Device-verified on Pixel 2
XL — same scene (blue rect + green triangle) via CGContext; eglSwapBuffers + frames,
no traps.

🟢 **P2.3b DONE (2026-06-18) — vendored REAL OpenCoreGraphics + merged CGContext.**
`Sources/OpenCoreGraphics/` is the actual upstream OCG library target (MIT, see
VENDORED.txt) with its empty `CGContext.swift` replaced by the wasi:canvas body and
an added `CGColor`. The guest draws with OCG's own `CGPath`/`PathElement`/`CGLineCap`
+ Foundation's `CGPoint`/`CGRect`. Device-verified on Pixel 2 XL (same scene via
genuine OpenCoreGraphics). Build note: OCG needs Foundation/CoreFoundation -> wasm
WASI emulation shims (-D_WASI_EMULATED_{SIGNAL,MMAN,PROCESS_CLOCKS} + -lwasi-emulated-*
in build.sh); cost = component ~7MB -> ~60MB (Foundation on wasm is heavy). A
production guest would use a slim no-Foundation geometry shim (the self-contained
P2.3 CoreGraphicsWasi variant, in git history).

🟢 **P2.3c DONE (2026-06-18) — the 3 mapping gaps emulated, device-verified.**
In the vendored `CGContext`, all 3 implemented with EXISTING wasi:canvas verbs (no
contract change): line dash (guest path-walk -> draw-path), offset+color shadow
(draw-twice: translate + mask-blur + color), alpha mask-clip (save-layer +
mask-then-content-with-src-in). Device-verified on Pixel 2 XL (dashed border, cyan
offset/blur shadow on a blue rect, green content clipped to a soft oval alpha mask).
Gotcha: mask-clip needs mask-first + src-in; content-then-dst-in only DIMS the mask
region (a primitive's blend touches only its own pixels). => wasi:canvas is
sufficient for the full CoreGraphics 2D CGContext.

🟢 **P2.3d/e DONE (2026-06-18) — gradients, clipping, images (device-verified).**
CGContext gained: CGGradient + drawLinear/RadialGradient(in:) (graphics.{linear,radial}-
gradient shaders); clip(to:)/clip() (clip-rect/clip-path); makeImage(rgba:) +
draw(_:in:) (graphics.image-from-rgba8 + draw-image-rect; CGImage now holds the wasi
image, dropped on deinit). Device-verified on Pixel 2 XL (linear gradient rect,
radial gradient clipped to a triangle, an RGBA bitmap). GOTCHA: image draws need an
opaque-WHITE paint color — the host multiplies color.alpha × paint.alpha
([[feedback_paint_alpha_pipeline]]), so color=0 made the image fully transparent.

**Remaining (P2.3 cont.):** text (needs the wasi:canvas/text world + a typeface).
SwiftUI stays out of scope.
