---
name: reference_openswiftui_custom_path_wandr
description: "How custom Path construction + stroking works on OpenSwiftUI-on-wandr (WASI): the wandrElements storage case, arc→bézier, strokedPath-as-fill, renderer SVG"
metadata: 
  node_type: memory
  type: reference
  originSessionId: efb9ba77-bb47-4ab5-bbac-3dcd59e2771e
---

**Context:** Upstream OpenSwiftUI backs `Path` by ORBPath (OpenRenderBox) / CGPath, both UNIMPLEMENTED
off-Apple — so the entire mutable element API (`move`/`addLine`/`addArc`/`addCurve`/`closeSubpath`),
`forEach`, and `strokedPath` all `_openSwiftUIUnimplementedFailure()` and trap at RENDER time. Built-in
shapes (Rectangle/RoundedRectangle/Circle/Ellipse/Capsule) render because the wandr renderer reads
`Path.storage` directly (never via ORBPath). Custom `Shape`s that build a path element-by-element
(e.g. a `Pie`) and any `.stroke`/`.strokeBorder` had no working path — surfaced by the Memorizwift
(Stanford Memorize) portability test. 2048/calculator never hit it (built-ins only).

**The implementation (2026-07-19):** a pure-Swift element buffer, gated `#if !canImport(CoreGraphics)`
so Darwin is untouched.
- `Path.Storage.wandrElements([Element])` — new storage case (Path.swift). Element = the public
  `Path.Element` (move/line/quadCurve/curve/closeSubpath). Exhaustive storage switches that needed the
  case: `retainRBPath` (→ unreachable, renderer never uses ORBPath), `isEmpty`, `boundingRect`. All
  OTHER storage switches have a `default` (DisplayListViewModel clip/roundedRect, wandrSVGPath).
- `Shape/Path+WandrElements.swift` — all geometry: storage→elements (`wandrElementList`), append,
  `wandrLastPoint` (= CG "current point"), arc→cubic-Bézier (`wandrArcElements`, ≤90°/seg, kappa
  0.5522847498), ellipse/roundedRect/rect element builders, transform mapping, `wandrBounds`,
  curve flattening, and `wandrStrokeOutline` (stroke as a FILLABLE outline = per-segment quads +
  square joins, since the renderer only fills — approximation, no miter/round joins, fine for thin
  borders).
- Path.swift method bodies: each `#if canImport(CoreGraphics) <trap> #else <wandrAppend...> #endif`.
  `strokedPath` returns `.wandrElements(wandrStrokeOutline(...))`. `forEach` iterates `wandrElementList`.
- `WandrDisplayListRenderer.wandrSVGPath` gained `case .wandrElements` → `wandrElementsSVG` (emits SVG
  M/L/Q/C/Z, points mapped through the transform). This is how a custom path reaches wasi:canvas.
- Also implemented `RoundedRectangle._Inset.path(in:)` (was unimplemented) — needed by `.strokeBorder`
  (inset then stroke): inset rect + reduced corner radius, delegating to `RoundedRectangle.path`.

**Gotchas:** the case MUST be gated `#if !canImport(CoreGraphics)` — an ungated CoreGraphics-named
module (or case handling) is what broke OpenSwiftUICore earlier (see below). Arc sweep direction:
`clockwise` decreases angle, else increases; Pie passes its own flag. Stroke joins use square caps
(slight overfill) — imperceptible at 2pt.

**Two more render fixes from the same port (SHIPPED, commit `1b07d8b1`):**
- **Flat `rotation3DEffect` displaced content off-screen.** A card flip's 2D projection mirrors the
  content about its LOCAL origin — the anchor-CENTER pivot is not carried in the ProjectionTransform
  here — so a face-down (180°) card landed at negative X and vanished (looked like "cards don't
  render"). Fix: in `wandrApplyProjection`, when there's no real perspective (m13≈m23≈0), draw the
  flip IN PLACE instead of applying the displacing mirror (the flipped face is hidden-by-opacity or
  a symmetric back). Diagnosing this needed a temporary DBGSHAPE log of each fill's rect+RGBA —
  the colors were correct all along, the RECTS were negative.
- **`.minimumScaleFactor` shrink-to-fit.** Implemented in `StyledTextContentView.sizeThatFits`:
  scale down ONLY when natural size exceeds the PROPOSED size, bounded by the factor; the renderer
  re-derives the same scale from frame-vs-natural. ⚠️ An earlier attempt keyed the clamp off
  `min(frame.width, frame.height)` and REGRESSED the calculator — a short string's frame is
  legitimately narrow (≈count×size×0.6), so a single-digit "0" got shrunk while 2+ digits rendered
  normally. Also: the natural-width estimate is now PER-CHARACTER (ASCII ~0.6em, non-ASCII/emoji
  ~1.15em) and SHARED between layout and renderer — a flat 0.6em made an emoji's box far too narrow
  so it overflowed to the right.

**Process lesson:** shared-framework changes need a regression pass over the other guests before
committing — 2048 + calculator + the new app. The calculator regression above was caught only by
re-running it. See [[feedback_humility_proven_vs_guessed]].

**RELATED — the canImport(CoreGraphics) trap:** do NOT create an apple-compat module literally named
`CoreGraphics`. It flips `#if canImport(CoreGraphics)` TRUE across the WHOLE graph, activating
OpenSwiftUICore's Apple-only CG paths (full CGContext/CGColorSpace/CGDataConsumer) that WASICanvas
lacks → build breaks. Stock code that `import CoreGraphics` for CGRect/CGPoint should import Foundation
instead. See [[reference_swift_openswiftui_wandr]].

**Build cache gotcha:** adding source files to a path-dependency (apple-compat) that SwiftPM has
cached may NOT trigger a rebuild; a stale `.build/.../Modules/<Mod>.swiftmodule` also keeps
`canImport(<Mod>)` true after you delete the target. Fix: remove the module's `.build`/`Modules`
artifacts + `.build/build.db` + `description.json` + `workspace-state.json` to force a re-plan
(keeps OpenSwiftUICore .o's, but OpenSwiftUICore itself re-checks). Related: [[reference_openswiftui_scroll_list_todo]].
