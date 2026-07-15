---
name: reference_openswiftui_gestures_offapple
description: OpenSwiftUI off-Apple gesture routing/arbitration on wasm (taps+drag) — the ViewTransform buffer-off-by-one crash, specificity+co-delivery binding, granular reset; plus remaining eleev gaps
metadata:
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

Making OpenSwiftUI's gesture path work off-Apple (wasm32) on the eleev 2048 app (Linux). Taps
(hamburger/reset open overlays), board swipe (move/merge), and tap-anywhere-to-dismiss all work,
crash-free. Builds on [[reference_openswiftui_sfsymbols_rendering]] (icons) and
[[reference_swift_openswiftui_wandr]]. Host input path: `runtime/wandr-host/src/input.rs`
`dispatch_pointer_routed` now LOGS `on_pointer TRAP: …` (was `let _ =` swallowing guest traps —
essential for diagnosing guest panics that otherwise surface only as later "cannot enter
component instance" on `render_frame`).

## AUDIT: is the port mess upstream OpenSwiftUI's or wandr's?
`repros/swift-canvas-spike/AUDIT-upstream-vs-wandr.md`. HEADLINE (from OpenSwiftUI's OWN README):
it is **"early development", "DO NOT use in production"**, and its "Current supported feature" list says
**"Text is not supported yet"** (+ non-color fills unimplemented `ShapeStyleRendering.render(style:)`
`_openSwiftUIUnimplementedFailure`; gesture routing is an upstream `GestureContainerFeature [TODO]`;
AppKit/UIKit "partly implemented"). So **unmodified OpenSwiftUI would NOT render eleev 2048 on Apple
either** (2048 is ~half text). The port is "finish OpenSwiftUI enough to run a real app," not "fix our
mess on top of a working OpenSwiftUI." The ViewTransform buffer crash + Bundle.main were UPSTREAM bugs we
FIXED (ViewTransform.swift is an upstream WIP file). Genuinely-ours churn: the gesture routing + modal
`.offset` hitFrame. Also: Monterey (2016 MBP, MacBookPro13,2) maxes at macOS 12 → can't build current
OpenSwiftUI (Swift 6.2 compiler RUNS on Monterey, but SwiftPM needs macOS-13 Foundation ABI). Modern
Mac / GitHub Actions macOS runner needed for the Apple baseline.

## FULL HANDOFF DOC (read this first for a new session)
`repros/swift-canvas-spike/HANDOFF-eleev-openswiftui.md` — complete build/run/deploy steps, everything
DONE, all REMAINING problems with root causes, key files, and the tree state. This memory is the short
version; the handoff is authoritative.

## SESSION 2026-07-15 — transform-aware + content-shape hit-testing (all device-verified)
Fixed §0 gesture bugs. All in `OpenSwiftUICore/Event/{Event/EventBindingManager.swift, Gesture/GestureViewModifier.swift}`.
- **hitFrame was the flat LAYOUT frame, ignoring render transforms.** `GestureFilter` set
  `hitFrame = CGRect(animatedPosition(), animatedCGSize())`, but `.offset`/`.scaleEffect`/`rotation3DEffect`
  are GeometryEffects that RESET the descendant's `position` to zero and put the placement in
  `inputs.transform` (`GeometryEffect.swift:134-136`). Fix: capture `inputs.transform` on the responder
  (`GestureFilter` `@Attribute transform` → `GestureResponder.viewTransform`) and map the global hit point
  to local via `viewTransform.convert(.localToSpace(.global), point:)` before `hitFrame.contains`.
  **DIRECTION GOTCHA:** use `.localToSpace(.global)`, NOT `.spaceToLocal` — GeometryEffectTransform appends
  effects with `inverse: true`, so `inputs.transform` already encodes global→local; `.spaceToLocal`
  double-inverts. Confirmed by the canonical `GeometryProxy.convert(globalPoint:to:)` (GeometryReader.swift).
  Fixes: header-swipe-moves-board, offset modal buttons not tappable, parked-overlay phantom frames.
- **Content-shape hit-testing (swipe only on the drawn board, not the greedy slot).** SwiftUI hit-tests
  where content is DRAWN, not the layout frame; a board centered in a greedy `GeometryReader` is hittable
  only over the square (it has a `Rectangle` fill), not the transparent padding. wandr's flat "Approach A"
  hitFrame = full slot. Fix: `GestureFilter` captures the content `@OptionalAttribute contentDisplayList`
  (`outputs.preferences.displayList`), unions its **top-level** `DisplayList.items[].frame` (do NOT recurse
  into `.effect`/`.states` — sublists are in the item's PRE-transform space, e.g. a `.position`-ed board's
  children sit un-centered at origin) → `GestureResponder.contentBounds`. Hit test uses
  `hitRegion = hitFrame ∩ contentBounds` (fallback hitFrame). Verified content bounds = the centered
  414² square, in the same local space as the mapped point. No regression to buttons (filled backgrounds
  fill their frame; icon buttons still fine).
- **Compute ABI crash surfaced by the fix:** once Settings/About navigation worked (tap now lands on the
  menu item), constructing `settingsView` (`List{Section{…}}`) hit `withUnsafeTuple(IAGTupleWithBuffer)` →
  wasm `signature_mismatch` (Swift lowered 6 args, C++ 4). Fixed in the Compute fork — see
  `[[reference_compute_wasm_abi_signature_mismatch]]`.

## THE crash (root cause of every "second interaction freezes / cannot enter component instance")
`ViewTransform.forEach(inverted:)` non-inverted branch (Layout/View/ViewTransform.swift ~271) had an
OFF-BY-ONE: it wrote `head` at buffer[0] then the fill loop ALSO started `index=0`, overwriting
head and leaving the LAST slot UNINITIALIZED. `bufferPointer.reversed()` then read that garbage
`AnyElement`, retaining a bogus pointer → "uninitialized element" / OOB trap the FIRST time a
MULTI-element transform was hit-tested (e.g. an OPEN side menu contributes offset + affine
elements; `EventListenerPhase → convert(point:) → forEach`). A guest trap during `on_pointer`
POISONS the whole component (`may_enter`=false) → every later `render_frame` traps "cannot enter
component instance". FIX: fill buffer[0..<depth] exactly once + `deinitialize`. This was misdiagnosed
for many iterations as a co-delivery / `producesVoidValue` / animPending problem — it was none of
those. Lesson: when a guest call is `let _ =`-swallowed, LOG it before chasing ghosts.

## Also fixed off-Apple
- `ViewTransform.convert(point:)` affine INVERSE was `#else`-unimplemented (assumed the shim lacked
  `.inverted()`). `OpenCoreGraphicsShims.CGAffineTransform.inverted()` IS a real 2×3 inverse — just
  use it, so hit-testing maps points correctly through a non-translation transform (open menu tilt).
- `ViewTransform.appendProjectionTransform` non-affine case: skip (renderer already drops rotation3D)
  — defensive, the `ProjectionTransformElement.forEach` witness may not be emitted on wasm.

## Gesture arbitration (EventBindingManager, no full gesture graph)
- `GestureContainerFeature.isEnabled == true` → geometric bind by `hitFrame` (gesture's global layout
  frame; `.offset` is a render transform, NOT in the layout frame). Bind the SMALLEST-area gesture
  containing the point (tightest interactive target) — a button's 48×48 tap beats a full-screen drag.
- CO-DELIVERY: also bind the smallest TAP (`producesVoidValue` = `ContentGesture.Value == Void`), so a
  click self-selects the tap even where a tighter continuous drag overlaps (tap-to-dismiss over the
  board's DragGesture). Both self-arbitrate by movement (tap=stillness, drag=translation). Process
  DISCRETE (tap) buckets FIRST.
- GRANULAR reset (the swipe-freeze fix): drop each responder's binding when THAT gesture goes
  terminal — NEVER blanket-reset the sequence. A co-delivered tap FAILS the instant a swipe moves;
  blanket reset (old `WandrRendererHost.didUpdate → reset()`) also dropped the still-active drag's
  binding, rebound it mid-gesture, lost its onEnded → eleev's `ignoreGesture` stuck true → swipes
  froze. Now `didUpdate` is a no-op; `sendDownstream` removes only terminated responders.
- Guest reactor MUST set `animPending=true` after every pointer event (WandrReactor.swift onPointer)
  or `withAnimation`-driven state changes never advance the animation clock (menu never slides).

## Rendering
- Single-word text (no space) must get effectively UNBOUNDED paragraph maxWidth in
  `WandrDisplayListRenderer` `.text`, else Skia breaks a long word mid-character (a tile's "16" →
  "1"/"6" once it slightly exceeds the estimated frame). Multi-word keeps +20% slack.

## `.allowsHitTesting(_:)` — implemented (the "global interceptor" fix)
Was a cosmetic no-op in the eleev SwiftUI shim (`repros/.../SwiftUI/PreviewAndModifiers.swift`), so
eleev's `.allowsHitTesting(!(modal||menu))` (SlideViewModifier) did nothing → the greedy background
board-drag intercepted swipes OVER an open menu and stole taps from menu items. Implemented for real
in OpenSwiftUI (appended to `Data/EnvironmentKeys/Enabled.swift` — a BRAND-NEW file in that target
was silently not compiled by SwiftPM, so it must live in an existing file): env key + `View
.allowsHitTesting` (`_EnvironmentKeyTransformModifier` AND-composing) + `_ViewInputs.allowsHitTesting`
Attribute. `GestureFilter` reads it → `GestureResponder.hitTestable`; `bindResponders` skips
`!hitTestable`. Removed the shim stub (shim `@_exported import OpenSwiftUI` re-exports the real one).
VERIFIED: swiping over an open menu no longer moves the board.

## `Bundle.main` crash — fixed
`Image("Icon")` (eleev AboutView) → `Image.init(_:bundle:)` did `.bundle(bundle ?? Bundle.main)`;
`Bundle.main` is a Foundation lazy global that TRAPS on wasm32 (`wasm unreachable`, resolves the
executable path — unimplemented under WASI), poisoning the guest. Fix: `Image.Location.bundle` is now
`Bundle?`; the inits use `Image._resolvedMainBundle` (`Bundle.main` on Apple, `nil` off-Apple).
Named-image loading is unsupported off-Apple anyway (resolves empty).

## Known-REMAINING eleev gaps (not yet fixed)
- **Menu items (Settings/About) don't navigate**: the tap DOES fire (menu closes via
  onMenuChangeHandler) but `FactoryContentView`'s `switch selectedView` doesn't visibly swap to the
  new view — a conditional-view update OR a blank-render (render(style:)/text) issue, not gestures.
- **Button ignores `.buttonStyle`**: OpenSwiftUI `Button` is implemented minimally as
  `label.onTapGesture(action)` — it does NOT apply the environment ButtonStyle. eleev's modal
  buttons use `.buttonStyle(FilledBackgroundStyle())`; without it the filled background never draws
  and the background-colored "New Game"/"Ok" text is invisible → game-over modal looks like a blank
  rectangle. Fix = implement ButtonStyle/PrimitiveButtonStyle resolution in Button.
- **Board-area swipe intermittently "freezes"** while a header-area swipe on the same greedy
  board-drag always works → suspect the board subtree's responders rebuild each tile-animation frame,
  dropping an in-flight gesture. Responder stability during animation — unfixed.
- **board-drag hitFrame is (0,0,460,734)** = greedy (includes the header), so swiping the upper area
  moves the board (commit c7a0a71b "bind swipe to the board square" didn't fully constrain it).
- Game-over modal dismisses by DRAGGING it down (`BottomSlidableModalModifier`), not tapping
  (`guard !hasGameEnded` disables the tap-dismiss by design).
