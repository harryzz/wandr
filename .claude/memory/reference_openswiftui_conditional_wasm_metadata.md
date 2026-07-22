---
name: reference_openswiftui_conditional_wasm_metadata
description: OpenSwiftUI if/else view-list crash on wasm = two hardcoded-64-bit metadata-ABI bugs (fixed); + Bundle.main + font gaps
metadata: 
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

Dropping the REAL eleev/swiftui-2048 onto OpenSwiftUI (repros/swift-canvas-spike,
target `T2iles`, behind SwiftUI/Combine/AudioToolbox shims) crashed at render frame #0
inside `renderWandrAppOnce` building `CompositeView`. **Not Combine** (the user's OpenCombine
hunch was a red herring — GameLogic's `@Published`/`NotificationCenter.publisher` chain
constructs fine; see [[reference_observableobject_wasm_exclusivity]]). Root = OpenSwiftUI
building an `if/else` `@ViewBuilder` as a view LIST (`_ConditionalContent._makeViewList`
→ `ConditionalTypeDescriptor`). Fixed in the fork (commit 81b68998):

1. **`Metadata.genericType(at:)` hardcoded 64-bit ptr size** —
   `OpenSwiftUICore/Util/AttributeGraphAdditions.swift`. Was `.advanced(by: index*8).advanced(by: 16)`
   (base 16 = 2 ptrs, stride 8). wasm32 ptrs are 4B → vector at base 8/stride 4; old code
   read garbage → `_ConditionalContent` branch types came back NULL/word0==0. Fix: derive
   from `MemoryLayout<UnsafeRawPointer>.stride` → `.advanced(by: (index+2)*ptrSize)`.

2. **Metadata access-function relative pointer called @convention(c)** —
   `OpenSwiftUICore/Runtime/ConditionalMetadata.swift` `ConditionalTypeDescriptor.init`.
   Upstream got `_ConditionalContent<T,U>.Storage` metadata by resolving the Storage nominal
   descriptor's access-fn relative ptr (`nominal.advanced(by:12)`, read Int32) and calling
   it. On wasm code lives in the FUNCTION TABLE, not linear memory → that address
   call_indirect'd as a table index → `wasm trap: undefined element: out of bounds table
   access`. Fix: get Storage type from `_ConditionalContent`'s sole stored field
   (`storage: Storage`) via `metadata.forEachField` — arch-neutral, no code-ptr poking.

**Diagnosis technique that worked**: unstripped `component new` (keep names) → host run gives
a named wasm backtrace. `_typeName`/the Swift demangler ABORTS on the malformed metadata, so
read the simple name straight from the context descriptor's Name field (relative-direct
C-string @ desc+8) via a tiny C shim — NOT the demangler. Calling C from OpenSwiftUICore:
use the `import OpenSwiftUI_SPI` module path, NOT `@_silgen_name` (mislowers → wasm
`signature_mismatch`, same gotcha the existing `_OpenSwiftUI_conformsToProtocol` shim documents).

**Remaining walls after the two fixes** (the app now evaluates `CompositeView.body`):
- `Bundle.main` traps on wasm (swift-corelibs-Foundation). eleev's `PlistConfiguration`
  reads Strings.plist via it. BRIDGED as a Store-seam: excluded the file, added
  `Sources/T2iles/WandrPlist.swift` reading `/assets/<name>.plist` via POSIX (callers `?? default`).
- System-font resolution + font modifiers were unimplemented on wasm (`canImport(CoreText)`
  false → whole descriptor path = `_openSwiftUIPlatformUnimplementedFailure`). FIXED (fork
  b4412d49): placeholder `CTFontDescriptor` carries traits (size+weight); text-style→size
  table (WandrWasmFontMetrics.swift); real `#else` for resolveSystemFont/resolveTextStyleFont
  + both ResolvedTraits inits; `ModifierProvider.resolveTraits` applies `modify(traits:)`
  (default `.init(resolve())` forced the unimplemented `modify(descriptor:)` → `.weight()`/
  `.bold()` trapped). Font DESIGN (.monospaced) dropped at the trait boundary — renders at
  correct size/weight in the host's default face, not a distinct monospace face (real
  monospace needs a face plumbed through draw-glyphs; sink takes only size).

## ✅ RESULT: real eleev/swiftui-2048 RENDERS on wasm32
All 4 fixes → the real app (unmodified except Audio + Bundle/Plist seams) builds its full
view graph and renders (frames ok=true, 0 traps). Fork commits: 81b68998 (conditional
metadata) + b4412d49 (font path). Main repo: 38551aad, 27264e60. NOT pushed.
## ✅ Split-board FIXED (fork 740a4681): `.offset`+`.position` + `.local`
Real eleev 2048 renders the WHOLE board (Windows screenshot verified). The "split" was the
side menu occluding the center. Two OpenSwiftUI fixes: (1) `_OffsetEffect` removed its custom
`_makeView` (OffsetPosition, a no-op over `.position()`/GeometryReader) → uses the generic
GeometryEffect render-transform path so the offset emits `.transform(.affine)` and applies;
(2) `ViewTransform.convert` — local↔local is now identity (a view's `.local` frame is its own
bounds, excludes ancestor render transforms like `.offset`). Without (2), `proxy.frame(in:.local)`
picked up the offset → `.center(in:.local)` mis-placed the menu, cancelling the offset. Diagnosis:
one instrumented build dumping the DisplayList tree (offset -780 applied at line N, then a
`.center` effit frame at x=1040 → net 260) — do the FULL-TREE dump in ONE build, not per-datapoint.

## 🔲 Swipe / tap "can't play" — gesture-system feature (NOT started; deep)
Pointers reach the guest (reactor onPointer → wandrSendPointer → MouseEvent →
`host.eventBindingManager.send`), but eleev's DragGesture.onChanged never fires. Root: (a)
`EventBindingManager` binds ONE responder per pointer (structural first-gesture); real SwiftUI
dispatches to ALL gestures in the hit path and arbitrates — eleev stacks overlapping gestures
(swipe on FactoryContentView + ZStack `.onTapGesture` + side-menu drag) so ours picks the wrong
(full-screen) one and starves the swipe. (b) `GestureContainerFeature.isEnabled == true` but
gesture hit-frames use the LAYOUT frame ("Approach A", GestureViewModifier ~line 366 `hitFrame`),
ignoring render transforms → the off-screen (offset) side menu still has a full-screen hitFrame
and eats board input. Fix = geometry/transform-aware leaf responders + multi-gesture dispatch.

## 🔲 Missing hamburger/reset buttons — SF Symbols (NOT started)
eleev: `Image(systemName:"text.justify")` + `Image(systemName:"arrow.counterclockwise.circle")`,
`.resizable().scaledToFit().frame(48)`. `Image(systemName:)` → OpenSwiftUI NamedImageProvider;
resolves to `.image` DisplayList content which the WandrDisplayListRenderer walker DROPS (only
.color/.shape/.text/.flattened handled). Buttons exist+tappable, just invisible. Fix = wasm
systemName resolve → glyph (bundle an icon TTF e.g. Tabler, map names) OR emit `.text` glyph;
no clean shortcut (systemName is a runtime graph attribute, not visible at `_makeView`).

Polish remaining (non-fatal): ShapeStyle gradient fills unimplemented (flat backgrounds);
monospace not a distinct face.
Full walk: `repros/swift-canvas-spike/CONDITIONAL_WASM_FINDINGS.md`.
