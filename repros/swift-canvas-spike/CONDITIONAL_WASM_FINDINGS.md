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

### Remaining polish (non-fatal)
- `ShapeStyleRendering.swift:203 render(style:) is unimplemented` warnings — gradient/complex
  ShapeStyle fills (eleev's RoundedClippedBackground) fall back; backgrounds may be flat.
- Font **design** (`.monospaced`) is dropped at the trait boundary — text renders at correct
  size/weight in the host's default face, not a distinct monospace face. Real monospace needs
  plumbing a face through the draw sink (`draw-glyphs` + a bundled TTF); the sink currently
  takes only a size.
- Interactivity (swipe/tap) on the real CompositeView not yet verified.
