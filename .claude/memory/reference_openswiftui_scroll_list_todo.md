---
name: reference_openswiftui_scroll_list_todo
description: "FUTURE TASK — replace the minimal off-Apple ScrollView + gesture-priority hack with the full ScrollGeometry-backed ScrollView + real List, per the designed OpenSwiftUI scaffolding"
metadata: 
  node_type: memory
  type: reference
  originSessionId: efb9ba77-bb47-4ab5-bbac-3dcd59e2771e
---

**Status (2026-07-18):** a MINIMAL scrollable container SHIPS and works (eleev/2048 Settings list
scrolls, audio checkbox reachable). See `[[reference_openswiftui_reactor_main_boot]]` for the port.

**What exists now (the shortcut):**
- `OpenSwiftUICore/View/Scroll/ScrollView.swift` — a hand-rolled minimal ScrollView: a custom
  `Layout` measures content height, `.clipped()` + drag-driven `.offset`, offset clamped via a
  `ScrollMetrics` reference-type side-channel (because `onPreferenceChange` isn't implemented here).
  Vertical only; NO momentum/fling, NO scroll indicator, NOT integrated with the scroll scaffolding.
- apple-compat `List` (`swift/apple-compat/Sources/SwiftUI/Containers.swift`) = `ScrollView{VStack}`.
- Gesture arbitration: scroll pan marked `.highPriorityGesture`; `EventBindingManager.bindResponders`
  changed to make priority the PRIMARY key (high-priority wins over lower regardless of area), area
  the tie-break. This works because the port's responder tree is FLAT — gesture nesting isn't
  preserved (`ViewResponder.parent`/`nextResponder` is NEVER wired; `isPrioritized`'s nextResponder
  walk is dead code; the tree is a flat `ViewRespondersKey` preference reduction). So depth/nesting
  can't decide scroll-vs-ancestor; priority does.

**THE PROPER VERSION (future task — architecture from the 2026-07-18 agent design):**
- Evolve `ScrollView` into a real scroll node backed by `ScrollGeometry`
  (`Layout/Geometry/ScrollGeometry.swift`, already "Complete", has `translate(by:limit:)` = the
  clamp/inset/RTL primitive) instead of ad-hoc `offset`/`ScrollMetrics`. containerSize=viewport,
  contentSize=measured content.
- Wire the existing scaffolding it was written for: publish `ScrollablePreferenceKey`
  (`View/Scroll/Scrollable.swift`), register `.scrollView`/`.scrollViewContent` named coordinate
  spaces (`Layout/CoordinateSpace/ScrollCoordinateSpace.swift`), two-way `.scrollPosition(_:)`/
  `(id:anchor:)` via `_GraphInputs.setScrollPosition` (`View/Scroll/ScrollPosition+Modifiers.swift`),
  service `ScrollTarget`/`ScrollStateRequest` for `scrollTo`, `onScrollGeometryChange`.
- Real `List` as a NEW view in OpenSwiftUI (`View/Scroll/List.swift`): `ScrollView(.vertical)` +
  lazy `ForEach`/`VStack` (use `IsInLazyContainer` so only visible rows realize), `Section`,
  separators, `listStyle`; retire the apple-compat `List`/`Section` shim once section/style/selection
  are covered.
- Full gesture arbitration (the deferred "PIECE 5" work, WASM-PORT-LOG.md): wire `parent`/
  `nextResponder` in the responder tree so `isPrioritized`'s ancestor/descendant walk works, add
  failure-requirement (`shouldRequireFailure`/`canPrevent`), and fix the single-responder-binding
  re-entrancy limit in `bindResponders` (co-delivering two gesture graphs traps "cannot enter
  component instance") so true `.simultaneousGesture` / nested scroll works. Also give
  `DragGesture.Value.velocity` real values (hardcoded `.zero` today) for fling/momentum.
- Polish: visual scroll indicator (a thin fading thumb sized by viewport/content, positioned by
  offset/maxOffset).

**Separate PREEXISTING bug (2026-07-18/19, VISUAL ONLY — one root cause, two symptoms):** a
`Toggle`/`CheckboxToggleStyle` does NOT redraw its check icon when its bound state changes on wandr —
the STATE is correct, but the visual stays STALE until an UNRELATED re-render (e.g. scrolling) forces
a repaint. Confirmed preexisting on device AND desktop, NOT from the scroll/List work.
- Symptom 1: eleev/2048 board-size radio buttons (`TileBoardSettingView`) render as if none selected
  (the latest tapped choice IS the real selection).
- Symptom 2: the audio-settings checkbox (`AudioSettingView`) keeps its old icon after toggling until
  you scroll a little.
Root cause is a reactivity/invalidation gap: a Toggle's `isOn`-binding change doesn't invalidate the
toggle's own body/visual → the icon isn't re-rendered on demand. eleev's app is UNMODIFIED → fix is
framework-side (`CheckboxToggleStyle`/`Toggle` re-render on binding change, i.e. the toggle body must
depend on `isOn` so a change dirties it). A future follow-up, low priority (visual only).
