# OpenSwiftUI on wasm — resume point (updated 2026-06-19, phase 3 in progress)

## ▶ START HERE (next session): multi-child views = MEMORY CORRUPTION
WORKING & committed on wasm32-wasip1: primitive + custom Views, **@State (reactive)**,
**Text**, single-type protocol conformance, **first DisplayList rendered**. (build/run:
`repros/openswiftui-wasm/probe`, `ANY_ATTRIBUTE_FIX=0` REQUIRED — see "build the probe" below.)
CURRENT WALL: `VStack { Color.red; Color.blue }` (any multi-child / `TupleView`) corrupts
memory on the TupleView/conformance path (single-child is clean). It manifests as a wild
function pointer → nondeterministic "undefined element" trap / hang, and **the failure point
moves with ANY code change** — so PRINT-INSTRUMENTATION CANNOT LOCALIZE IT (proven, commit
`44664f4f`). NEXT APPROACH (not print-probing): wasm memory sanitizer / `wasmtime` debugging,
or bisect the upstream ABI mismatch that corrupts memory only on the tuple path. Prime
suspects: Compute tuple metadata reflection (`TupleType.type(at:)` / `AGTupleWithBuffer` —
the latter is in the UN-FIXED bounded swiftcc set, would be a natural corruption source) or
the `unsafeBitCast(storage, to: (any View.Type).self)` existential write (View.swift:202)
doing a wrong-size/offset read/write on wasm32. Full detail in the "VStack/TupleView" section
below. Git tip = `44664f4f`; all patches match `/tmp` (openswiftui-phase1-wip + compute-wasm
+ oag-fork). Cheap-iteration harness for Compute-side ABI checks: `repros/compute-wasm/computerun` (~4s).

## 🎉 PHASE 3 FIRST PIXEL (2026-06-19): OpenSwiftUI renders a DisplayList on wasm
`WindowGroup { Color.red }` → built + run under wasmtime → **clean exit 0**:
```
OpenSwiftUI backend: stdout
surface: 640.0x480.0
display-list-version: 3
rendered:
  - fill x:0.0 y:0.0 w:640.0 h:480.0 #FF3B30FF      # Color.red, full surface
```
A real SwiftUI view, laid out by the AttributeGraph (Compute) engine, emitted as an
OpenSwiftUI `DisplayList`, end-to-end on wasm32-wasip1. This is exactly the input the
device-verified Option-B drawer (`repros/swift-canvas-spike/.../DisplayListRenderer.swift`)
turns into CGContext → wasi:canvas → Skia/EGL. **Phase 3's core unknown is resolved.**

The fix that delivered it: `exit(0)` immediately after `host.renderOnce()` in
`StdoutApp.swift` (`#if os(WASI)`) — the render always completed; the only trap was in
TEARDOWN (`GraphHost.deinit → Subgraph.forEach`, an arg-closure swiftcall wall), and the
abort was happening before stdout flushed. Exiting before the closure's locals deinit
sidesteps that wall AND flushes stdout. (a)+(b) were the same issue.

### ✅ @State + Text WORK (2026-06-19): onUpdate/onInvalidation wired as stored plain-C callbacks
`WindowGroup { ContentView() }` with `@State` renders on wasm, clean exit 0:
- `ContentView { @State count=7; Text("count \(count)") }` → renders (empty `rendered:`
  because the stdout renderer emits only fills, not `.text` — real glyphs = phase-4 drawer).
- **Visible @State proof** (this exact view HUNG before the fix):
  `Color.red.opacity(count > 5 ? 0.25 : 1.0)` with count=7 →
  `fill … #FF3B3040` (red, alpha 0x40 = 0.25). @State is stored, read in `body`, and its
  value flows through the reactive graph into the DisplayList. Deterministic (re-ran clean).
- THE FIX (in `compute-wasm.patch`): `onUpdate`/`onInvalidation` were no-op'd; now wired as
  STORED plain-C callbacks. C++ `Context` gets `_update_callback_c`/`_invalidation_callback_c`
  (plain fn+ctx) + `set_*_callback_c`, invoked plain-C in `call_update`/`call_invalidation`;
  `extern "C" AGGraphSet{Update,Invalidation}CallbackC` entry points; Swift `Graph.swift`
  boxes the closure (`_GraphUpdateBox`/`_GraphInvalidationBox`) + passes a non-capturing
  `@convention(c)` trampoline + `Unmanaged.passRetained` box ptr (lives for graph lifetime).
  Mirrors the fork's existing `_UpdateBox`/`AGRetainClosureC` rule-update pattern. Gotcha:
  the `*C` context param is non-optional (`const void*` w/o `_Nullable`) → trampolines take
  `UnsafeRawPointer`, not `UnsafeRawPointer?`.
- Earlier "custom View hangs" was a stale build; the real hang was @State, now fixed.

### 🔶 VStack/TupleView (2026-06-19): conformance wall cleared; new "undefined element" wall
- ✅ **`swift_conformsToProtocol` C-shim DONE** (in `openswiftui-phase1-wip.patch`): it's
  `C_CC` per Swift `RuntimeFunctions.def:1859` (NOT swiftcall — specialist was wrong), so
  the `@_silgen_name` call mislowered. Added `_OpenSwiftUI_conformsToProtocol` plain-C
  wrapper in `OpenSwiftUI_SPI/Util/ProtocolDescriptor.{h,c}` (forwards to the C_CC runtime
  symbol) + `TypeConformance.swift` `#if os(WASI)` routes through it. The conformance trap
  is GONE.
- ⛔ **NEW WALL (diagnosed): `wasm trap: undefined element: out of bounds table access`**
  in the **`DebugReplaceableView` type-eraser dispatch**. Hit by `VStack { Color.red; Color.blue }`
  (no @State/Text needed). Consistent trap (nondeterministic trap-vs-hash → a WILD/uninitialized
  function pointer: an OOB table index traps, a valid-but-wrong index hangs).
  Path (verified backtrace): `ViewGraph.updateOutputs → Subgraph::update →
  StatefulRule._update → DynamicViewContainer.updateValue` (`View/DynamicView/DynamicView.swift:73`)
  → `view.makeChildView()` where `view: V = DebugReplaceableView` →
  `DebugReplaceableView.makeChildView` (`DebugReplaceableView.swift:94`) →
  `storage.makeChildView` (virtual dispatch into `DebugReplaceableViewStorage<Content>`) →
  the type-erased indirect call to `Content`'s make-function is OOB on wasm (Content =
  `TupleView<(Color,Color)>`). `@_typeEraser(DebugReplaceableView)` is gated on
  `OPENSWIFTUI_SUPPORT_2025_API && compiler(>=6.2)` (NOT DEBUG) — so a release build won't
  avoid it. Single views go through DebugReplaceableView fine; only multi-child OOBs.
  - WORKAROUND TRIED & FAILED (reverted): disabling `@_typeEraser(DebugReplaceableView)` on
    wasm (View.swift) only changed the symptom (OOB trap → hang) — because `AnyView` ALSO
    routes through `makeDynamicView` → `DynamicViewContainer` (AnyView.swift:57). So the eraser
    CHOICE isn't the cause.
  - REAL ROOT (localized): the failure is the per-element **runtime-conformance witness
    dispatch** for `TupleView` content, NOT the eraser. `TupleView._makeView`
    (`View/TupleView.swift`) builds each child via `TypeConformance<ViewDescriptor>.visitType`
    (`View/View.swift:201`) → `unsafeExistentialMetatype` does
    `unsafeBitCast(storage, to: (any View.Type).self)` packing the witness table returned by
    `swift_conformsToProtocol` into an existential metatype, then calls `_makeView` through that
    witness. On wasm a witness-table function-pointer entry resolves to an OOB `call_indirect`
    (nondeterministic trap/hang = a wild funcref index). Single-view Content works (it doesn't
    take this runtime-conformance-per-element path); only TupleView does.
  - 🔑 **KEY FINDING (hypothesis-(a) instrumentation attempt): it's MEMORY CORRUPTION, not a
    static missing witness.** Adding `_wasmDiag` (unbuffered `write(2)`) prints at
    `DynamicViewContainer.updateValue` / `tupleDescription` / `visitType` produced ZERO output
    and made the binary hang IMMEDIATELY (before even the usual "HitTestBindingModifier
    unimplemented" warning). I.e. **the failure point moves dramatically with any code change**
    — the classic signature of a wild pointer from corrupted memory. Print-instrumentation
    CANNOT localize it (adding a probe relocates the crash). The "undefined element" trap vs
    hang nondeterminism is the same story. So the next approach must NOT be print-probing:
    use memory bisection / a wasm memory sanitizer, or find the upstream ABI mismatch that
    corrupts memory only on the multi-child (TupleView) path (single-child is clean). Prime
    suspect: tuple metadata reflection (`Compute TupleType.type(at:)`/`AGTupleWithBuffer`) or
    the `unsafeBitCast(storage, to: (any View.Type).self)` existential write, doing a
    wrong-sized/wrong-offset read/write on wasm32. (Instrumentation reverted; tree clean.)
  - HYPOTHESES for the real fix (need Swift-runtime-on-wasm investigation):
    (a) the witness table from `swift_conformsToProtocol` for tuple element types may be an
        un-instantiated generic pattern on wasm (entries are bad) — may need
        `swift_conformsToProtocolCommon`/a different lookup, or instantiating the witness;
    (b) the `unsafeBitCast(storage, to: (any View.Type).self)` existential-metatype layout or
        the funcref representation differs on wasm (witness fn pointers are table indices);
    (c) `ViewDescriptor.tupleDescription`/`TupleType` element enumeration yields wrong
        conformances on wasm. Next: instrument `TypeConformance.visitType`/`tupleDescription`
        to print the element type + witness, and check whether `_makeView` via the witness is
        the exact OOB call.
  - `VStack+@State+Text` HANGS = the valid-but-wrong-funcref variant of the same wild pointer;
    same root, clears together.
- Bounded Compute swiftcc set still pending IF hit (would TRAP, not seen yet):
  `AGGraphReadCachedAttribute`, `AGGraphSearch`, `AGTupleWithBuffer`, `AGTypeApplyEnumData`/
  `MutableEnumData` (validate each in the 4s harness `repros/compute-wasm/computerun`).

### NEXT (other)
- `Subgraph.forEach`/`AGSubgraphApply` `*C` variant for clean teardown (currently sidestepped
  by `exit(0)` in StdoutApp before deinit).
- PHASE 4: wire the real `DisplayList` into the Option-B CGContext drawer → device. The
  stdout renderer not emitting `.text` is fine — the CGContext drawer handles `.text`.

### (historical) reading `@State` HANGS — root = onUpdate/onInvalidation no-op (NOW FIXED above)
Progress toward Text+@State (all verified by running the probe):
- Custom `View` with a reactive `body` **renders fine** (the earlier "custom View hangs"
  was a STALE incremental build — confirmed `ContentView { Color.red }` renders 3/3).
- Cleared two OpenSwiftUI off-Apple stubs (in `openswiftui-phase1-wip.patch`):
  - `_GraphInputs.defaultInterfaceIdiom` (`InterfaceIdiomPredicate.swift:36`) was
    `_openSwiftUIUnimplementedFailure()` on non-Apple → default to `.phone` (wandr is a
    phone-class device). Needed by `Text.makeCommonAttributes`.
  - `Text` localization hit `Bundle.main`, which CRASHES in Foundation's lazy `_mainBundle`
    init on wasm. `Text+Localized.swift` (resolve + resolvesToEmpty) now `#if os(WASI)`
    localize only if an explicit bundle was passed, else use the key literally (the
    documented no-table fallback). No more Bundle.main on wasm.
- **THE WALL: reading `@State` hangs.** `ContentView { @State count; Color.red.opacity(count>0 ?…) }`
  → infinite loop (exit 124). Custom View *without* @State renders; adding @State + reading
  it spins. The spin is NOT in `Update.dispatchActions` nor the render-driver loop (bounded
  counters never tripped — instrumentation since removed). It's in the reactive
  update/invalidation machinery → almost certainly the **`onUpdate`/`onInvalidation` no-op
  shortcut** (Compute `Graph.swift`, `#if arch(wasm32)`): @State's dependency invalidation
  can't propagate/settle, so the graph never reaches steady state.
- **FIX (next): properly wire `onUpdate` AND `onInvalidation` as STORED plain-C callbacks.**
  Unlike the synchronous callbacks (forEachField/mutateBody), these are stored and invoked
  later by C++, so box the Swift closure (heap object) + pass a non-capturing
  `@convention(c)` trampoline + retain/release the box. The Compute fork ALREADY has this
  exact template: `_UpdateBox` + `AGRetainClosureC`/`AGReleaseClosure` in
  `Attribute/AttributeType.swift` (used for the rule `_update` callback). Mirror it for
  `AGGraphSetUpdateCallback`/`AGGraphSetInvalidationCallback` (need C++ `*C` setters that
  store a plain-C fn+ctx and invoke plain-C in `Context::call_update`/invalidation).
- Text itself: construction + resolution now work; whether Text *layout* (fonts/paragraph,
  a `StatefulRule`) has further walls is UNKNOWN — @State hangs before we get there. Note
  the stdout renderer doesn't emit `.text` items anyway (only fills); real glyphs come via
  the phase-4 CGContext drawer.

### ⛔ (earlier framing) custom `View` with a reactive `body` — RESOLVED (was a stale build)
Isolated: `Color.red` DIRECTLY in `WindowGroup` renders ✅; wrapping it in
`struct ContentView: View { var body: some View { Color.red } }` → **infinite loop**
(wasmtime exit 124, no stderr). Independent of `@State`/`Text` (both also hang, same cause).
- The render driver's outer `repeat…while true` is bounded (breaks in ≤2 iters); the spin
  is INSIDE the update machinery. Prime suspect: `Update.dispatchActions()`
  (`Data/Update.swift:191`) — `repeat { … } while !Update.actions.isEmpty` with **no
  iteration bound**; if dispatched actions keep re-enqueuing, it never drains.
- Likely root: the `onUpdate`/`onInvalidation` **no-op shortcut** (Compute `Graph.swift`,
  `#if arch(wasm32)`) breaks the update-completion signal for reactive bodies; primitive
  views don't exercise it, custom views do. This is the phase-3 follow-up previously
  flagged as "owed".
- DIAGNOSE NEXT: add a bounded counter + `preconditionFailure` in `dispatchActions`
  (and/or `ViewGraph` transaction loop) to convert the hang into a backtrace and confirm
  the spinner + the re-enqueuing action. Then FIX: properly wire `onUpdate` as a plain-C
  callback variant (like `attribute_modify_c`/`AGGraphMutateAttributeC` — it's a STORED
  callback, so box the closure: see the fork's existing `_UpdateBox` + `AGRetainClosureC`
  pattern in compute-wasm) and/or drain `_wasmDrainMainRunLoop()` so transactions settle.
- Minimal repro is the current `probe/Sources/Probe/ProbeApp.swift` (custom View, no State).

### NEXT (for richer UIs — Text / VStack / @State), all bounded + validatable in the 4s harness
- `Subgraph.forEach`/`AGSubgraphApply` `*C` variant — for clean teardown + views that
  call forEach mid-render. (engine `Subgraph::apply_c`, like `attribute_modify_c`.)
- `swift_conformsToProtocol` C-shim in `OpenSwiftUI_SPI` — unblocks `VStack`/`TupleView`.
- Bounded Compute set: `AGGraphReadCachedAttribute`, `AGGraphSearch`, `AGTupleWithBuffer`,
  `AGTypeApplyEnumData`/`MutableEnumData`.
Then PHASE 4: wire the real `DisplayList` into the Option-B CGContext drawer → device.

## 🔭 (earlier) PHASE 3: the OpenSwiftUI render pipeline RUNS on wasm
De-risk probe `repros/openswiftui-wasm/probe` (a `@main App { VStack { Color.red; Color.blue } }`)
builds to `probe.wasm` (~138MB debug) and **executes under wasmtime**: `App.main →
runStdoutApp → AppGraph → GraphHost → ViewGraph.instantiateOutputs → render → View
construction`. The AttributeGraph/Compute reactive engine genuinely runs in wasm. The
remaining work is a mechanical grind of ABI walls (below).

### Build/run the probe (NOTE: ANY_ATTRIBUTE_FIX=0 is now REQUIRED)
```bash
cd repros/openswiftui-wasm/probe
OPENSWIFTUI_ANY_ATTRIBUTE_FIX=0 ANY_ATTRIBUTE_FIX=0 \
OPENSWIFTUI_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 \
OPENRENDERBOX_LIB_SWIFT_PATH=/tmp/oag-fork/Sources/SwiftCorelibs/include \
swift build --product probe --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I/tmp/oag-shims -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS
wasmtime run .build/wasm32-unknown-wasip1/debug/probe.wasm   # prints the DisplayList when done
```

### Phase 3 walls cleared (in the patches)
1. **Link-time CFRunLoop/Timer undefined** — libs compiled but linking the exe surfaced
   Foundation symbols absent on wasm. Guarded CF code in `RunLoopUtils.swift`
   (addObserver/runAllowingEarlyExit/CFRunLoopMode) + a `Timer` shim shadowing
   `Foundation.Timer` in `WasmThreadingShim.swift` (its impl pulls CFRunLoopTimer*).
2. **`OPENSWIFTUI_ANY_ATTRIBUTE_FIX` → `preconditionFailure("#39")` stubs** — disabled the
   fix (`ANY_ATTRIBUTE_FIX=0`) to bind the REAL Compute `AnyAttribute`/`onInvalidation`/
   `Subgraph.*`; needed one `package import` fix (`View_Indirect.swift`).
3. **swiftcc closure ABI mismatch (the main grind).** Swift closures passed to
   `AG_SWIFT_CC(swift)` C fn-pointer params mislower on wasm → `signature_mismatch` trap.
   KEY: zero-arg `()->Void` closures lower fine (e.g. `AGSubgraphApply` works); closures
   **with args or a return value** break. Each broken one needs a plain-C `*C` variant:
   C++ engine `_c` method (if the closure is invoked inside the engine) + `extern "C"`
   wrapper + header decl (`#if defined(__wasi__)`) + Swift `#if arch(wasm32)` routing with a
   non-capturing `@convention(c)` thunk + boxed/by-pointer context. Template = the fork's
   pre-existing `AGGraphInternAttributeTypeC` (synchronous: `withoutActuallyEscaping` +
   `withUnsafePointer`). Cleared so far: `onUpdate`/`onInvalidation` (guarded no-op),
   `withMainThreadHandler` (run body directly), `forEachField` (`AGTypeApplyFields/2C`),
   `AGGraphMutateAttribute` (`attribute_modify_c`). Gotcha: C params without `_Nullable`
   import as NON-optional in the `@convention(c)` thunk (don't force-unwrap).

### 🧭 STRATEGY (validated 2026-06-19 — two agents + an empirical test)
Root cause (confirmed): `swiftcall` carries closure context/error in *register* slots
absent on wasm; clang (C++ engine) and swiftc disagree on the `call_indirect`
function-table type → `signature_mismatch`. So zero-arg `()->Void` lower fine; closures
**with args/return** break. **No compiler flag fixes it; 6.3.2 is the latest SDK; the
upstream auto-thunk fix was reverted → a toolchain bump will NOT help.**
- TESTED & REJECTED the proposed "one header edit" general fix: flipping
  `AG_SWIFT_CC`/`AG_SWIFT_CONTEXT` to plain-C on wasm in `AGBase.h`. With a raw Swift
  closure → still traps (a Swift closure's own fn is swiftcc). With a `@convention(c)`
  thunk to the **original** symbol → STILL traps. Only a **separate `*C` symbol** +
  `@convention(c)` thunk works (= the per-function pattern already in use). Reverted it.
- REAL process wins (kill the "cycling"): (1) validate each Compute `*C` shape in the
  **seconds-fast** harness `repros/compute-wasm/computerun` (~4s rebuild) BEFORE the
  ~15-min probe rebuild; (2) the remaining surface is BOUNDED — ~5 Compute callbacks +
  exactly ONE Swift-runtime symbol (`swift_conformsToProtocol`); (3) target the smallest
  view first.

### 🎉 MILESTONE (2026-06-19): single-`Color.red` probe — render pipeline RUNS TO COMPLETION
Shrinking the probe to `WindowGroup { Color.red }` gets PAST View construction and the
whole render — the only trap now is in **TEARDOWN**: `runStdoutApp` closure ends →
`StdoutRendererHost`/`ViewGraph`/`GraphHost` deinit → `GraphHost.invalidate` →
`Subgraph.forEach` (an `(AnyAttribute)->Void` arg-closure → `AGSubgraphApply`) traps.
Open: the DisplayList didn't print — `renderOnce()` set up the renderer (logged
"OpenSwiftUI ViewRendererVendor: …osui") but `DisplayList.ViewRenderer.render(from:list)`
didn't emit (bare Color may produce an empty list / need real scene sizing, or the
onUpdate no-op suppressed a display-list-update pass). NEXT: (a) `*C` variant for
`Subgraph.forEach`/`AGSubgraphApply` (validate in harness first); (b) investigate the
empty emit — try a `Text`/sized view, or check whether `requestedOutputs=[.displayList]`
actually produced items. The remaining Compute walls (`AGGraphReadCachedAttribute`,
`AGGraphSearch`, `AGTupleWithBuffer`, `AGTypeApplyEnumData`/`MutableEnumData`) only get
hit by richer views.

### ⛔ EARLIER WALL (VStack probe): `signature_mismatch:swift_conformsToProtocol`
A DIFFERENT class — a **Swift runtime** function, not Compute. `OpenSwiftUICore/Runtime/
TypeConformance.swift:52` declares `@_silgen_name("swift_conformsToProtocol") func(...) ->
UnsafeRawPointer?`; the call mislowers on wasm (hit during `VStack._makeView →
TupleView._makeViewList → ProtocolDescriptor.conformance`). Candidate fix: a C shim in the
`OpenSwiftUI_SPI` C target that calls `swift_conformsToProtocol` with the correct C ABI and
is imported by Swift (mirrors the Compute *C pattern but for a runtime symbol). Needs
experimentation (its wasm calling convention is unconfirmed; SDK headers are stripped).

### Remaining likely swiftcc walls after that (args/return closures, not yet hit)
`AGGraphReadCachedAttribute`, `AGGraphSearch`, `AGTupleWithBuffer`, `AGTypeApplyEnumData`,
`AGTypeApplyMutableEnumData` (all in `/tmp/Compute`, none guarded yet).

### Phase-3 patches (regenerate /tmp from these — /tmp is ephemeral!)
- `openswiftui-phase1-wip.patch` (base `bb31b59`) — now phases 1–3 OpenSwiftUI-repo changes.
- `oag-fork.patch` (base `f20328e`) — OAG fork working tree.
- `compute-wasm.patch` (base `efb754b`) — NEW: the Compute swiftcc `*C` variants
  (Graph.swift/Metadata.swift/AnyAttribute.swift + ComputeCxx AGType/AGGraph/Graph .cpp/.h).
- `probe/` — the de-risk app (committed as files in the repo).

---

## ✅ PHASES 1+2 DONE: OpenSwiftUICore AND OpenSwiftUI compile for wasm32-wasip1
- Phase 1: `swift build --target OpenSwiftUICore … wasm` → **Build complete!** (0 errors).
- Phase 2: `swift build --target OpenSwiftUI … wasm` → **Build complete! (21.65s)** (0 errors).
The whole Foundation threading/run-loop substrate is shimmed; zero View/render errors
across both layers. Next = **phase 3** (WandrRendererHost + wire DisplayList → Option-B drawer).

### Phase 2 walls cleared (all in the patches)
- `Thread.sleep(forTimeInterval:)` added to the WASI `Thread` shim — typed `Double`
  not `TimeInterval` (Foundation is imported `internal`, so a `package` method can't
  expose the `TimeInterval` alias). Used only by test-harness loop pumping.
- `_ViewTest.loop()` / `turnRunloop()` / `turnRunLoopIfNeeded()` (Test/ViewTest.swift,
  test scaffolding shipped IN the lib) — `RunLoop.current.run(mode:before:)` is absent
  on WASI, so `#if os(WASI)`-guarded to drain `_wasmDrainMainRunLoop()` + render instead
  of pumping a (nonexistent) run loop.
- `Graph.archiveJSON(name: String?)` static added to the **OAG fork** Compute adapter
  (`oag-fork/Sources/OpenAttributeGraphShims/Adapter/Compute.swift`). AppGraph calls the
  AttributeGraph-standard static; Compute's instance `archiveJSON` is a `fatalError` stub,
  so the static is a no-op (debug launch-profiling only). Captured in `oag-fork.patch`.

## 🎯 TARGET GOAL: run **https://github.com/eleev/swiftui-2048** on wandr (Pixel 2 XL)
A real, polished, pure-SwiftUI game (the locked "real app" target). Verified suitable:
**40× `import SwiftUI`, no UIKit, no storyboards, no 3rd-party deps**; only `AudioToolbox`
(stub on wasm) + `Combine`. It's the end-to-end proof — a real SwiftUI app rendering on
the device through the whole stack.

Path to it: compile **OpenSwiftUICore** (then OpenSwiftUI) for `wasm32-wasip1` → wire the
validated DisplayList→CGContext renderer (Option B) → drop in swiftui-2048.
Full plan + scope: `docs/swift-openswiftui-wandr-feasibility.md` (phases 0–5).

## What's done (proven, pushed)
- Engine: `harryzz/Compute@wasm32-wasip1-osp` (AttributeGraph on wasm, reactive 42).
- `harryzz/OpenAttributeGraph` (WASI un-stubs; engine = Compute backend).
- `harryzz/OpenCoreGraphics@wasm32-wasip1` (CGContext over wasi:canvas, device-verified).
- Renderer backend prototyped + device-verified: `repros/swift-canvas-spike` (P4).
- Phase 0: OpenCombine/OpenObservation build unmodified; OpenRenderBox builds (compile-only).

## Build environment (the /tmp layout the OpenSwiftUI build expects)
OpenSwiftUI uses `USE_LOCAL_DEPS` → siblings at `../<Name>`. Recreate if /tmp was wiped:
```
/tmp/OpenSwiftUI            # clone of OpenSwiftUIProject/OpenSwiftUI + this patch
/tmp/OpenAttributeGraph  -> symlink to /tmp/oag-fork   (harryzz/OpenAttributeGraph, un-stubbed)
/tmp/OpenRenderBox       -> symlink to /tmp/OpenRenderBox-dep
/tmp/OpenObservation     -> symlink to /tmp/OpenObservation-dep
/tmp/OpenCoreGraphics       # upstream clone (stub CGContext compiles; wasi:canvas backend wired in phase 3)
/tmp/Compute                # harryzz/Compute on branch wasm32-wasip1-osp (OAG's Compute backend, ../Compute)
/tmp/oag-fork/Checkouts/swift -> symlink to /tmp/Compute/Submodules/swift-runtime-headers
/tmp/oag-shims/             # dispatch/syslog/openssl-sha/uint shims on -Xcc -I (from repros/compute-wasm/shims + a dispatch/dispatch.h)
```
Apply the WIP patches (both base-pinned):
- OpenSwiftUI repo (base `bb31b59`): `cd /tmp/OpenSwiftUI && git apply repros/openswiftui-wasm/openswiftui-phase1-wip.patch`
  — self-contained: CREATES `Util/WasmDispatchShim.swift` + `Util/WasmThreadingShim.swift`
  and carries the phase-1+2 OpenSwiftUICore/OpenSwiftUI edits (no manual file copy).
- OAG fork (base `f20328e`): `cd /tmp/oag-fork && git apply repros/openswiftui-wasm/oag-fork.patch`
  — the full phase-0/1/2 OAG working-tree state (un-stubs + Compute-adapter `archiveJSON`).
  /tmp is ephemeral, so this snapshot is the source of truth until pushed to the fork.

## The build command
```bash
cd /tmp/OpenSwiftUI
BASE=~/.swiftpm/swift-sdks/swift-6.3.2-RELEASE_wasm.artifactbundle/swift-6.3.2-RELEASE_wasm/wasm32-unknown-wasip1
OPENSWIFTUI_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 \
OPENRENDERBOX_LIB_SWIFT_PATH=/tmp/oag-fork/Sources/SwiftCorelibs/include \
swift build --target OpenSwiftUICore --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I/tmp/oag-shims -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS
```

## Walls cleared (in the patch)
- Dispatch shim (`WasmDispatchShim.swift` + guarded `import Dispatch` in AnimationListener);
  OpenCombineFoundation dep gated off non-Darwin (Package.swift); dladdr guarded
  (OpenSwiftUI_CSymbols.c); WASILibc branches (StandardLibraryAdditions.swift).
- **Threading substrate (`WasmThreadingShim.swift`)** — single-threaded WASI shims:
  - `Thread.isMainThread` → always `true` (the 2 explicit `import class Foundation.Thread`
    in StateObject/AttributeInvalidatingSubscriber are `#if !os(WASI)`-guarded; the shim
    provides a module-level `enum Thread`).
  - pthread TLS for `ThreadSpecific` — pure-Swift `pthread_key_create/getspecific/setspecific`
    over a process-global table (single thread ⇒ TLS == global). `pthread_key_t` IS visible
    from WASILibc so it's reused. Shims are `internal` (the imported C type is internal — a
    `package` func can't re-export it).
  - **RunLoop is fully `@available(*, unavailable)` on WASI** (even `RunLoop.main` traps at
    type-check) — so do NOT extend RunLoop. The two call sites (`RunLoopUtils.onNextMainRunLoop`,
    `TimerUtils.withDelay`) are `#if os(WASI)`-guarded to route through `_wasmEnqueueMainRunLoop`
    + `_wasmDrainMainRunLoop()` (a global queue the host frame loop drains in phase 3). Timers
    are no-ops until wired to the host frame clock. (The only other RunLoop user, CAHostingLayer,
    is `#if canImport(QuartzCore)` so excluded.)

## ⚠️ Phase-3 follow-ups owed by the threading shim (don't lose these)
- `_wasmDrainMainRunLoop()` must be called once per host frame, else `onNextMainRunLoop`
  deferred work (invalidations) never runs → no UI updates. It's `package`; bump to `public`
  if the host glue lives in a different Swift module.
- `withDelay` timers don't fire yet (no run loop). Wire them to the host frame clock when
  animations/timeouts are needed (phase 4).

## After OpenSwiftUICore compiles
Phase 2: OpenSwiftUI (app layer). Phase 3: a `WandrRendererHost` (model on
`StdoutRendererHost`) + wire real `DisplayList.Item` into the Option-B drawer
(`repros/swift-canvas-spike/Sources/SwiftSpike/DisplayListRenderer.swift`). Phase 4:
hand-written `Text`+`@State`+`Button` on device. Phase 5: `eleev/swiftui-2048`.
