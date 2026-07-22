---
name: reference_openswiftui_reactor_main_boot
description: How a wandr guest boots its OWN unmodified @main App on a wasip1 reactor (no _start); the WandrReactorExports/CWandrExports/CWandrBoot mechanism that keeps the app dir free of reactor stubs
metadata: 
  node_type: memory
  type: reference
  originSessionId: efb9ba77-bb47-4ab5-bbac-3dcd59e2771e
---

**Goal achieved (2026-07-18):** eleev/swiftui-2048's pristine `@main struct T2ilesApp: App` runs on
wandr UNMODIFIED, and ALL reactor glue lives in shared libraries — the app target carries zero
`@_cdecl` stubs. See `[[reference_swift_openswiftui_wandr]]`.

**The core problem:** a wandr guest is a wasip1 REACTOR (`-mexec-model=reactor`) — there is no
`_start`, so Swift's `@main`-generated entry (`__main_argc_argv`) is never auto-called; and even when
called, `App.main()` must NOT run-to-completion/exit (the host owns the loop, driving frames via
exported callbacks).

**The mechanism (all framework/library, nothing in the app):**
1. **Host** calls `wandr:ui-shell/startup` `on-init` exactly once after instantiate (separate
   probe-only world from `shell-events` — adding a func to an existing interface breaks its
   all-or-nothing `World::new()` probe for every other guest). See `[[reference_openswiftui_conditional_wasm_metadata]]`.
2. `on-init` (in `WandrReactorExports`) → `WandrRuntime.bootWandrReactorApp()`: arms the reactor
   (`armWandrReactor()`), then calls `wandr_run_app_main()`.
3. `wandr_run_app_main()` is a tiny **C shim** (`CWandrBoot` target in the wandr-runtime package) that
   calls `__main_argc_argv(0,0)`. MUST be C: a Swift `@_silgen_name` decl lowers it with Swift's
   calling convention → `(i32,i32,i32,i32)` vs the real `(i32,i32)` → wasm-ld inserts a
   `signature_mismatch:main` trampoline that TRAPS (`unreachable`). `@_extern(c)`'s `Extern` feature
   won't propagate to emit-module on the 6.3.2 wasm SDK. The C shim gives the correct ABI for free.
4. `__main_argc_argv` runs eleev's `T2ilesApp.main()` → OpenSwiftUI's `App.main()` sees the reactor is
   armed → `registerWandrApp(Self())` + `return` (instead of `runStdoutApp`/`runApp`→Never). Registry
   lives in `OpenSwiftUI/.../Stdout/WandrApp.swift` (`armWandrReactor`/`registerWandrApp`/
   `launchRegisteredWandrApp`, `@_spi(WandrRenderer)`); launcher closure captures the concrete app
   type so `renderWandrAppOnce` stays generic.
5. wandr-runtime's frame loop calls `launchRegisteredWandrApp(options:)` on the first real-sized frame
   with its wasi:canvas CGSink.

**Package layout (all in main repo under swift/OpenSwiftUIProject/):**
- `CWandrExports` — wit-bindgen leaf (mirrors CWasiAudio): world exports frame/pointer-handler +
  shell-events/frame-pacing/startup, imports metrics. Provides the `__wasm_export_*` wrappers + types
  + `component_type.o`.
- `wandr-runtime/Sources/WandrReactorExports` — the 6 `@_cdecl exports_*` impls (forward into
  WandrRuntime) + on-init. **Opt-in product**: only apps that link it get the symbols, so apps keeping
  hand-written stubs (OpenSwiftUIDemo, SwiftSpike) don't get duplicate-symbol conflicts.
- `wandr-runtime/Sources/CWandrBoot` — the C boot shim.

**App wiring (repros/swift-canvas-spike, target T2iles):** depends on `WandrReactorExports` (NOT
CSwiftSpike); links `CWandrExports/generated/cwandr_exports_component_type.o` (+ cwasi_canvas/audio);
`@main T2ilesApp` un-excluded, verbatim. Export wrappers stay live because WandrReactorExports'
on-init references `wandr_ui_shell_metrics_get_density()` (an import stub in the same
`cwandr_exports.o`), pulling the whole object.

**Verify:** `./build-t2iles.sh` (release, the device config) or `WANDR_T2ILES_CONFIG=debug
./build-t2iles.sh`, run on desktop (`wasm-android-host --app wandr.swiftui.demo`), expect host log
`called on-init once` + `render_frame #N ok=true`.

**RELEASE CMO crash — FIXED (2026-07-18)** via `.unsafeFlags(["-Onone"])` in OpenSwiftUI's
`sharedSwiftSettings` (Package.swift, commit 363e1e62). At `-O` the CrossModuleOptimization SIL pass
crashes on `OpenSwiftUICore/StyleContext`'s parameter-pack (`each Q`) protocol conformances ("Abstract
conformance with bad subject type" / `forAbstract` at ASTContext.cpp:5924). Investigated thoroughly:
- 6.3.2 AND 6.3.3 both crash identically (not a 6.3.x-line fix).
- main-snapshot 6.5-dev FIXES the CMO crash but default CMO then trips a pervasive
  `@_alwaysEmitIntoClient` serialization verifier bug (192 funcs) — no toolchain builds this at full -O.
- Default CMO can't be disabled by flag: SwiftPM force-appends `-enable-default-cmo` (last), which
  `-disable-cmo` does NOT cancel; `@_semantics("optimize.no.crossmodule")` can't reach the synthesized
  thunk CMO chokes on. So `-Onone` (lands after SwiftPM's `-O`, wins → no opt pipeline → no CMO pass)
  is the deterministic lever. Compute/AttributeGraph (a SEPARATE package, the heavy hot path) stays
  `-O`; only the OpenSwiftUI view layer is unoptimized. Revisit when a fixed stable toolchain ships.

**Future:** the clean production app will live under `apps/user/` as a proper wandrpkg (only eleev
sources + package.toml) consuming WandrReactorExports; the `repros/swift-canvas-spike` spike keeps the
gated `WandrHeadless` harness (can't be a separate target — it needs eleev's INTERNAL types, and
making them public would modify eleev's sources).
