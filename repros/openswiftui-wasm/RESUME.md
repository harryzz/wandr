# OpenSwiftUI on wasm — phase 1 resume point (2026-06-18)

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
Apply this WIP: `cd /tmp/OpenSwiftUI && git apply repros/openswiftui-wasm/openswiftui-phase1-wip.patch`
(base commit `bb31b59`) and add `WasmDispatchShim.swift` →
`Sources/OpenSwiftUICore/Util/`.

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
Dispatch shim (`WasmDispatchShim.swift` + guarded `import Dispatch` in AnimationListener);
OpenCombineFoundation dep gated off non-Darwin (Package.swift); dladdr guarded
(OpenSwiftUI_CSymbols.c); WASILibc branches (StandardLibraryAdditions.swift).

## Remaining phase-1 work — the threading/observation substrate (~8 files)
All Foundation concurrency, NOT SwiftUI logic (zero View/render errors so far — good sign):
- `ThreadUtils.swift` — `Thread` + `pthread_key_create`/`getspecific`/`setspecific` (TLS)
- `RunLoopUtils.swift` — `RunLoop.perform`/`.add`/`.common`
- `TimerUtils.swift`, `MainActorUtils.swift`, `ObservationUtils.swift`,
  `StateObject.swift`, `ObjectLocation.swift`, `AttributeInvalidatingSubscriber.swift`
Fix = a single-threaded wasm shim: `Thread`→main, pthread-TLS→a global, `RunLoop`→
frame-loop/stub, `MainActor`→trivial. Bounded, mechanical (the `pthread`/`RunLoop` ones
are real shims, not just guards). Then continue iterating `swift build` to the next wall.

## After OpenSwiftUICore compiles
Phase 2: OpenSwiftUI (app layer). Phase 3: a `WandrRendererHost` (model on
`StdoutRendererHost`) + wire real `DisplayList.Item` into the Option-B drawer
(`repros/swift-canvas-spike/Sources/SwiftSpike/DisplayListRenderer.swift`). Phase 4:
hand-written `Text`+`@State`+`Button` on device. Phase 5: `eleev/swiftui-2048`.
