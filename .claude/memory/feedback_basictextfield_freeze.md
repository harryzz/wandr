---
name: BasicTextField tap-freeze — legacy API only; use TextFieldState API
description: Legacy `BasicTextField(value, onValueChange)` hangs wasm at 100% CPU on tap-to-focus. NEW `BasicTextField(state: TextFieldState, ...)` API renders + focuses cleanly. Cursor-blink delay() spin was separately fixed via WasiFrameDispatcher.Delay impl. LazyColumn confirmed working without changes.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---

## TL;DR (verified 2026-05-13)

- ✅ **LazyColumn** — works as-is.
- ✅ **`WasiFrameDispatcher.Delay` impl** — fixes cursor-blink busy-loop. Required infrastructure.
- ❌ **Legacy `BasicTextField(value, onValueChange)`** — tap-to-focus freezes wasm main thread at 100% CPU in the synchronous `defaultTextFieldPointer` onTap path. Even reproduces in absolute-minimal `Surface { BasicTextField(value, ...) }`. Material3's `TextField` / `OutlinedTextField` wrap this — also freeze.
- ✅ **NEW `BasicTextField(state: TextFieldState, ...)`** (formerly `BasicTextField2`) — renders, focuses, NO freeze. Different state plumbing avoids the legacy onTap synchronous-recompose loop. **Use this in apps.**
- ✅ **Hardware-keyboard typing** — new `on-key-event-v2(kind, code-point, key-id)` WIT call carries the resolved UTF-32 codepoint + Compose-compatible key-id. The test app's `WasiInput.setKeyHandler` builds a Compose `KeyEvent` from these and calls `realScene.sendKeyEvent`. Verified end-to-end: `adb shell input keyevent KEYCODE_A KEYCODE_B` types "ab" into the focused field.
- ⏳ **IME / soft keyboard** — still deferred. Would need JNI from Rust to `InputMethodManager` to show/hide the system keyboard and bridge IME composition state back to Compose's `PlatformTextInputService`.

## What works (confirmed 2026-05-13)

- **LazyColumn**: virtualized list with 200 items renders correctly. Nested scrolling inside an outer `verticalScroll` Column works (items #1→#10 → #7→#16 → #168→#177 via successive flings). The old "currently crashes composition" comment in the test app was stale — one or more of our earlier fixes (identityHashCode for `DerivedSnapshotState`, host-side live transforms, post-pointer-event flush) silently resolved it. **No code change needed**.
- **BasicTextField rendering**: with `var text by remember { mutableStateOf("hello world") }`, a plain `BasicTextField(value = text, onValueChange = { text = it })` composes and renders the text correctly. No crash.

## What broke and what we fixed along the way

### Fix #1 — `WasiFrameDispatcher` now implements `Delay`

`compose-foundation`'s `CursorAnimationState.snapToVisibleAndAnimate()` runs `while(true) { delay(500); cursorAlpha = 0f; delay(500); cursorAlpha = 1f }`. Its kdoc explicitly says "pure coroutine delays will not cause any work until the delay is over." But on wasi, `kotlinx-coroutines` falls back to `DefaultDelay` if no dispatcher in the context implements `Delay` — and on wasmWasi that fallback effectively spins.

**Fix:** `WasiFrameDispatcher` now implements `kotlinx.coroutines.Delay`:
- `scheduleResumeAfterDelay(timeMillis, continuation)` adds a `DelayedTask(deadlineMillis = nowMillis()+timeMillis, runnable)` to an unsorted list; `flush()` (called once per frame) moves due tasks to the main queue and resumes them.
- `invokeOnTimeout(timeMillis, block, context)` does the same for `withTimeout`-style timers, returning a `DisposableHandle` that nulls the runnable on cancel.
- `nowMillis()` reads `cachedNanoTime()` (already updated by the renderer each frame via `updateCachedNanoTime(nanos)`) and divides by 1e6.
- New public `cachedNanoTime()` getter in `compose-ui-wasi/.../UiActuals.wasi.kt` so the dispatcher can read it.

Files: `compose-ui-wasi/src/wasmWasiActuals/kotlin/androidx/compose/ui/platform/WasiFrameDispatcher.kt`, `…/UiActuals.wasi.kt`.

The fix is correct and `delay()` no longer spins. **Keep it.**

## What's still broken — the BasicTextField tap freeze

**Symptom:** Tap a real `BasicTextField` → screen rendering stops, all progress indicators freeze, wasm `android_main` thread sits at ~100% CPU in state R. No log spam (the spin doesn't go through any path we log). No exception. No `WFD.dispatch` activity (counted via debug instrumentation, < 100 dispatches before stop). `scene.render` is never called again after the tap.

**Timeline of one frozen tap (verified via per-event log probes):**

```
ptr DOWN at (368,1715)
ptr DOWN sendPointerEvent returned       (3ms — Compose processed DOWN cleanly)
WFD.scheduleResumeAfterDelay(492ms)      (long-press timer from detectTapAndPress.withTimeoutOrNull)
ptr DOWN flush done
ptr UP at (368,1715) → sendPointerEvent...
[FREEZE — no further logs ever]
```

The freeze is **inside `realScene.sendPointerEvent(UP)`** — the synchronous call never returns. CPU stays at 100% R indefinitely.

**Where the bisect points to:**

- `Modifier.focusable()` alone on a Box: **no freeze**.
- `Modifier.clickable {}` alone on a Box: **no freeze** (same `detectTapGestures` machinery as BasicTextField uses internally for taps).
- `BasicTextField` with `readOnly = true` or `cursorBrush = SolidColor(Color.Transparent)`: **freezes** identically.
- `BasicTextField` with `enabled = false` (no focus acquired): **1-second hiccup then resumes** — confirms the spin starts when focus is acquired.
- `BasicTextField` inside `Column(.verticalScroll())`: freezes. (We tested this in case the iOS-specific tip about parent `Modifier.clickable` conflicting applied — it didn't.)
- **`Surface { BasicTextField(value, onValueChange) }` — ABSOLUTE MINIMAL — STILL FREEZES**. No Card, no Column, no Box, no surrounding verticalScroll. Just Surface (which doesn't install gestures) → BasicTextField. Confirms the freeze is **intrinsic to BasicTextField on wasi**, not a parent-modifier conflict.

So the freeze is in code that runs ONLY when BasicTextField gains focus, and it's NOT cursor blink (the spin doesn't go through `delay()`), and it's NOT focus state machinery alone (focusable+clickable both work), and it's NOT a parent-gesture conflict.

**Suspect path** (`compose-multiplatform-core/.../text/CoreTextField.kt::defaultTextFieldPointer`):

```kotlin
.tapPressTextFieldModifier(interactionSource, enabled) { offset ->
    requestFocusAndShowKeyboardIfNeeded(state, focusRequester, !readOnly)
    if (state.hasFocus && enabled) {
        if (state.handleState != HandleState.Selection) {
            state.layoutResult?.let { layoutResult ->
                TextFieldDelegate.setCursorOffset(offset, ...)
                if (state.textDelegate.text.isNotEmpty()) {
                    state.handleState = HandleState.Cursor
                }
            }
        } else {
            manager.deselect(offset)
        }
    }
}
```

This `onTap` lambda is invoked **synchronously** inside `TapGesturesDetector.skiko.kt::detectTapGestures` (line 141: `onTap?.invoke(firstRelease.changes[0].position)` — no `launch`). If anything inside this synchronous block enters an infinite recompose/state-write loop, the whole `sendPointerEvent(UP)` hangs.

Top candidates for the spin:
1. `focusRequester.requestFocus()` → focus state change → recompose → SnapshotStateObserver callback → re-evaluates layer block / pointer-input / something that writes state again → infinite recompose loop. (Compose's classic "state write during recompose triggers another recompose" footgun.)
2. `TextFieldDelegate.setCursorOffset(offset, layoutResult, processor, offsetMapping, state.onValueChange)` — writes back through `onValueChange` → state mutation → recompose loop.
3. `state.handleState = HandleState.Cursor` — another state write feeding back into the same composition.
4. `Modifier.snapshotFlow { writeable }.collect { ... }` (CoreTextField line 362) — a collected snapshotFlow that re-emits constantly because `writeable` keeps changing during the synchronous onTap path.

## What's NOT the cause (ruled out)

- `delay()` busy-spinning — fixed via Delay impl, no more `delay()`-related spins.
- `EmptyPlatformTextInputService.startInput/show/hide` — all no-op overrides; can't loop.
- `detectTapGestures` itself — `Modifier.clickable` uses it and works.
- Focus state machine alone — `Modifier.focusable()` alone works.
- Cursor blink animation — no `scheduleResumeAfterDelay` logs fire during the freeze (would log if cursor blink had reached its first `delay(500)`); freeze happens before cursor blink starts.

## What to try next (out of session)

1. **Add diagnostic logs to `defaultTextFieldPointer`'s onTap closure** — log entry, after `requestFocusAndShowKeyboardIfNeeded`, after each branch, on exit. Whichever log doesn't fire identifies the line that spins.
2. **Add a Snapshot.observeReads counter** to find an infinite recompose loop. If recompose count keeps growing, that's bug.
3. **Test on a HW keyboard, no soft-keyboard** path — `Modifier.focusable() + Modifier.onKeyEvent { ... }` on a Box might be enough for v1 text input without going through BasicTextField at all.
4. **Try `BasicTextField2` (TextFieldState-based)** if it's available in this version — totally different state plumbing, may dodge the spin.
5. **Implement a real `PlatformTextInputService`** (instead of `EmptyPlatformTextInputService`) and see if the freeze is in `startInput`'s sync chain when there's nothing wired to it.

## Task #52 status

- LazyColumn part: **done** — no work needed.
- BasicTextField part:
  - Legacy API (`value, onValueChange`): **deadlock confirmed and documented**, do not use.
  - **TextFieldState API: works — render + focus clean.**
  - Hardware-keyboard / IME wiring: separate follow-up.

## How to apply

- **In Compose apps targeting wasi, ALWAYS use `BasicTextField(state: TextFieldState, ...)`** — the legacy `(value, onValueChange)` overload freezes on tap. The test app's `TextFieldCard` demonstrates the working pattern with `rememberTextFieldState("hello world")`.
- Keep the Delay impl on `WasiFrameDispatcher` indefinitely; it's correct and a real platform need.
- Material3's `TextField` / `OutlinedTextField` still use the legacy API under the hood; they'll freeze too. Use Material3 `TextField(state, ...)` overload if/when supported, OR build a Material3-style decorator manually wrapping `BasicTextField(state, ...)`.
- For typing: wire hardware key events in the test app's `SkikoInputDelegate.onKeyEvent` — build a Compose `KeyEvent` with `codePoint` from winit's `event.text` and call `realScene.sendKeyEvent(...)`. (`adb shell input keyevent KEYCODE_A` would then type "a" into a focused field.)
