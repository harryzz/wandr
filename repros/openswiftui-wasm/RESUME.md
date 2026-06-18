# OpenSwiftUI on wasm — phase 1 resume point (2026-06-18)

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
