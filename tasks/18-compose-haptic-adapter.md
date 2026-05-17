# Task 18 — Compose `LocalHapticFeedback` → WIT haptics adapter

> **Status: ✅ device-verified 2026-05-17 — Material3 button click buzzes
> the Pixel 2 XL** with the expected intensity gradient
> (Confirm/click = light, LongPress = strong). Closes the loop between
> the Compose UI layer and the vendor vibrator HAL set up in task 16.

## What this task does

Bridges `androidx.compose.ui.hapticfeedback.HapticFeedback` (the
interface every Compose widget reaches through `LocalHapticFeedback`)
to our existing WIT `haptics` interface (task 16), so Compose
widgets that call `performHapticFeedback(...)` actually buzz the
device instead of no-opping.

Upstream's
`androidx.compose.ui.platform.DefaultHapticFeedback.skiko.kt` is a
literal no-op (`// TODO(demin): implement HapticFeedback`); we
replace it via `CompositionLocalProvider(LocalHapticFeedback provides
WasiHapticFeedback())` at the scene root in `RealComposeApp.kt`.

## Implementation

**`wart-app/src/wasmWasiMain/kotlin/WasiHapticFeedback.kt`** — new file.
`class WasiHapticFeedback : HapticFeedback` maps each of Compose's
13 `HapticFeedbackType` ordinals to one of our WIT `Feedback` enum
variants:

| `HapticFeedbackType` | WIT `Feedback` | AIDL `Effect` / `Strength` |
|----------------------|----------------|----------------------------|
| KeyboardTap          | Tap            | TICK / LIGHT |
| TextHandleMove       | Tap            | TICK / LIGHT |
| SegmentTick          | Tap            | TICK / LIGHT |
| SegmentFrequentTick  | Tap            | TICK / LIGHT |
| VirtualKey           | VirtualKey     | TICK / MEDIUM |
| Confirm              | Click          | CLICK / MEDIUM |
| ContextClick         | Click          | CLICK / MEDIUM |
| GestureEnd           | Click          | CLICK / MEDIUM |
| GestureThresholdActivate | Click      | CLICK / MEDIUM |
| ToggleOn / ToggleOff | Click          | CLICK / MEDIUM |
| LongPress            | LongPress      | HEAVY_CLICK / STRONG |
| Reject               | DoubleClick    | DOUBLE_CLICK / MEDIUM |
| (else)               | Click          | CLICK / MEDIUM |

The mapping is coarse — 13 Compose types → 5 WIT variants. Finer
fidelity can be added by extending the WIT enum + `haptics_impl.rs`
mapping to more AIDL `Effect` constants; not needed for v1.

Match on `HapticFeedbackType.<Name>` references (not on the int
ordinal) so a future upstream reorder doesn't silently invert the
mapping.

**`wart-app/src/wasmWasiMain/kotlin/RealComposeApp.kt`** —
`buildRealComposeScene` now installs the bridge alongside
`LocalLifecycleOwner`:

```kotlin
val hapticFeedback = WasiHapticFeedback()
scene.setContent {
    CompositionLocalProvider(
        LocalLifecycleOwner provides lifecycleOwner,
        androidx.compose.ui.platform.LocalHapticFeedback provides hapticFeedback,
    ) {
        MaterialDemoApp()
    }
}
```

That's the entire integration — every widget in the demo tree now
sees `LocalHapticFeedback.current` return `WasiHapticFeedback`
instead of the no-op default.

## Device verify

Base Material3 `Button(onClick = {})` doesn't fire haptic feedback
by default (only widgets like `Slider` with `steps > 0`, DatePicker,
and `Modifier.combinedClickable(onLongClick = ...)` do). For a
deterministic on-device test the demo's two `Button` widgets were
temporarily wired to explicit
`haptics.performHapticFeedback(HapticFeedbackType.Confirm)` and
`HapticFeedbackType.LongPress` respectively. Both buzzed with the
expected intensity gradient (Primary lighter, Secondary stronger),
confirming the chain:

```
Compose LocalHapticFeedback.current.performHapticFeedback(type)
  → WasiHapticFeedback.performHapticFeedback(type)
  → WitHaptics.Import.perform(WitHaptics.Feedback.X)
  → wart-host haptics_impl.rs
  → rsbinder → IVibrator.perform(Effect.X, Strength.Y, null)
  → vendor HAL → motor
```

After verify, the explicit calls were removed from `ButtonRow()` —
Material3 widgets that haptic-by-default will pick up the bridge
through `LocalHapticFeedback` automatically.

## Out of scope

- Per-widget haptic effects beyond the 5-variant WIT enum. Would
  need WIT enum extension + new AIDL `Effect` mappings.
- Generic vibration patterns (rhythm-style, custom waveforms). The
  existing `WitHaptics.Import.vibrateMs(durationMs)` covers the
  duration-only case.
- iOS-style haptic engine integration. Not applicable.
