---
name: WasiLifecycle ↔ LocalLifecycleOwner bridge pattern
description: How to wire host activity lifecycle (delivered via WIT) into Compose's LocalLifecycleOwner so widgets using lifecycle-runtime-compose work on wasmWasi.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
The `LocalLifecycleOwner` CompositionLocal in `compose-runtime-wasi/.../LocalLifecycleOwner.wasi.kt` defaults to `staticCompositionLocalOf { error("LocalLifecycleOwner not present") }`. Any widget that reads it (lifecycle-runtime-compose `LifecycleEventEffect`/`LifecycleResumeEffect`, navigation, certain Material3 internals) throws unless an owner is provided.

The working pattern (verified 2026-05-12):

1. Build a `LifecycleOwner` whose `lifecycle: Lifecycle` is a `LifecycleRegistry(this)`. Bridge construction must seed `registry.currentState` from `WasiLifecycle.currentState()` AND install `WasiLifecycle.addObserver { state -> registry.currentState = mapState(state) }`.

2. Map the WIT 7-state enum (initialized/created/started/resumed/paused/stopped/destroyed) to the Compose 5-state enum (DESTROYED/INITIALIZED/CREATED/STARTED/RESUMED): PAUSED→STARTED, STOPPED→CREATED, DESTROYED→DESTROYED, others 1:1.

3. At the scene root, wrap `setContent { ... }` body with `CompositionLocalProvider(LocalLifecycleOwner provides bridge) { /* content */ }`.

Setting `registry.currentState = State.RESUMED` from INITIALIZED auto-fires intermediate ON_CREATE → ON_START → ON_RESUME events to observers — the LifecycleRegistry walks the state machine for you.

The bridge file is at `/home/harry/skiko/test-app/src/wasmWasiMain/kotlin/WasiLifecycleOwnerBridge.kt`. It's currently in test-app rather than skiko-wasm-wasi because skiko has no compose-runtime dependency; if multiple apps need it, lift it into skiko-wasm-wasi (which already owns `WasiLifecycle`) and add `androidx.lifecycle:lifecycle-runtime` as a dependency, or define a side-channel interface in compose-runtime-wasi that skiko implements.

`enforceMainThread = true` in `LifecycleRegistry(provider)` is fine on wasmWasi — `isMainThread()` is hardcoded to `true` in `LifecycleRegistry.web.kt`, which is the webMain actual we inherit. `createUnsafe(owner)` is not required.

**How to apply:**
- For any new Compose Multiplatform module added on wasmWasi that may consume LocalLifecycleOwner, ensure the scene root provides the bridge (or hoist a default into `LocalLifecycleOwner.wasi.kt`'s `staticCompositionLocalOf { ... }`). Without it, any read throws.
- Current bridge only carries host-emitted transitions. Host today emits only `Resumed`/`Stopped` (winit binary). For real onPause/onStop semantics, see tasks #42 (granular state emission) and #43 (state preservation across suspend).
