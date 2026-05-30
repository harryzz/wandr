---
name: IME / soft-keyboard options for wasi text input
description: Four practical paths to soft-keyboard support beyond the hardware-keyboard wiring already in tree. Compose's integration point is `PlatformContext.startInputMethod(PlatformTextInputMethodRequest)`. Recommended: combine in-canvas hand-rolled keyboard (no JNI) with optional JNI-to-InputMethodManager upgrade path.
type: reference
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
## Current state (2026-05-13)

What's already working:
- **Hardware keyboard** via `on-key-event-v2(kind, code-point, key-id)` WIT call + `WasiInput.setKeyHandler`. `adb shell input keyevent KEYCODE_A` types into a focused `BasicTextField(state: TextFieldState, ...)`. USB / Bluetooth keyboards also work.
- **`BasicTextField(state: TextFieldState, ...)`** renders, focuses cleanly, and consumes hardware keys end-to-end.

What's NOT wired (this memo's subject): on-screen soft keyboard. On a phone without a physical keyboard, tap focuses the field but no virtual keyboard appears.

## Compose's integration point

`PlatformContext.startInputMethod(request: PlatformTextInputMethodRequest)` is the single funnel — the API every `BasicTextField(state, ...)` ultimately calls when focus is acquired. Our current `PlatformContext` returns the default (no-op). To wire IME we provide a custom override.

File reference:
- `compose-multiplatform-core/compose/ui/ui/src/skikoMain/.../PlatformContext.skiko.kt::startInputMethod` — default no-op
- `.../PlatformTextInputMethodRequest.skiko.kt` — the request object: text-field state snapshot, `onEditCommand(List<EditCommand>)`, IME action callback, `focusedRectInRoot`, `editText(block)` — everything an IME needs to query / mutate the text field
- `.../RootNodeOwner.skiko.kt::startInputMethod` — forwards to `platformContext.startInputMethod(request)`, called from the per-component `textInputSession` scope

So no matter which IME approach we pick, the wiring point is the same: provide a custom `PlatformContext` (we already do this in `buildRealComposeScene` for `WindowInfo.containerSize` — task #50) whose `startInputMethod` delegates to our backend.

## The four options

### Approach 1 — JNI bridge to Android's `InputMethodManager` (the "proper Android way")

Use the system soft keyboard (Gboard, SwiftKey, voice input, emoji picker, autocorrect, etc.). Industrial-strength Android text input.

**What's needed:**
- Rust-side JNI binding (`jni` crate) OR a small Java helper class compiled into the host APK
- Java helper: a `View` subclass that owns a `BaseInputConnection` (or full `InputConnection`) — Android's system requires a View to attach the IME to. Our `NativeActivity` doesn't have one by default; we'd add a hidden 0×0 View just to host the connection
- `InputMethodManager.showSoftInput(view, 0)` to pop the keyboard
- `BaseInputConnection.commitText(text, newCursorPosition)` from `onUpdateSelection` → forward through WIT to wasm → call `PlatformTextInputMethodRequest.editText { commitText(text) }`
- IME insets via `WindowInsetsCompat.getInsets(Type.ime())` on resume / `OnApplyWindowInsetsListener` — Compose's `WindowInsets.ime` would need a wasi actual feeding off this

**Effort:** 1-2 days. Mostly the Java InputConnection + JNI plumbing. The wasi-side `PlatformContext.startInputMethod` impl is mechanical.

**Pros:** Full system IME — gestures, voice, emoji, autocorrect, accessibility, dark mode, all non-Latin scripts via the user's installed IME, system-wide undo, etc.

**Cons:** Adds first JNI dependency to host (presently zero Java code in the APK; we use `cargo apk` + winit's native activity). Adds insets handling (otherwise text field hides behind the keyboard). Soft-keyboard auto-show/hide rules differ across Android versions; need testing.

### Approach 2 — Stay hardware-keyboard-only (current state)

No additional work. Works for:
- USB / Bluetooth keyboards
- Pen-input devices that emit hardware key events
- Developer workflow via `adb shell input keyevent` / `adb shell input text`

**Cons:** Phones without a HW keyboard can't type. Not viable for end-user apps that need text input on stock devices.

### Approach 3 — In-canvas hand-rolled keyboard (Compose-only, no JNI)

A Compose composable rendered as an overlay on our skia canvas. Tapping a key sends a synthetic `KeyEvent` (or directly emits an `EditCommand`) into the focused `BasicTextField`. The "keyboard" is just Compose layout + buttons drawn on the same canvas as the rest of the UI.

**What's needed:**
- A `WasiSoftKeyboard()` composable: 4×10ish grid of `Box(Modifier.clickable { ... })`, sized to ~40% of the bottom of the screen
- Captured at app root: when any TextFieldState gains focus, show the keyboard; on blur, hide it
- Custom `PlatformContext.startInputMethod` that:
  - Posts a `LocalKeyboardVisible.value = true` flag
  - On `onKeyTap`, forwards via `request.editText { commitText(char) }` or `request.editText { delete(1) }` for backspace
  - Bypasses the system entirely — no JNI, no InputConnection

**Effort:** ~1 week for a usable US-ASCII layout. Add shift/symbols layers (~1 day). Non-Latin scripts require per-locale layouts.

**Pros:** Pure Kotlin/Compose work. Zero JNI. Looks consistent with the app's theme. No insets headaches (the keyboard IS part of the app canvas — we know where it is).

**Cons:** Can't use system IMEs — no Gboard suggestions, no voice input, no emoji picker, no swipe typing, no accessibility hooks. Non-Latin scripts need hand-rolled IME logic (impractical for languages with complex composition like CJK).

### Approach 4 — Compose's experimental `PlatformTextInputModifierNode` hooks

Compose 1.7+ has a per-node `textInputSession` mechanism (lives in `BasicTextField(state, ...)`'s machinery). A child composable can hook the input session directly via `Modifier.semantics { setText { ... } }` or by providing its own `PlatformContext`. Useful for unit-testing text input, but the actual IME backend still needs to be one of (1) or (3).

**This isn't a fourth option so much as an integration aid for either #1 or #3** — it lets you scope IME behavior per text field rather than globally. Mention here because it might come up; not a standalone path.

## Recommendation

Combine **Approach 3 (in-canvas keyboard)** as the default backend with **Approach 1 (JNI)** as an opt-in upgrade for apps that need a system IME. Specifically:

1. **Phase 1 — in-canvas keyboard:** Build a `WasiSoftKeyboard` composable + custom `PlatformContext.startInputMethod` impl that pops the keyboard when a text field gains focus. ~1 week. Unblocks all phone form-factor text input for English. Add multi-locale layouts on demand.
2. **Phase 2 — JNI to InputMethodManager:** Add the Java helper class + IME insets WIT export. Apps can opt in by providing `PlatformContext.startInputMethod = SystemInputMethod` instead of `InCanvasKeyboard`. ~1-2 days once the integration point exists.
3. **Both can coexist** — `BasicTextField(state, ...)` calls `startInputMethod` regardless of which backend is wired. Apps decide.

Hardware keyboard (already wired) works alongside any of these — typing on a Bluetooth keyboard while the soft keyboard is visible is fine; both feed `EditCommand`s into the same `TextFieldState`.

## When to revisit

- After a real app demands non-English text input (CJK / Devanagari / Arabic) — that pushes us toward JNI sooner rather than later.
- After a UX bug appears where the in-canvas keyboard's hit-test conflicts with `Modifier.scrollable` / nested scroll (would need to flip to system IME).
- If `cargo apk` ever ships an easier Java-file integration story (currently it's `package.metadata.android` config in Cargo.toml + a thin Gradle layer; doable but not as smooth as bare Rust).

## What stays unchanged regardless of which path we pick

- Hardware key path (`on-key-event-v2` + `WasiInput.setKeyHandler`) — keep it.
- `WasiFrameDispatcher.Delay` impl for cursor blink — keep it.
- `BasicTextField(state: TextFieldState, ...)` (the new API) as the recommended TextField — legacy `(value, onValueChange)` still freezes on tap.
- Material3 `TextField` / `OutlinedTextField` — still wrap the legacy API; still freeze. Do not use yet.
