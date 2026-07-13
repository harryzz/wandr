# OpenSwiftUI `_ConditionalContent` view-list crash on wasm32 — root causes

Context: dropping the REAL eleev/swiftui-2048 app (`Sources/T2iles/`) onto OpenSwiftUI
crashed at render frame #0 inside `renderWandrAppOnce` (building the `CompositeView`
graph). GameLogic/Combine construct fine — the crash is pure OpenSwiftUI view-graph.

Named backtrace pinned it to `_ConditionalContent._makeViewList` →
`ConditionalTypeDescriptor.init` (an eleev `if/else` in a `@ViewBuilder`, built as a
view LIST). Two distinct wasm32-ABI bugs in the same path, both **hardcoded-64-bit**:

## Bug 1 — `Metadata.genericType(at:)` hardcodes 64-bit pointer size
`OpenSwiftUICore/Util/AttributeGraphAdditions.swift`

```swift
// BEFORE (64-bit only):
UnsafeRawPointer(rawValue).advanced(by: index &* 8).advanced(by: 16)...
// base 16 = 2 pointers (Kind + Descriptor), stride 8 = one pointer — both assume 8-byte ptrs.
```
On wasm32 pointers are 4 bytes, so the generic-argument vector is at offset `2*4=8`
with stride 4 — the old code read at `16 + index*8`, landing in garbage. Symptom:
`_ConditionalContent<True,False>` branch types came back NULL / word0==0.

Fix: derive offsets from `MemoryLayout<UnsafeRawPointer>.stride` →
`.advanced(by: (index + 2) * ptrSize)`. Correct on both 64-bit (16/8) and wasm32 (8/4).

## Bug 2 — metadata access-function relative pointer is called via @convention(c)
`OpenSwiftUICore/Runtime/ConditionalMetadata.swift` (`ConditionalTypeDescriptor.init`)

Upstream got `_ConditionalContent<T,U>.Storage` metadata by resolving the Storage
nominal descriptor's metadata-access-function relative pointer (`nominal.advanced(by:12)`,
read Int32, add) and calling it `@convention(c)`. On wasm code lives in the **function
table**, not linear memory, so that resolved "pointer" is a linear-memory address
call_indirect'd as a table index → `wasm trap: undefined element: out of bounds table access`.

Fix: get the Storage type from `_ConditionalContent`'s sole stored field
(`public let storage: Storage`) via runtime field reflection
(`metadata.forEachField`), which is arch-neutral. No raw code-pointer poking.

## Why our custom demo didn't hit this
The custom `OpenSwiftUIDemo` uses `if cond {…}` (no `else`) → `Optional`/`OptionalView`
(single generic arg) and/or the `_makeView` path; eleev uses `if/else` → two-arg
`_ConditionalContent` in the `_makeViewList` path, which is what exercises both bugs.

Both fixes verified: with them, the `_ConditionalContent` branches resolve to the real
eleev view types and the graph build proceeds into actual view-body evaluation. Diagnostic
scaffolding (`_wandr_cond_fputs`/`_wandr_type_name` in ProtocolDescriptor.{c,h}, the
leaf-conformance log guard) has been REMOVED — only the two fixes above remain.

## Progression after the two OpenSwiftUI fixes (each a distinct wasm gap)
3. **Bundle.main traps** — eleev's `PlistConfiguration(name:"Strings")` reads via
   `Bundle.main.path(forResource:)`; `Bundle.main`'s lazy init traps on wasm
   (swift-corelibs-Foundation). BRIDGED as a Store-class seam: excluded
   `Utils/Plist/PlistConfiguration.swift`, added `Sources/T2iles/WandrPlist.swift`
   (API-identical, reads `/assets/<name>.plist` via POSIX). Callers already `?? default`
   on nil, so absence is graceful.
4. **System-font resolution + font modifiers unimplemented on wasm** (FIXED, fork b4412d49).
   `canImport(CoreText)` is false on wasm, so the whole descriptor-producing path was
   `_openSwiftUIPlatformUnimplementedFailure()`. Root, not the design constant. Fixed by
   making the placeholder `CTFontDescriptor` carry the resolved traits (point size + weight),
   a text-style→size table (WandrWasmFontMetrics.swift), real `#else` branches for
   `resolveSystemFont`/`resolveTextStyleFont` + both `ResolvedTraits` inits, and a
   `resolveTraits` on `ModifierProvider`/`StaticModifierProvider` that applies `modify(traits:)`
   (the default `.init(resolve(...))` forced the unimplemented `modify(descriptor:)`, so
   `.weight()`/`.bold()` trapped). Custom demo used only `.system(size:weight:)` (SystemProvider's
   own resolveTraits) so it never hit this.

## ✅ RESULT: the real eleev/swiftui-2048 app RENDERS on wasm32
With all four fixes the real app (unmodified except the Audio + Bundle/Plist seams) builds its
full view graph and renders — frames #0.. ok=true, 0 traps. Two OpenSwiftUI fork commits:
81b68998 (conditional metadata) + b4412d49 (font path).

### ✅ Split-board — FIXED (fork 740a4681): `.offset`+`.position` composition
The board renders whole; the side menu hides off-screen. Two OpenSwiftUI bugs (both needed):
1. `_OffsetEffect` used a custom `_makeView` mutating `inputs.position` (OffsetPosition) —
   a no-op over a `.position()`/GeometryReader child that re-establishes its coordinate space.
   Removed the override → uses the generic GeometryEffect render-transform path; the offset
   now emits `.transform(.affine)` into the DisplayList and is applied by the walker.
2. That exposed `ViewTransform.convert` applying the WHOLE transform stack for a `.local`
   target. A view's `.local` frame is its own bounds and must exclude ancestor render
   transforms (`.offset` shifts `.global`, not `.local`). So `proxy.frame(in:.local).midX`
   picked up the offset (260→1080), and `.center(in:.local)` mis-placed the menu, cancelling
   the offset (1080−780=260, visible). Made local↔local conversion the identity → `.center`=260,
   offset −780 → −520 (hidden). Board unaffected (still x=26). Verified via draw-rect trace +
   Windows screenshot (whole board).

### (historical) original diagnosis of the split-board `.offset`+`.position` bug
The 4×4 board renders perfectly (tiles at x=38/152/266/380, uniform spacing — verified via
draw-rect dumps). The visual "split" is an **occluder**: eleev's side menu (`CompositeSideView`
→ `SideMenuView`, an 80×904 panel) renders **centered at x=260 over the board** instead of
hidden off-screen. It should hide via `.center(...)`(=`.position(260)`) then
`.offset(x: -(width + width/2))` ≈ **-780**. The offset never moves it. Traced precisely:
- GeometryReaders report correct sizes (520×1040), so the offset VALUE computes to -780.
- Upstream `.offset` = `OffsetPosition` (mutates `inputs.position`). But `SideMenuView`'s OWN
  inner `GeometryReader` **re-zeroes `inputs.position`** (establishes a local coord space,
  GeometryReader.swift:50), discarding the enclosing offset; then `.center` places at 260 in
  that zeroed space. So OffsetPosition is a no-op here.
- Switching `_OffsetEffect` to the SwiftUI-correct **render-transform** path (generic
  `GeometryEffect._makeView` → `DefaultGeometryEffectProvider` → `.transform(.affine(-780))`)
  DOES emit the -780 affine to the DisplayList (walker applies `.transform(.affine)`), but the
  menu fill still draws at x=260 across all 180 frames — i.e. the positioned menu content is
  **not nested under the offset effect** in the DisplayList (`GeometryEffectDisplayList` wraps
  `outputs.preferences.displayList`, but the `_PositionLayout` content lands in a separate/
  sibling list, likely an async-attribute timing issue). Also the walker DROPS
  `.transform(.projection)` and `.transform(.rotation3D)` (the menu's `.rotation3DEffect`), and
  `.allowsHitTesting` is a no-op stub — all contributing to "can't play".
- **Fix needed (deep):** make the GeometryEffect render-transform correctly nest a
  `.position()`/GeometryReader child's content under the effect in the DisplayList, + handle
  projection/3D-rotation/clip + real `.allowsHitTesting` in the walker. Scoped but non-trivial
  AttributeGraph/DisplayList work; all experimental changes reverted (fork = committed fixes only).

### Windows-only: DPI window-crop (separate, worked around)
`win.inner_size()` reports 520×1040 while the real client is 346×693 (÷1.5 DPI) because the host
process is DPI-unaware → softbuffer draws 520-wide into a 346-wide window (crop) + input mismap.
Worked around per-user via AppCompat registry (`HIGHDPIAWARE`); permanent fix = embed a
PerMonitorV2 manifest in the host build (Windows rebuild). Not the cause of the split (Linux
DPI 1.0 shows the identical split).

### Remaining polish (non-fatal)
- `ShapeStyleRendering.swift:203 render(style:) is unimplemented` warnings — gradient/complex
  ShapeStyle fills (eleev's RoundedClippedBackground) fall back; backgrounds may be flat.
- Font **design** (`.monospaced`) is dropped at the trait boundary — text renders at correct
  size/weight in the host's default face, not a distinct monospace face. Real monospace needs
  plumbing a face through the draw sink (`draw-glyphs` + a bundled TTF); the sink currently
  takes only a size.
- Interactivity (swipe/tap) on the real CompositeView not yet verified.
