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

**The 3 real gaps (all minor, all already known-shaped):**

1. **Line dash** (`CGContextSetLineDash`) — no dash field in `paint`. Guest
   pre-computes dashed path geometry (Flutter does dashing in-framework too).
2. **Offset+color drop shadow** (`CGContextSetShadowWithColor`) — `paint.blur`
   (mask-blur) has sigma+style but **no offset/color**; emulate by drawing the
   shape twice (offset blurred copy). Already a named deferral in the Flutter
   `dart:ui` mapping (`drawShadow`); `scene.layer.set-shadow-elevation` covers
   layer elevation shadows.
3. **Alpha-mask / text-as-clip** (`CGContextClipToMask`, text drawing mode
   `.clip`) — no alpha-mask clip verb; emulate via `save-layer` + `dst-in` blend.
   Niche.

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
