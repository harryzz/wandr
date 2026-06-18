# Swift + OpenSwiftUI on wandr — feasibility memo

> Researched 2026-06-13 (web only, no spike). Companion to the other
> guest-UI evals (`avalonia-wandr-feasibility.md`, `qt-wandr-feasibility.md`,
> `flutter-wandr-feasibility.md`). Question: can a Swift/SwiftUI-style guest
> render on wandr through `wasi:canvas`, the way Slint/Avalonia do?

## Verdict

**Not practical yet — but the gate is the FRAMEWORK, not the runtime
(re-examined 2026-06-18; see that section below).** Mirror image of Dart:
Swift has the **strongest substrate** of the gated languages (LLVM →
`wasm32-wasi`, standalone, no JS host, **no exnref/EH gate** — `throws` is
value-based, so Dart's #54394 has no analog here; Embedded Swift available) and
the **weakest framework** (no shipping Skia/Canvas existence proof). Toolchain
(custom-WIT) has **narrowed** from "no precedent" to "emerging" — WasmKit
`wit-tool` now does `.wit`↔Swift, and it's *in our control* (skiko-tier), unlike
Dart's compiler gate. And `wasi:canvas` ↔ OpenCoreGraphics (CGContext) maps with
only **3 minor, already-anticipated gaps** (dash, offset-shadow, mask-clip) — so
the contract is ready; the blocker is OpenSwiftUI/OpenCoreGraphics *maturity*. Net:
upgrade the substrate verdict, keep "wait" for the framework reason only. The
original 2026-06-13 framing below:

**Not practical yet — gated on upstream (Qt/Flutter class), with no active
momentum on either gating piece.** The architecture is the right *shape*
(pluggable renderer + the wasip1+`--adapt` route are both real), but the two
things that would make it actually work are **unproven** (toolchain) and
**untracked / design-only** (framework) — see the evidence check below. Not
"almost possible": tractable-in-principle, nobody's-doing-it. Today:

1. **Toolchain (producing the guest):** *narrower than it first looks.*
   Swift compiles to `wasm32-wasi` (WASI Preview 1) officially (Swift SDK
   for WebAssembly, 6.2+), and that's **enough** — every wandr guest is a
   wasip1 module run through the **wasip1→component adapter**
   (`wasm-tools component new --adapt wasi_snapshot_preview1.wasm`). Compose
   (Kotlin, wandr-fork adapter) and Avalonia (C#, componentize-dotnet's
   bundled adapter) BOTH ship this way; neither emits a native wasip2
   component. So Swift's wasip1 output is not the blocker, and WasmKit's
   wasip2 work (a *host* runtime concern) is irrelevant to producing a
   guest. The actual gap is the **custom WIT bindings** — no Swift
   `wit-bindgen` generator (it ships Rust/C/C++/C#/Go), so the
   `wasi:canvas`-import lowering + input-handler-export lifting +
   component-type must come from elsewhere: hand-rolled canonical ABI (the
   Kotlin-skiko model) or, more practically, **`wit-bindgen c` + Swift's
   first-class C-interop**. Bounded work, the same class Kotlin already
   pays. (The community's Swift-on-wasm UI energy is browser/DOM —
   ElementaryUI/BridgeJS — not this, so you'd be early.)
2. **Framework (OpenSwiftUI):** the cross-platform reimplementation is
   **early**. Off-Apple it's Ubuntu-partial (build/test, no deploy);
   Android/Windows "not supported yet"; **no WASI target**. Crucially
   **text is not supported yet**, and `OpenAttributeGraph` (the reactive
   engine) is "not fully implemented — only API-compatible," so most core
   features only work on Apple against the real `AttributeGraph`.

## Evidence check (forums / issues / forks, 2026-06-13)

Verified against primary sources, because the "shape fits + adapter route"
reasoning risked overstating readiness:

- **No Swift custom-WIT precedent.** `wit-bindgen` has no Swift backend
  (Rust/C/C++/C#/Go); there is **no `componentize-swift`** (cf.
  componentize-py/js/dotnet, Go bindings); and no public POC of a Swift
  wasm component **exporting a custom WIT interface** surfaced. WasmKit's
  component-model work (SwiftWasm Feb/Mar-2026 updates) is a **host runtime**
  feature (running components), not guest production. Swift→wasm
  *compilation* is solid (in CI), but the wasip1+`--adapt`+(wit-bindgen-c +
  C-interop) chain for a guest with custom imports/exports is **untried in
  public** — same unknown class that cost the Kotlin/skiko path months + a
  fork.
- **OpenSwiftUI's Skia renderer is diagram-only.** The issue tracker has
  **zero** items on WASM/WASI/Skia/a cross-platform renderer / rendering
  roadmap. The shipped off-Apple renderer is GTK4 (windowing-bound); text
  unsupported; no WASI target. So the wandr-relevant rendering path has no
  tracked work behind it — it's a box in `arch.png`, not an effort.
- **Context:** WASI Preview 2 went *stable* in 2025 (P3 in progress) — the
  spec is mature, which removes one excuse, but doesn't move Swift's guest
  tooling or OpenSwiftUI's renderer.

Net: the adapter insight is real (the toolchain isn't blocked on Swift
gaining *native* wasip2), but "tractable if someone writes the glue + the
renderer" is not "almost there." Both gating pieces are absent today.

## Why the *shape* is nonetheless right

SwiftUI's pipeline is the same shape that made Slint/Avalonia cheap:
view tree → **AttributeGraph** (reactive dependency engine) → **RenderBox**
(Apple's private C++ engine) emits a **display list** of drawing commands →
CoreAnimation/Metal composite + CoreText for text. A display list is
exactly what maps onto `wasi:canvas` (it's what Slint's ItemRenderer and
Avalonia's `IDrawingContextImpl` are). OpenSwiftUIProject mirrors this with
**OpenRenderBox/OpenBox** (RenderBox reimpl) + **OpenAttributeGraph**.

**Crucially, OpenSwiftUI already has a pluggable renderer abstraction** —
the `main` `Package.swift` selects a backend per platform via build
conditions: `renderBoxCondition` (RenderBox), `renderGTKCondition` (GTK4 /
Cairo, the current **Linux** path via the `CGTK` system lib),
`swiftUIRenderCondition` (Darwin). The **architecture diagram
(`Screenshots/Architecture/arch.png`) additionally shows a Skia renderer**
as a (cross-platform) backend — but there is **no Skia target in `main`
yet**; it's design/roadmap, the shipped non-Apple renderer is GTK4. The
Swift ecosystem already has Skia bindings to build it on (`SkiaKit` —
migueldeicaza / UnGast).

This is the load-bearing good news: a pluggable renderer protocol with
multiple swappable backends is **exactly the seam wandr needs** — a
`WandrRenderer` backend (forwarding to `wasi:canvas`) would slot in beside
GTK4/Skia/RenderBox, the same way slint-wandr/avalonia-wandr slot a backend
into Slint/Avalonia. And a **Skia** backend is the ideal match: `wasi:canvas`
is itself skia-shaped (grown from skiko), so Skia-API draw ops map ~1:1.
So the architecture is genuinely the right shape — confirmed, not just
inferred. The mold fits; the parts (a non-Apple rasterizing renderer + text)
aren't built yet.

## The two concrete blockers

**Toolchain.** A wandr guest is a wasip2 component importing `wasi:canvas`,
but — as Compose and Avalonia prove — you don't need *native* wasip2 from
the language: a wasip1 module + the `--adapt` step is the whole game, and
Swift already emits wasip1. Swift, being conventional linear-memory (like
C#/Rust, unlike Kotlin's WasmGC), would almost certainly use the **stock**
adapter, not the wandr fork (the fork exists only for the Kotlin
ScopedMemory/State bug). So component production is a solved-shape problem.
The one real piece of work is the **custom WIT bindings**: no Swift
generator → hand-rolled canonical ABI
(`[[feedback_wit_bindgen_no_kotlin_generator]]`, the skiko-Kotlin model)
or, more practically, `wit-bindgen c` + Swift C-interop. Real but bounded,
and the same class Kotlin already pays. Footprint: the full Swift runtime
in wasm is not small (tens of MB class); **Embedded Swift** shrinks
binaries but drops reflection / large parts of the runtime and existentials
that SwiftUI-style code leans on — likely incompatible with OpenSwiftUI
as-is, so plan for the full runtime.

**Framework.** OpenSwiftUI is ~2 years in, active, but early off-Apple: no
text, incomplete AttributeGraph, no WASI target, and the only shipped
non-Apple renderer is GTK4/Cairo (a windowing-system-bound path, not
wandr-usable). The pluggable-renderer abstraction it DOES have (correction
to an earlier draft of this memo: it exists — RenderBox/GTK4/SwiftUI-render
backends, with Skia diagrammed) clears the "pluggable renderer exists" bar
that Avalonia/Slint needed. What's missing is a *non-Apple, non-GTK,
rasterizing* backend (the diagrammed Skia one) **plus text** — until one of
those lands, there's nothing to point at `wasi:canvas`.

## Recommendation

**Wait.** Don't spike now — neither half is ready. Track two signals:
1. The **Skia renderer in the arch diagram** actually landing as a target
   (or any non-Apple software-rasterizing backend) + text
   (CoreText-replacement). A Skia backend is the green flag — its draw ops
   map ~1:1 to `wasi:canvas`, so the `WandrRenderer` becomes a thin reskin
   of it rather than new work.
2. (Lesser signal — the toolchain is mostly there.) A **Swift custom-WIT
   bindings story** — most likely `wit-bindgen c` + C-interop, since
   component production via wasip1+`--adapt` already works. Not worth
   building until signal 1 lands; there's nothing to draw with first.

Derisking spike order *if/when* both move: (1) a bare Swift wasip2 component
exporting a trivial WIT via `wit-bindgen c` + C-interop and drawing one
`draw-rect` — kills the toolchain unknown, same role as the Avalonia spike
#1; (2) OpenRenderBox display-list → `wasi:canvas`. Until then this is a
"promising language, not-ready stack" — revisit, don't build.

## Re-examined 2026-06-18 — substrate vs framework, and the OpenCoreGraphics mapping

Re-derived from first principles (the TeaVM/Dart lens: split **substrate** from
**framework**, check host-shimmability, check the EH/exnref angle) rather than
restating the 2026-06-13 verdict. The shape sharpens into a clean mirror image of
Dart.

### Swift is the mirror image of Dart: best substrate, worst framework

- **Substrate — essentially DONE, and the strongest of the gated languages.**
  Swift is **not** WasmGC+JS-host (Dart/Kotlin); it's LLVM → **`wasm32-wasi`
  linear memory**, merged since 2024, **runs standalone on wasmtime with no JS
  host at all**. So the entire "WASI stdlib" workstream that gates Dart **does not
  exist for Swift** — the stdlib already builds for wasm32-wasi over WASI libc.
- **No EH/exnref gate.** This is the decisive contrast with Dart. Swift's `throws`
  is **value-based error propagation** (error in the calling convention,
  compile-time checked), *not* wasm exception handling; traps lower to
  `unreachable`. So Swift-on-wasm **does not depend on the wasm exception-handling
  proposal** — the legacy→`exnref` migration that gates Dart (#54394) has **no
  analog here**. Embedded Swift on wasm exists for a runtime-light footprint.
- **Toolchain (custom-WIT) — NARROWED since the 2026-06-13 note.** That note said
  "no Swift custom-WIT precedent." Now stale: **WasmKit's `wit-tool` generates
  Swift bindings from `.wit` (and `.wit` from Swift)**, and bidirectional
  component interop is an explicit Swift goal ([WebAssembly
  vision](https://github.com/swiftlang/swift-evolution/blob/main/visions/webassembly.md)).
  Still preliminary, but moved from *absent* → *emerging*; plus the `wit-bindgen
  c` + first-class C-interop fallback. Crucially this gate is **in our control**
  (a skiko-tier spike), unlike Dart's compiler-gated EH.
- **Framework — THE wall, and worse than Dart's** (no shipping existence proof
  like Flutter's web_ui). This stays the gating piece.

Net: Swift isn't "harder/easier" than Dart — it's gated on the **opposite layer**.
Dart's gate is toolchain/runtime (closing fast, Google-funded); Swift's gate is
the **UI framework** (no funded owner). Upgrade the *substrate* verdict (Swift's
WASI story is the strongest, EH-gate-free); the *overall* "wait" stands for the
framework reason only.

### The OpenSwiftUI rendering stack — it's a CoreGraphics (Canvas2D) clone

OpenSwiftUI mirrors Apple's *entire* private 2D stack as open analogs, and the
WASM ones are named explicitly:

- **OpenAttributeGraph** ← Apple `AttributeGraph` (reactive engine; "API-compatible
  only" today).
- **OpenRenderBox** ← Apple's private C++ **RenderBox** display-list engine;
  explicitly targets *"Linux, WASI, and Windows."*
- **OpenCoreGraphics** ← Apple **CoreGraphics (CGContext / Quartz 2D)** — *Apple's
  Canvas2D* — "for use in **WASM** environments, 100% API compatibility."

Pipeline: views → OpenAttributeGraph → OpenRenderBox (display list) →
**OpenCoreGraphics (CGContext-shaped 2D)** → a backend. Because the drawing API is
CGContext — a **SkCanvas twin** — Swift sits firmly in the **`wasi:canvas` lane,
not WebGPU** (CGContext has no mesh/pipeline primitive). SwiftUI's optional
`Shader`/`.colorEffect`/`drawingGroup()` Metal effects are the same additive
GPU-lane tail as Flutter's `drawVertices`/`FragmentProgram` — they'd ride a future
`wasi-webgpu`, but are not the core and not required.

### `wasi:canvas` 0.0.2 ↔ CoreGraphics (CGContext) mapping

Mapped against the CoreGraphics/CGContext API surface OpenCoreGraphics mirrors
(its own README is thin / early, backend unconfirmed — see caveat). **The contract
fits with only 3 minor, already-anticipated gaps** — `wasi:canvas` is *not* the
blocker.

| CGContext op | `wasi:canvas` 0.0.2 home |
|---|---|
| SaveGState/RestoreGState | `canvas.save`/`restore` |
| BeginTransparencyLayer(WithRect)/End | `canvas.save-layer(bounds, alpha)` |
| Translate/Scale/Rotate/ConcatCTM | `canvas.translate`/`scale`/`rotate`/`concat` |
| BeginPath/MoveTo/AddLine/AddCurve/AddQuadCurve/AddArc/AddRect/AddEllipse/ClosePath | accumulate into an SVG path-data string → `canvas.draw-path`/`clip-path` (binding holds CG's mutable current-path guest-side) |
| FillPath/EOFillPath/StrokePath/DrawPath | `canvas.draw-path(path, fill-rule, paint.style)` |
| FillRect(s)/StrokeRect/ClearRect | `canvas.draw-rect` / `clear` |
| FillEllipseInRect/StrokeEllipseInRect | `canvas.draw-oval` (bonus: `draw-rounded-rect`/`draw-double-rounded-rect`) |
| Clip/EOClip/ClipToRect(s) | `canvas.clip-path`/`clip-rect` (fill-rule) |
| SetFill/StrokeColor (+RGB/Gray/CMYK) | `paint.color` (ARGB; CMYK/Gray convert guest-side) |
| SetAlpha / SetBlendMode | `paint.alpha` / `paint.blend` |
| SetLineWidth / SetLineCap / SetLineJoin / SetMiterLimit | `paint.stroke-width`/`stroke-cap`/`stroke-join`/`stroke-miter` ✓ (all present) |
| SetShouldAntialias / SetInterpolationQuality | `paint.anti-alias` / `sampling` |
| SetFill/StrokePattern | `graphics.image-pattern` / `shader-blend` |
| DrawLinearGradient / DrawRadialGradient | `graphics.linear-gradient`/`radial-gradient` (+ sweep = superset) |
| DrawImage(rect)/DrawTiledImage; bitmap decode | `canvas.draw-image`/`draw-image-rect`; `graphics.decode-image`/`image-from-rgba8` |
| ShowGlyphsAtPositions / SetFont / Core Text layout | `text.glyphs.draw-glyphs` (+ `typeface.from-bytes`); high-level → `text.layout` paragraph/builder (richer than CG) |
| CGLayer / CGBitmapContext (offscreen) | `graphics.new-offscreen` + `canvas.snapshot` |
| PDF page ops / color-space mgmt / font smoothing | out of scope (not UI) / host-internal — not gaps |

**The 3 "gaps" — all emulated with EXISTING verbs, device-verified 2026-06-18**
(task 114 P2.3c, in `repros/swift-canvas-spike`'s vendored `CGContext`; no
`wasi:canvas` contract change — so the contract is *sufficient* for the full
CoreGraphics 2D `CGContext`):

1. **Line dash** (`CGContextSetLineDash`) — no dash field in `paint`. Emulated
   guest-side: flatten the path to polylines, split into on/off runs per the
   pattern+phase, emit "on" runs as sub-paths → `draw-path`. (Flutter dashes
   in-framework too.)
2. **Offset+color drop shadow** (`CGContextSetShadowWithColor`) — `paint.blur`
   (mask-blur) has sigma but **no offset/color**; emulated by drawing the shape
   twice — a `translate`d copy in the shadow color with `paint.blur`, then the real
   shape. (The Flutter `dart:ui` `drawShadow` deferral; `scene.layer.
   set-shadow-elevation` covers layer elevation shadows.)
3. **Alpha mask-clip** (`CGContextClipToMask`, text drawing mode `.clip`) — no
   alpha-mask clip verb; emulated via `save-layer` + a blend mode. **Correct idiom:
   mask-first, then content with `src-in`** (content covers the whole region so
   `src-in` zeroes it outside the mask). The dual (content then mask with `dst-in`)
   only *dims* the mask region — a drawn primitive's blend touches only its own
   pixels.

**Structural note (not a gap):** CG is an *imperative* model (mutable current-path
+ mutable graphics state); `wasi:canvas` is *value-paint + path-as-string*. So the
OpenCoreGraphics→`wasi:canvas` backend holds the CG graphics state guest-side and
flushes a resolved `paint` per draw — exactly the adaptation Avalonia's
`DrawingContext`→skiko and `dart:ui`→canvas already make. Binding-layer work, not
a contract change.

**Caveats.** OpenCoreGraphics is *early*: README gives no backend (TODO references
Silicae / libs-quartzcore), and OpenSwiftUI **text is unsupported** today, so
"100% API compatibility" is an API-surface claim, not "produces pixels." The
mapping proves the **contract** is ready (the retarget seam is OpenCoreGraphics's
backend, or OpenRenderBox's display-list replay, → `wasi:canvas`); the blocker
remains OpenSwiftUI/OpenCoreGraphics *maturity* and *building that backend* — not
any missing `wasi:canvas` verb.

## How it slots into the guest-UI lineup

| | Avalonia (C#) | Slint (Rust) | **Swift + OpenSwiftUI** |
|---|---|---|---|
| Guest toolchain | wasip1 + adapter (WIT gen) | wasip1 + adapter + wit-bindgen | wasip1 + adapter ✓ (same route); gap = **WIT bindings** (C-interop or hand-roll) |
| Framework cross-platform | mature + pluggable headless backend | shipping skia/femtovg backends | **early off-Apple; no text; no WASI** |
| Renderer→wasi:canvas | shipped (this repo) | shipped (this repo) | right *shape* (RenderBox display list) but not built off-Apple |
| Verdict | build it ✅ | shipped ✅ | **wait** — gated on upstream (Qt/Flutter class) |

Sources: Swift-for-Wasm Mar-2026 update + Swift.org WASM SDK docs;
OpenSwiftUIProject (OpenSwiftUI / OpenBox / OpenRenderBox /
OpenAttributeGraph) READMEs + Swift Package Index; wit-bindgen language
list. Cross-checks against `docs/wasm-component-language-support.md`
(Swift not first-class) and `[[reference_qt_wandr]]` /
`[[reference_flutter_go_ui_wandr]]` (the "gated on upstream" pattern).

## OpenSwiftUI integration architecture + ORB/OAG probe (2026-06-18)

Added after the CGContext-on-wasi:canvas work (task 114): with OpenCoreGraphics's
`CGContext` complete over `wasi:canvas`, this is exactly how OpenSwiftUI would sit
on top — and a probe of how complete the two layers above it are.

### The decisive seam
`OpenRenderBox/Sources/OpenRenderBox/Render/ORBDisplayList.swift` exposes
`public func render(in context: CGContext, options:)` (and
`beginCGContext(withAlpha:) -> CGContext`). **The whole OpenSwiftUI stack
rasterizes into a `CGContext`** — i.e. into the thing we implemented over
`wasi:canvas`. So OpenCoreGraphics is the *bottom of OpenSwiftUI's pipeline*, not
a parallel demo.

### Layer stack (non-Darwin / wandr path; deps confirmed in OpenSwiftUI Package.swift)
```
OpenSwiftUI        View / @State / modifiers / layout / gestures
  ├─ OpenAttributeGraph   reactive BRAIN: attributes/rules/subgraphs; @State →
  │                        invalidate → recompute the render tree
  └─ OpenRenderBox        render OUTPUT: ORBDisplayList (draw items) +
        │                  ORBDisplayList.render(in: CGContext)  ◄── THE SEAM
        ▼
     OpenCoreGraphics  CGContext  ✅ (paths/fills/strokes/gradients/images/text/
        │              clip/blend)  — done on wasi:canvas, device-verified
        ▼
     wasi:canvas → wandr-host (skia/EGL) → pixels
  (OpenQuartzCore/CALayer sits beside ORB for the layer tree — currently a stub;
   maps naturally onto wasi:canvas `scene.layer` if SwiftUI is layer-backed.)
```

### Per-frame data flow
```
wandr frame-handler.on-frame
  → OpenSwiftUI render pass (bodies/layout/geometry are ATTRIBUTES)
  → OpenAttributeGraph recomputes only what @State invalidated
  → OpenRenderBox ORBDisplayList (the frame's draw items)
  → displayList.render(in: cgContext)            ← our CGContext
  → OpenCoreGraphics → wasi:canvas → skia/EGL → screen
pointer/key (wasi:input-handlers) → OpenSwiftUI events → @State → next frame
```

### Two render routes (reconciles with "Why the *shape* is right" above)
OpenSwiftUI selects a renderer per platform (`renderBoxCondition` / `renderGTKCondition`
/ `swiftUIRenderCondition`); the **shipped non-Apple path is GTK4/Cairo**, and a Skia
backend is diagram-only. The `ORBDisplayList.render(in: CGContext)` route is the
**RenderBox-style** path. So a `WandrRenderer` could either (a) implement that route
over our `CGContext`, or (b) slot in as a backend *beside* GTK forwarding to
`wasi:canvas`. Either way it terminates in `wasi:canvas` (via our CGContext). The
probe below is of the RenderBox route + the shared reactive engine.

### Probe of the two layers above CGContext (clones @ HEAD, 2026-06-18)

| Layer | What it must provide | Probe finding | Verdict |
|---|---|---|---|
| **OpenCoreGraphics** | the `CGContext` render target | full CGContext on wasi:canvas | ✅ done |
| **OpenRenderBox** | build `ORBDisplayList` + `render(in: CGContext)` that walks every item kind → CGContext | **`render(in:)` is `_openRenderBoxUnimplementedFailure()` — a STUB**; no display-list item/content model; ~935 LOC total, mostly `ORBPath*`/animation shells | 🔴 the rasterizer **doesn't exist yet** — the seam is our CGContext, but the walker + display-list content model are unwritten |
| **OpenAttributeGraph** | functional reactive graph (attributes, rules, subgraphs, invalidation, type-metadata runtime) | **released + actively developed** — v0.5.0, commits days old (2026-06-16); ~1,956 Swift + ~4,733 C++ (`OAGGraph`/`OAGSubgraph`/`OAGAttribute`); **23-file AttributeGraph compatibility test suite**; **builds + `swift test` on Linux in CI (`ubuntu.yml`, Swift 6.3.2)** via a setup script. Far past "API-compatible only". **Caveat for wandr:** the C++ engine reflects Swift's runtime type-metadata ABI (`#include <swift/Runtime/Metadata.h>`), so it needs Swift's *private runtime headers* at build (provided by their CI setup, not a stock toolchain — a naive `swift build` fails here on that header). **No WASI/wasm CI job** — WASI is a stated goal, not yet verified building for wasm; the metadata-header/ABI coupling is the open hurdle. | 🟡 mature on Linux/Apple; **unproven on WASI** (the wandr gate) |
| **OpenSwiftUI** | the framework producing the display list via OAG | early off-Apple (text unsupported, GTK4 renderer) | 🔴 early |
| **wandr driver** | drive a render pass on on-frame, feed input, present | we have the reactor (on-frame/pointer/frame-pacing) | 🟢 trivial |

### Corrected takeaway
Earlier I framed OAG as the single gate and ORB as "in our control, builds on what
we have." The probe flips that: **OAG is the more-built layer** (real Swift+C++
engine in progress), while **OpenRenderBox's `render(in: CGContext)` rasterizer is
an explicit stub** and its display-list content model is absent. So OpenSwiftUI on
wandr is gated on BOTH maturing — and the *ORB display-list→CGContext walker* (which
renders into our CGContext) is the larger missing piece today, even though its
bottom edge (CGContext) is solved. Of the four boxes: bottom done, driver trivial,
**ORB rasterizer = biggest hole, OAG = substantial-but-unverified.** Still
upstream-gated (don't fork the framework); revisit when ORB's `render(in:)` lands.

### OAG status re-check (2026-06-18) — mature, but WASI is the wandr gate
Prompted by recent GitHub activity: OpenAttributeGraph is **not** "in progress /
unverified" — it's **released (v0.5.0), actively developed (commits days old), and
builds + passes a 23-file AttributeGraph compatibility suite on Linux CI** (Swift
6.3.2). So on Linux/Apple it's a real, tested engine. The wandr-specific blockers
are narrower than "is the engine done":
1. **No WASI build yet** — WASI is a stated project goal but there's no wasm CI job;
   building for `wasm32-wasip1` is unproven.
2. **Swift runtime-metadata coupling** — the C++ core `#include`s
   `swift/Runtime/Metadata.h` (it reflects Swift's type-metadata ABI to apply
   attribute rules), so it needs Swift's *private runtime headers* and a matching
   metadata ABI. On a stock toolchain a naive `swift build` fails on that header;
   on wasm those headers + ABI availability are the real question.
So the OAG gate for wandr isn't "finish the engine" (it's largely done) — it's
"**make the existing engine build + run on the WASI target**" (the metadata-ABI/
header port). That's a more bounded, upstream-shaped task than reimplementing it.

**WASI build attempted 2026-06-18 (`swift build --swift-sdk swift-6.3.2-RELEASE_wasm`
+ Foundation emulation flags) — fails fast in OAG's native layer.** Two concrete
blockers pinned:
1. **POSIX gap** — `Platform/include/platform/log.h` `#include <syslog.h>` →
   `'syslog.h' file not found` (WASI has no syslog). The Platform/Utilities C layer
   assumes a fuller POSIX than wasip1 provides.
2. **Swift runtime-metadata coupling** — `metadata.cpp` `#include
   <swift/Runtime/Metadata.h>` (absent from the toolchain *and* the wasm SDK; only
   in the Swift compiler source) + a matching metadata ABI on wasm.
So OAG is **mature on Linux/Apple but not WASI-portable today** — the gate is a
real native-layer port (POSIX shims for the Platform/Utilities C + the Swift
runtime-metadata headers/ABI for the C++ core), upstream-shaped, not a wandr fork.
This is the single biggest dependency for OpenSwiftUI-on-wandr.

#### syslog shim probe + `Metadata.h` attainability (2026-06-18)
Added a `syslog.h` shim (stderr-redirect, `static inline`) on the WASI build's
include path → blocker #1 cleared; the build **marched straight to blocker #2**
(`swift/Runtime/Metadata.h`), confirming the POSIX gaps are shimmable and the
metadata coupling is the real wall.

- **The right destination for that shim is `wasi:logging`, and wandr already
  implements it** (`wit/deps-upstream/logging/logging.wit` @0.1.0-draft +
  `runtime/wandr-host/src/consolidated_impl.rs`). So a production OAG-on-wandr
  lowers `Platform/log.c` `syslog()` → `wasi:logging/logging.log(level, context,
  message)` (`LOG_CRIT/ERR`→error/critical, `LOG_INFO`→info), reaching host
  logging (logcat on device). Guest-side `wit-bindgen c` import; **host half done**.

- **Is `Metadata.h` / the metadata ABI attainable on `wasm32-wasip1`? YES.**
  Probed the wasm SDK's `libswiftCore.a` (wasi): it **defines 749 metadata symbols,
  including the C++ `swift::TargetExistentialTypeMetadata<InProcess>` methods** that
  `Metadata.h` declares — i.e. the exact runtime types OAG's C++ reflects, compiled
  for wasm32. (Expected: WebAssembly is a tier-1 Swift target since 6.1; generics/
  classes need metadata.) The runtime half is genuinely present.
  - The **header itself isn't shipped** in the SDK (only `shims/Reflection.h`), but
    it's **vendorable** from the matching Swift 6.3.2 compiler source via
    `LIB_SWIFT_PATH` — exactly what OAG's Linux build already does. Mechanical
    (a chain of internal headers), not a fundamental blocker.
  - **Remaining real risk = Swift↔C++ interop on wasm.** OAG *is* a C++↔Swift
    interop project; that interop is fragile even on Linux (OAG CI: Swift 6.2.4
    crashed the frontend on C++ interop; 6.3.2 fixed) and is **unconfirmed for
    wasm32** (no public statement; no wasm CI), with wasm32's 32-bit pointers a
    further wrinkle for the internal metadata layout.

**Net:** the deepest-seeming OAG blocker (`Metadata.h`) is **attainable** — the
metadata ABI is present in the wasm runtime and the header is vendorable. So the OAG
WASI port is a **bounded native-layer port** (syslog→`wasi:logging` + vendor the
Swift internal headers + get cxx-interop to build for wasm32), upstream-shaped and
*not blocked by metadata being absent*.

#### Swift↔C++ interop on `wasm32-wasip1` — PROVEN (2026-06-18)
The one genuinely-unverified link (OAG is a C++↔Swift interop project). Built a
minimal SwiftPM package — a C++ target (`struct Adder` with state + a `.cpp`-defined
method) and a Swift executable with `.interoperabilityMode(.Cxx)` constructing the
C++ object and calling its method — for `wasm32-wasip1`:
- **Compiles** (C++ → wasm object + cxx-interop Swift), **links**, and **runs under
  wasmtime**: `swift↔C++ on wasi: 40 + 2 = 42`.
- Only snag = a **packaging path quirk**: `libswiftCxx.a` ships in
  `.../usr/lib/swift/wasi/` but the static link searches `.../swift_static/wasi/`
  (which has `Cxx.swiftmodule` but not the archive) → `wasm-ld: unable to find
  library -lswiftCxx`. Fixed with `-Xlinker -L.../usr/lib/swift/wasi`. Not a
  fundamental gap.

#### OAG WASI port — actual attempt, layers cleared (2026-06-18)
Drove `swift build --swift-sdk swift-6.3.2-RELEASE_wasm` against OAG and cleared
blockers one by one (each a bounded, mechanical fix — no fundamental wall):
1. **`syslog.h`** → stderr/`static inline` shim on `-Xcc -I` (prod: → `wasi:logging`). ✅
2. **`swift/Runtime/Metadata.h` + `HeapObject.h`** → vendored from **`jcmosc/swift-runtime-headers`
   @ `release/6.3`** (a standalone, version-tagged package of exactly these Swift
   internal headers) on `-Xcc -I…/include`. Build got *past* `metadata.hpp`. ✅
3. **wasm32 ABI `static_assert`s** — `Utilities/HashTable.hpp` (+3 files) hardcode
   64-bit field offsets/sizes to lock binary-compat with **Apple's** AttributeGraph
   (`offsetof(HashNode,key)==8`, `sizeof(UntypedTable)==80`, …) → fail on wasm32
   (ILP32). Only **22 asserts in 4 files**, and the tree already has a
   `__POINTER_WIDTH__` macro to guard them (Apple-compat-only, irrelevant on wasm).
   Neutralized → `HashTable.cpp`/`Heap.cpp`/`Subgraph.cpp`/`OAGGraphContext.cpp`
   compile. ✅
4. **next wall:** `swift/ABI/Metadata.h` `#include <llvm/ADT/ArrayRef.h>` — OAG's own
   C++ engine (`OpenAttributeGraphCxx`) pulls the **full `swift/Runtime → swift/ABI
   → llvm/ADT`** header chain (header-only LLVM ADT, vendorable from LLVM source). 🔴

**Key structural finding:** OAG's Linux CI doesn't build its *own* Cxx engine — it
builds against the **Compute backend (`jcmosc/Compute`)**. Its `swift-runtime-headers`
submodule vendors **both** `include/swift/*` **and** `stdlib/include/llvm/*` (the
LLVM ADT headers), and its Package `-isystem`'s both — so the `swift/ABI → llvm/ADT`
chain that walled OAG's own Cxx **resolves cleanly**. So the **pragmatic OAG-on-WASI
path is the Compute backend.**

#### Compute backend WASI build attempt (2026-06-18) — got much further
Built `jcmosc/Compute` directly for `wasm32-wasip1` (syslog shim + Foundation
emulation + `-lswiftCxx -L`):
- **Cleared the walls OAG's own Cxx hit**: Platform + Utilities compiled with **no
  HashTable ABI-assert failures**, and `Metadata.cpp`/`MetadataVisitor.cpp`/
  `ContextDescriptor.cpp` compiled — the full `swift/Runtime`+`llvm`+metadata chain
  resolved via Compute's complete submodule. ✅✅✅
- **Next walls (all bounded):** (a) CF shim `SwiftCorelibsCoreFoundation/*` was
  `.when(platforms:[.linux])`-gated → enabling `.wasi` resolved it (Package edit);
  (b) `Platform/sha.h` `#include <openssl/sha.h>` → needs a SHA shim/stub (WASI has
  no openssl); (c) `Data/Page.h` `sizeof(IAG::data::page)==24` → Compute's *own*
  wasm32 ABI assert (same class — guard on pointer width).

**Verdict of the live attempt (two engines, many layers):** every wall is a bounded,
mechanical fix — POSIX shims (`syslog`, `openssl/sha`), vendored Swift+LLVM headers
(Compute's submodule, turnkey), Package platform conditions (`.wasi`), and a handful
of pointer-width ABI-assert guards. **No fundamental blocker exists**; Compute is the
right base, getting through the hard metadata/llvm layers with no header work.

#### ✅ Compute (the AttributeGraph engine) BUILDS for wasm32-wasip1 (2026-06-18)
`swift build --swift-sdk swift-6.3.2-RELEASE_wasm` of `jcmosc/Compute` → **`Build
complete!`** (compiles **and links** the whole engine). The complete fix-set, all
bounded/mechanical:
1. `syslog` → header-only shim (prod → `wasi:logging`).
2. `swift/Runtime`+`swift/ABI`+`llvm/ADT`+metadata chain → Compute's
   `swift-runtime-headers` submodule (vendors `include/swift` *and*
   `stdlib/include/llvm`, `-isystem`'d by its Package) — **turnkey, no hand-vendoring**.
3. CoreFoundation shim → flip its target/define from `.when([.linux])` to
   `[.linux, .wasi]` (one Package edit).
4. `openssl/sha.h` → header-only real SHA-1 shim.
5. **7** pointer-width ABI `static_assert`s (`sizeof(HeapObject)==16`,
   `sizeof(page)==0x18`, …) → guarded (Apple-binary-compat-only, irrelevant on wasm).
6. `uint` (BSD alias wasi-libc lacks) → `typedef unsigned int uint;` in the 2 files
   that use it.
That's the make-or-break dependency of the whole stack **building for WASI**.

#### Runtime proof — consumer build (corrected 2026-06-18; the "module cycle" was self-inflicted)
First attempt to *run* a minimal graph (`Graph()`/`Subgraph`/`Attribute(value:42)
.value`) via a consumer exe hit a clang-module cycle (`c++/v1/inttypes.h`/`errno.h`
`#include_next` ↔ `SwiftWASILibc` ↔ `std_inttypes_h`). **I first wrote this off as a
SwiftWasm toolchain bug — that was wrong.** Diagnosis (full include-trace +
shim audit):
- **Our POSIX/SHA shims are NOT involved** — the entire cycle is SDK-internal
  (`WASI.sdk/include/c++/v1/*`, `swift_static/shims/LibcOverlayShims.h`,
  `clang/include/inttypes.h`); `/tmp/oag-shims` appears nowhere, and none of our
  shims defines/shadows `inttypes.h`/`errno.h`.
- **It was self-inflicted:** my consumer target set `.interoperabilityMode(.Cxx)`,
  which drags **libc++** into the *consumer's* clang-module graph, where its
  `inttypes.h`/`errno.h` `#include_next` wrappers collide with the WASI-libc overlay
  module. **Compute's own API is consumable WITHOUT cxx-interop** — its test targets
  use `.enableExperimentalFeature("Extern")`, not `.interoperabilityMode(.Cxx)` (the
  Swift `Compute` module exposes a pure-Swift API over the C++ engine).
- **Fix = drop `.interoperabilityMode(.Cxx)` from the consumer.** The cycle vanished;
  the consumer **compiles and reaches the linker**.

Remaining = pure link resolution — **6 undefined symbols, all bounded**:
`__cxa_allocate_exception`/`__cxa_throw` (link `libc++abi.a`, in the SDK);
`swift::Demangle::makeSymbolicMangledNameStringRef` (in `swift_static/wasi/
libswiftCore.a`); `IAG::Graph::print_cycle` (defined only in Apple-only `Graph.mm`
→ stub on wasm); and `madvise`/`memfd_create` (POSIX — wasi-libc lacks them).

**The allocator gap — checked against WASI#304 + wasi-libc + the source (not assumed).**
`Sources/ComputeCxx/Data/Table.cpp` backs the graph data table with a growable VM
region. I first called this "needs an mmap port"; reading the source + the WASI state
makes it much smaller:
- **WASI#304**: full POSIX `mmap` (file-backed, `MAP_SHARED`, protection, `memfd`)
  is *deliberately not* coming to WASI — only an anonymous "read into memory" MVP;
  the caller owns address space (WebAssembly is one flat `memory.grow` space).
- **But wasi-libc ships anonymous mmap**: `libwasi-emulated-mman.a` (in the SDK)
  **defines `mmap`/`munmap`/`mprotect`**, honoring `MAP_PRIVATE|MAP_ANON`
  (malloc-backed). Just `-D_WASI_EMULATED_MMAN -lwasi-emulated-mman`.
- **And Compute already has a memfd-free path**: the **macOS** branch uses
  `mmap(nullptr, size, …, MAP_PRIVATE|MAP_ANON, -1, 0)` for *both* the 1 MiB initial
  region and `grow_region` — `memfd_create`/`ftruncate`/`MAP_SHARED` are **only the
  Linux `#else`**. So wasm should take the *macOS-shaped* branch, which lands exactly
  on wasi-libc's anonymous mmap. `memfd_create` is then **eliminated, not stubbed**.

So the real port is a small `#if defined(__wasi__)` branch in `table()` /
`grow_region()`:
- initial + grow → `mmap(MAP_PRIVATE|MAP_ANON)` (emulated mman);
- grow preserves handed-out offset-pointers by **`memcpy` old→new** (the macOS
  `vm_remap`/Linux-`MAP_SHARED` equivalent) and keeps the old mapping in
  `_remapped_regions` — no `mremap`/`MAP_SHARED` needed;
- `madvise` (page-reuse hint, absent from emulated mman) → **no-op** (advisory; on
  wasm the backing is malloc anyway); `mprotect` is only used under an off-by-default
  env and is provided regardless.

i.e. the "last functional gap" is **a ~one-branch allocator change using mmap that
wasi-libc already provides**, not a from-scratch allocator and not blocked by WASI's
lack of `MAP_SHARED`/`memfd`. (Reading first — per the project rule — turned "port
the allocator" into "take the existing anonymous-mmap branch.")

#### ✅ Implemented + the engine RUNS on wasm (2026-06-18)
Implemented the `#if defined(__wasi__)` allocator branch in `Table.cpp`
(`table()` + `grow_region()`: anonymous `mmap(MAP_PRIVATE|MAP_ANON)` + `memcpy`-grow;
`madvise`→no-op; `memfd_create` eliminated) and a guard on the Apple-only
`Graph::print_cycle` call. Resolved **all** link symbols — each a real, named fix:
- `__cxa_throw`/`__cxa_allocate_exception`: the wasi SDK ships **no exception
  runtime** and Compute has **zero explicit `throw`s** → compile its C++ with
  `-fno-exceptions` (implicit `std::` throws → abort). Cleared.
- `swift::Demangle::makeSymbolicMangledNameStringRef`: header/lib **inline-namespace
  skew** — `libswiftCore` defines it in `Demangle::__runtime::`, the headers
  declared plain `Demangle::`. Fix: `-DSWIFT_INLINE_NAMESPACE=__runtime` (the
  `__has_attribute`-gated `SWIFT_BEGIN_INLINE_NAMESPACE`). Cleared.
- `mmap`/`munmap`/`mprotect` → `-lwasi-emulated-mman`; `-lc++abi`, `-lswiftCore`.

Result: **`Build complete!` → the 64 MB component RUNS under wasmtime and executes
real engine code** — `Attribute.value` → `graphContext.internAttributeType` →
`IAGGraphInternAttributeType`. So Compute's allocator + metadata + graph paths all
work on wasm; the predicted allocator port was exactly right.

**The one remaining trap — the genuine final frontier:** `signature_mismatch:
IAGGraphInternAttributeType` — a wasm `call_indirect` type mismatch in the **Swift↔C++
closure-passing ABI**. C++ declares the `make_attribute_type` callback with
`__attribute__((swiftcall))` + `swift_context` (to match a Swift `() -> ptr`
closure's (fn,ctx) lowering); on wasm, **swiftc's closure lowering and clang's
`swiftcall` lowering emit mismatched wasm function signatures** for the indirect
call. This is a real, narrow Swift-on-wasm toolchain/ABI issue (swiftcall
consistency between swiftc and clang) — **not** the allocator (solved), headers
(solved), exceptions (solved), or anything in wandr. Mitigation (Compute-side,
bounded): replace the swiftcall-closure intern pattern with a plain **C-CC**
callback + explicit context (`IAG_SWIFT_CC(c)`) at the ~3 `internAttributeType`
sites, sidestepping `swiftcall` entirely — or an upstream swiftc/clang fix.

**Net:** the AttributeGraph engine **compiles, links, and runs on wasm32-wasip1**,
executing genuine engine code; the sole remaining issue is a precisely-located
Swift↔C++ `swiftcall` closure-ABI mismatch with a known bounded workaround. The
deepest dependency of OpenSwiftUI-on-wandr is no longer a question of *if* — it's
down to one characterized interop detail.

#### Root cause nailed + intern C-CC rework validated (2026-06-18)
Reworked the intern site to a C-CC callback and the run **advanced past it** — the
trap moved from `IAGGraphInternAttributeType` to the *next* closure site
(`IAGRetainClosure`). So the rework is correct, and the root cause is now proven:
- **Minimal test**: a plain C `int call_it(int(*f)(int),int)` calling a Swift
  `@convention(c)` callback **runs fine on wasm** (`42`). So swiftc↔clang
  function-pointer interop is NOT broken in general.
- **The real culprit is `@_silgen_name`** (Compute's binding style): it lowers a
  passed Swift closure with the **Swift** calling convention, whose wasm funcref
  type ≠ what clang's `call_indirect` expects → `signature_mismatch`. Neither the
  calling-convention attribute nor struct/pointer lowering was the cause (tested
  both — same trap); switching to a **header-declared, C-imported** entry with a
  plain `@convention(c)` thunk fixed the intern call.
- **Fix shape (intern)**: add a C entry `IAGGraphInternAttributeTypeC(graph,type,
  make,ctx)` to the ComputeCxx header (so Swift imports it with the C ABI, not
  `@_silgen_name`); split `intern_type` into a reusable `register_attribute_type`
  + a wasm `intern_type_c`; the Swift side passes a non-capturing `@convention(c)`
  thunk + a pointer to the (in-language) closure, which `intern_type` invokes
  synchronously. Plus the `__wasi__` allocator branch, `-fno-exceptions`,
  `-DSWIFT_INLINE_NAMESPACE=__runtime`, and the mman/c++abi/swiftCore link libs.

**Remaining (harder) class — stored closures.** `IAGRetainClosure` (and the rule
`update` path) **retain a Swift closure and have C++ call it LATER**, so the
synchronous in-language trick doesn't apply. The portable fix is a **Swift-side
registry + C-CC trampoline**: Swift stores the closure, hands C++ a plain
`@convention(c)` trampoline + token; C++ stores/calls the trampoline (C ABI); the
trampoline re-enters Swift and invokes the closure in-language. Mechanical but it
touches Compute's core `ClosureFunction` bridge and the ~18 `@_silgen_name`
closure sites — a bounded engine-side port, not a toolchain wall.

**Bottom line:** Compute on wasm runs real engine code; the intern closure path is
fixed and validated; finishing a full graph run is a known, mechanical extension of
the same C-import/trampoline pattern to the stored-closure sites — entirely
engine-side, no toolchain or wandr blocker.

#### ✅✅✅ FULL GRAPH RUN on wasm — `attribute.value = 42` (2026-06-18)
Built the registry + C-CC trampoline for the stored-closure (`IAGRetainClosure`)
path and the engine **executes a real graph on wasm32-wasip1**:
```
Compute AttributeGraph on wasi: attribute.value = 42
```
i.e. `Graph()` → `Subgraph(graph:)` → `Subgraph.current = …` → `Attribute(value: 42)`
→ read `.value` == **42**, end to end under wasmtime. The complete, source-grounded
fix-set (all bounded, no toolchain or wandr blocker):
1. **Allocator**: `#if defined(__wasi__)` branch in `Table.cpp` — anonymous
   `mmap(MAP_PRIVATE|MAP_ANON)` (wasi-emulated-mman) + `memcpy`-grow; `madvise`
   no-op; `memfd_create` eliminated.
2. **Exceptions**: `-fno-exceptions` (wasi SDK ships no exception runtime; Compute
   has no explicit `throw`s).
3. **Demangle**: `-DSWIFT_INLINE_NAMESPACE=__runtime` (header/lib inline-namespace
   skew on `makeSymbolicMangledNameStringRef`).
4. **Synchronous closure (intern)**: header-declared C-imported
   `IAGGraphInternAttributeTypeC` + plain `@convention(c)` thunk; `intern_type` split
   into `register_attribute_type` + `intern_type_c`; closure invoked in-language via
   a context pointer.
5. **Stored closure (retain/update)**: a `_UpdateBox` (Swift heap object) + a
   non-capturing `@convention(c)` `_updateTrampoline`; a non-refined
   `IAGRetainClosureC` C entry; `AttributeType::_update` made a plain C-CC pointer on
   wasm so C++ invokes the trampoline with the C ABI. The box's lifetime rides the
   existing `swift_retain`/`swift_release` (it's a Swift class).
6. **Link**: `-lwasi-emulated-mman`, `-lc++abi`, static `-lswiftCore`; `syslog`/
   `openssl-sha` shims; `uint` typedef; 7 guarded pointer-width ABI asserts;
   `print_cycle` guard.

**Root cause that drove 4–5**: `@_silgen_name` lowers a passed Swift closure with the
*Swift* calling convention, whose wasm funcref type ≠ clang's `call_indirect` —
proven by a minimal C-import `@convention(c)` callback running fine (`42`). The cure
is C-import + `@convention(c)` everywhere a closure crosses to C++.

**Scope honestly stated:** a *value* attribute is verified end-to-end. The
*rule-execution* path (where `_update` is actually invoked) is now **wired** for
C-CC (`_update` is plain C + the trampoline), but not yet exercised by a test; and
the other `@_silgen_name` closure sites (~18, e.g. `Subgraph`/`Rule`/`AnyAttribute`)
take the same mechanical treatment as a full OpenSwiftUI app reaches them. But the
**core question is now answered with a running engine**: OpenAttributeGraph executes
on wasm — the deepest dependency of OpenSwiftUI-on-wandr is real, not hypothetical.

#### ✅✅✅ REACTIVE RULE runs on wasm — `ruleAttr.value = 42` (2026-06-18)
Exercised a *computed* rule (not a constant): an input `Attribute(value: 21)` and a
`struct DoubleRule: Rule { @Attribute var input: Int; var value: Int { input*2 } }`;
reading `Attribute(DoubleRule(input:)).value` drives the graph to **update** the rule
attribute and yields **42** under wasmtime:
```
Compute rule on wasi: ruleAttr.value = 42
```
This exercises the **whole reactive path**: intern → retain → the C-CC `_update`
trampoline **actually invoked by C++** → the rule body runs → it **reads its input
dependency** (21) → computes → **writes the output value** → 42. So the stored-closure
trampoline is validated *in execution*, not just wired.

One more `@_silgen_name` boundary surfaced en route and took the same cure:
`IAGGraphSetOutputValue` (the rule's output write) — a refined `@_silgen_name`
function whose Swift-CC call mismatches clang's C ABI on wasm; fixed with a
non-refined C-imported `IAGGraphSetOutputValueC` (header-declared), exactly like
intern/retain. **This nails the general rule:** on wasm, every refined
`@_silgen_name` C-bridge call needs the **C-import** form (header-declared, called
with the C ABI); `@_silgen_name`'s Swift-CC lowering is what mismatches (proven
repeatedly: intern, retain, setOutputValue — and a minimal C-import callback runs
fine). The remaining ~15 `@_silgen_name` sites are the same mechanical conversion as
a full app reaches them.

**This is the headline:** OpenAttributeGraph — Apple's AttributeGraph reimplemented —
now runs a **reactive, dependency-tracked graph computation on `wasm32-wasip1`**. The
make-or-break layer of OpenSwiftUI-on-wandr is not just buildable but *functionally
executing* on wandr's substrate.

**Net:** the AttributeGraph engine **compiles + links for wasm32-wasip1** (bounded,
documented fix-set), and consuming it is a normal Swift import (no cxx-interop, no
module cycle). *Running* a graph is gated on porting Compute's `mmap`/`memfd`
allocator to wasm — engineering in Compute, not a toolchain or wandr blocker. (My
earlier "toolchain bug" claim is retracted.)

So **every gating toolchain mechanism for OAG-on-WASI is now demonstrated feasible**:
Swift C++ interop ✅ (compiles/links/runs on wasm), metadata ABI ✅ (present in the
wasm runtime), metadata header (vendorable), POSIX gaps (shimmable; logs →
`wasi:logging`). The remaining OAG WASI work is **bounded engineering** — wire all
its POSIX deps + vendor the Swift runtime-metadata header chain + the `-lswiftCxx`
link path — **with no fundamental blocker**. That materially upgrades the whole
OpenSwiftUI-on-wandr outlook: its deepest dependency (OAG on WASI) is now
toolchain-de-risked.

## Rendering seam decided + Option-B prototype device-verified (2026-06-18)

Explored how OpenSwiftUI renders before picking a rasterizer. Finding: OpenSwiftUI
builds **its own `DisplayList`** (`OpenSwiftUICore/Render/DisplayList`) — a concrete
item model (`.color`/`.image`/`.shape(Path,Paint,FillStyle)`/`.text` + effects
`opacity`/`clip`/`blur`/`shadow`/`transform`) — and draws it via **pluggable backends**
(`UIKitDisplayList`, `AppKitDisplayList`, `RenderBoxView`, GTK, `StdoutRendererHost`).

So **OpenRenderBox is just the *Apple* backend** — implementing `OpenRenderBox.render(
in:)` would be the wrong seam. The right move (Option B) is a **wandr DisplayList
drawing backend** parallel to UIKit/GTK, mapping DisplayList → our **CGContext →
wasi:canvas** (the renderer host is proven pluggable by `StdoutRendererHost`, not
Apple-bound). Also note: OpenRenderBox has **no complete third-party engine** (unlike
AttributeGraph's Compute) — its own `render(in:)` is a stub — which further rules out
that path.

**Prototyped Option B in isolation** (no full OpenSwiftUI build needed): a faithful
`DisplayList` mirror + a recursive `render(_:into: CGContext)` drawer, on the task-114
spike (`repros/swift-canvas-spike`, P4). ✅ **Device-verified (Pixel 2 XL)** — a scene
of content (color/shape/text) + nested effects (an `.opacity` inside a `.transform`)
renders correctly through CGContext→wasi:canvas, no traps. So OpenSwiftUI's DisplayList
maps 1:1 onto our shipped CGContext; the real `DisplayList.Item` types drop straight
into this drawer once OpenSwiftUICore builds on wasm.

**Updated critical path to `eleev/swiftui-2048`:**
1. ✅ Compute engine on wasm (`harryzz/Compute`, reactive `42`)
2. ✅ OAG-Shims + Compute backend on wasm (reactive `42` via the API OpenSwiftUI uses;
   `harryzz/Compute` branch `wasm32-wasip1-osp`)
3. ✅ Rendering seam decided + the wandr DisplayList→CGContext backend prototyped &
   device-verified (Option B)
4. 🔲 **Port OpenSwiftUICore/OpenSwiftUI to wasm** — the big remaining lift (uses #2 +
   OpenCoreGraphics); then slot the real DisplayList types into the #3 drawer + a
   `WandrRenderer` host (like `StdoutRendererHost`)
5. 🔲 `eleev/swiftui-2048`

## Scope: the OpenSwiftUICore/OpenSwiftUI wasm port (step 4) — 2026-06-18

Measured from the source. This is the **dominant remaining effort** of the whole stack.

### Size
- **OpenSwiftUICore: ~121k LOC / 557 files** (the engine: View/layout/state/DisplayList).
- **OpenSwiftUI: ~31k LOC / 222 files** (the app layer: App/Scene + UIKit/AppKit glue).
- ~150k LOC total — an order of magnitude larger than Compute (~5k) or OAG.

### The key de-risk: it already builds non-Darwin
There's a **`ubuntu.yml` CI** — OpenSwiftUI's non-Darwin path compiles on Linux. So this
is a **Linux→wasm delta** (the proven Compute/OAG/OCG playbook), NOT a from-scratch
off-Apple port. The 144 `canImport(Darwin)` guards already have working `#else` branches
(Linux builds), so wasm ≈ Linux-non-Darwin for those.

### Dependency tree (each needs a wasm build)
- ✅ OpenAttributeGraph (Shims + Compute backend) — done (`harryzz/Compute@wasm32-wasip1-osp`)
- ✅ OpenCoreGraphics (CGContext over wasi:canvas) — done (`harryzz/OpenCoreGraphics@wasm32-wasip1`)
- 🔲 **OpenRenderBox** — we render via our own DisplayList backend, but OpenSwiftUICore
  *imports* `OpenRenderBoxShims` for a few DisplayList types (`ORBDisplayListContents`,
  `ORBPath`). So it must **compile** on wasm (render(in:) can stay a stub — we never call it).
- 🔲 **OpenObservation** (`@Observable`) — wasm build.
- 🔲 **OpenCombine** (`ObservableObject`/`@Published`) — wasm build (mature pure-Swift).
- 🔲 swift-log / swift-crypto / swift-numerics — pure Swift, expected to build ~as-is.
- **Host-side only (no wasm port):** swift-syntax + the OpenSwiftUI **macros** run at build
  time on the host; SymbolLocator is a build tool. DarwinPrivateFrameworks = Darwin-only, excluded.

### wasm-specific delta (the proven patterns)
- **~70 `@_silgen_name`** → `@_extern(c)` (mostly non-generic, so clean; struct-returns
  need the out-param-wrapper variant, generics need a thin wrapper — both already done for OAG).
- **14 `os(WASI)`** stubs → un-stub (like OAG's 4 — old SwiftWasm 5.9.1 bugs, fixed on 6.3.2).
- **Foundation + ICU** link (heavy; already wired for the OAG-Shims build).
- **Renderer:** bring our validated **DisplayList→CGContext backend** (Option B) + a
  `WandrRendererHost` modeled on `StdoutRendererHost` (the proven pluggable host). Set
  `buildForDarwinPlatform=false`, no `RENDERBOX`/`RENDER_GTK`.

### Phased plan
0. **Dep forks on wasm**: OpenRenderBox(Shims, compile-only) + OpenObservation + OpenCombine
   (+ confirm swift-log/crypto/numerics build); macros build for host.
1. **OpenSwiftUICore compiles on wasm**: build config + 70 `@_extern(c)` + 14 un-stubs +
   Foundation/ICU + iterate compile errors (the Linux path is the guide).
2. **OpenSwiftUI (app layer) compiles on wasm**: the View/App/state subset.
3. **WandrRendererHost** + wire the real `DisplayList.Item` types into the Option-B drawer.
4. **Smoke test**: a hand-written `Text`+`@State`+`Button` view renders on device.
5. **`eleev/swiftui-2048`** runs.

### Effort / risk
**Multi-week** (the largest phase by far). De-risked by: Linux CI (the path compiles),
every wasm technique already proven (extern-c, Foundation link, un-stubs, our renderer +
engine done). Main *unknown*: whether OpenSwiftUI's non-Darwin View/render path is as
complete as it is *buildable* (OAG's engine compiled but was a stub — watch for the same
in the render/layout paths). Recommend tackling phases 0→1 first and re-assessing at the
first OpenSwiftUICore wasm build.

### Phase 0 — dependency libs on wasm: ✅ DONE (2026-06-18)
All build on `wasm32-wasip1` with the swift-6.3.2 SDK:
- **OpenCombine** — builds **unmodified**. No fork needed.
- **OpenObservation** — builds **unmodified**. No fork needed.
- **OpenRenderBox** (compile-only; we render via our own DisplayList backend) — builds
  with **no source edits**, via build config: `OPENRENDERBOX_LIB_SWIFT_PATH=<a
  SwiftCorelibs/include with CoreFoundation+dispatch headers>` (reused the OAG fork's)
  + the dispatch shim on `-Xcc -I` + `-fno-exceptions`/emulation flags. Its Cxx engine
  only needs CF (5×) + dispatch (1×) headers — no allocator (lighter than OAG/Compute).
- swift-log / swift-numerics — pure Swift, expected to build as-is (validate in phase 1).
- **Risk flagged for phase 1: `swift-crypto`** (BoringSSL C/asm — historically hard on
  wasm). It's a transitive OpenSwiftUI dep; may not be on the render path. Test when
  OpenSwiftUICore pulls it; if it blocks, check whether the wasm path actually uses it.

So phase 0 needed **zero forks** — these three build from upstream with build flags only.

### Phase 1 — first wall (2026-06-18): Dispatch/GCD on wasm
Set up the sibling-deps build (OAG-Shims+Compute backend, OpenRenderBox/OpenObservation
/OpenCombine wasm, upstream OpenCoreGraphics for the Core compile) and attempted
`swift build --target OpenSwiftUICore --swift-sdk …wasm`. First wall:

**No libdispatch/GCD on wasm** — `DispatchQueue`/`DispatchTime`/`OperationQueue`/`Timer`
+ OpenCombine schedulers (`SchedulerTimeType`/`DataTaskPublisher`) not in scope. The
wasm SDK ships no Dispatch module (Linux has swift-corelibs-libdispatch; wasm doesn't).

**Bounded:** concentrated in **3 files**, all on the async/animation path:
- `Render/DisplayList/DisplayListViewRenderer.swift` (1 `DispatchQueue` — async render)
- `Render/DisplayList/CAHostingLayer.swift` (10 `Timer` + 2 `DispatchQueue` — CALayer
  hosting + animation ticks)
- `Animation/Animation/AnimationListener.swift` (`DispatchTime`/`DispatchQueue` — anim timing)

So it's the **scheduling/animation** surface, not pervasive. Fix options:
1. **Minimal single-threaded Dispatch shim** (wasm is single-threaded + wandr has a frame
   loop): `DispatchQueue.main.async`→enqueue-to-next-frame, `asyncAfter`/`Timer`→driven by
   `on_frame` nanos, `DispatchTime`→monotonic clock. Reusable; the right foundation for
   `withAnimation`/`Timer` (which swiftui-2048 uses).
2. Guard the 3 files' Dispatch paths on wasm (faster; static render only, no animation).

Recommend (1) — a small `wasm-dispatch` shim driven by the frame loop. **More walls
expected deeper** once these compile (the build only reached the first target's errors);
this is the first of likely several, but the theme (Dispatch/scheduling) is now known.

### Phase 1 — in progress: walls cleared + remaining (2026-06-18)
Iterating the OpenSwiftUICore wasm build. The blockers are **entirely the Foundation
concurrency/threading/platform substrate** — *not* SwiftUI's View/layout/render logic
(zero errors there so far, the encouraging signal: the framework code looks portable
once the substrate is shimmed).

**Cleared so far** (all in `/tmp/OpenSwiftUI`, to fold into an OpenSwiftUI fork later):
- **Dispatch/GCD**: added `OpenSwiftUICore/Util/WasmDispatchShim.swift` (single-threaded
  `DispatchQueue.main`/`DispatchTime`, compile-focused) + guarded `import Dispatch` in
  `AnimationListener.swift` (`#if canImport(Dispatch)`).
- **OpenCombineFoundation**: gated its dep off non-Darwin in `Package.swift`
  (`addOpenCombineSettings`) — Core only imports `OpenCombine`, never the Foundation
  bridge (which needs URLSession/OperationQueue/os_unfair_lock — absent on wasm).
- **dladdr**: guarded `OpenSwiftUI_SPI/.../OpenSwiftUI_CSymbols.c` (`#if defined(__wasi__)`
  → NULL; no dynamic linking on wasm).
- **platform `#error`**: added `canImport(WASILibc)` branches to `StandardLibraryAdditions
  .swift` (only ~4 files lacked them — upstream already added WASILibc to most).

**Remaining (~8 files, the threading/observation layer)** — needs a wasm threading shim:
`ThreadUtils` (Thread + `pthread_key_create/get/setspecific` TLS), `RunLoopUtils`
(`RunLoop.perform/add/.common`), `TimerUtils`, `MainActorUtils`, `ObservationUtils`,
`StateObject`, `ObjectLocation`, `AttributeInvalidatingSubscriber`. Fix = a single-
threaded shim layer (Thread→main, pthread-TLS→a global, RunLoop→frame-loop/stub,
MainActor→trivial) — bounded (~8 files), mechanical, same playbook.

**Also confirmed:** swift-crypto/BoringSSL **compiles on wasm** (the flagged risk is
passing). Net: phase 1 is a substrate-shim grind, not a SwiftUI-logic rewrite — the
View/render code (where our Option-B DisplayList backend plugs in) hasn't thrown an error.
