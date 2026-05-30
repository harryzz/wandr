---
name: Granular Android lifecycle via WindowEvent::Focused proxy
description: How the host emits Paused/Resumed transitions on wasi without JNI hookup to Activity.onPause/onResume — uses winit's Focus events as a proxy.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
winit 0.30 with `android-native-activity` collapses Android's `MainEvent::Start/Resume/Pause/Stop/Destroy` events: only `InitWindow` (NativeWindow created) and `TerminateWindow` (NativeWindow destroyed) reach the app as `Event::Resumed` / `Event::Suspended`. The real Activity lifecycle events are silently dropped with TODOs in winit's source (`winit-0.30/src/platform_impl/android/mod.rs` lines 268-293).

Workable proxy: `WindowEvent::GainedFocus`/`LostFocus` *do* reach us as `WindowEvent::Focused(bool)`. On Android these fire adjacent to `onResume`/`onPause` (LostFocus is dispatched right before onPause, GainedFocus right after onResume). Map:

- `WindowEvent::Focused(false)` → emit `Paused` (state, not event)
- `WindowEvent::Focused(true)` → emit `Resumed`
- existing `App::suspended` → emit `Stopped` (TerminateWindow)
- existing first-frame seed → emit `Resumed` (InitWindow)

Use a `set_lifecycle(state)` helper that only dispatches when the new state differs from `lifecycle.current`. The guest-side `LifecycleRegistry` walks the state machine internally (RESUMED→STARTED dispatches ON_PAUSE event automatically), so the bridge in test-app just needs to update `registry.currentState`.

Verified 2026-05-12 end-to-end:
- HOME press → `→ PAUSED` (WasiLifecycle observer) → `LifecycleEventEffect ON_PAUSE fired` (Compose) → ~400ms later `→ STOPPED`.

**Still not emitted: `Created`, `Started`, `Destroyed`.** Reaching these requires bypassing winit's MainEvent mapping. Options for a future cycle:
- Fork android-activity to insert our own callback before winit consumes the MainEvent.
- Subclass `NativeActivity` in Java and route activity lifecycle callbacks through JNI (requires customizing the AndroidManifest cargo-apk generates — not currently easy).
- Switch to `android-game-activity` and write a small custom Java shim that wraps GameActivity to forward all six lifecycle callbacks.

**Why:** Saves diving into winit internals. The Focused-event proxy is the simplest path to "good enough" lifecycle without changing the activity class.

**How to apply:**
- When wiring lifecycle-aware Compose APIs (animations, coroutines that need cancel-on-pause, lifecycle-runtime-compose effects), they work today for ON_RESUME and ON_PAUSE.
- If a Composable explicitly needs ON_CREATE / ON_START / ON_DESTROY, currently it gets ON_RESUME at compose time (LifecycleRegistry walks INITIALIZED → CREATED → STARTED → RESUMED at first state-set) and will never see ON_DESTROY (store is destroyed before host can fire it; see task #43). Document this limitation in the API surface until granular events are wired.
