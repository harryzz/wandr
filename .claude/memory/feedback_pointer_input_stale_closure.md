---
name: pointerInput(Unit) + detectTapGestures captures onClick once; use rememberUpdatedState
description: `Modifier.pointerInput(Unit) { detectTapGestures(onTap = { onClick() }) }` freezes the `onClick` lambda at the first recomposition because the coroutine started by `pointerInput` runs forever (key=Unit) and `detectTapGestures` captures `onClick` once. If the parent recomposes the lambda (e.g. a soft-keyboard key whose keyDef flips when Shift toggles), the displayed widget updates but the click handler stays bound to the original lambda — produces wrong-case letters, missing auto-unshift, "sometimes works, sometimes not" symptoms. Fix: route the lambda through `rememberUpdatedState`.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
**Rule.** When the click handler passed into a `Modifier.pointerInput(Unit) { detectTapGestures(...) }` block can change across recompositions, never capture it directly inside `onTap`. Always wrap with `rememberUpdatedState`:

```kotlin
val currentOnClick by rememberUpdatedState(onClick)
Modifier.pointerInput(Unit) {
    detectTapGestures(onTap = { currentOnClick() })
}
```

Same goes for any other parameter the closure reads (display string, codepoint, mode flag, etc.).

**Why:** `Modifier.pointerInput(key) { block }` keeps the block running until `key` changes. With `key = Unit` the block runs once for the lifetime of the node. `detectTapGestures` captures whatever `onTap` lambda was passed at the moment the block ran — that lambda is the FIRST recomposition's lambda, with FIRST recomposition's captures. Subsequent recompositions of the parent don't rebuild the pointerInput coroutine, so they don't update the captured lambda either. We were bitten on `WasiSoftKeyboard.KeyButton` 2026-05-13: Shift would visually flip the layout to uppercase but tapping a key still triggered the lowercase-keyDef's onClick (which sent the lowercase codepoint AND auto-unshifted against the stale `shifted` value).

**How to apply:** This is mandatory for any in-canvas widget where the same physical element handles different actions across recompositions — soft-keyboard keys, scoreboard cells, tappable list rows whose row index changes, dropdown items whose label changes. If you ever see a "sometimes works, sometimes doesn't, depending on what state was when first composed" bug on a `pointerInput(Unit)` modifier, this is the cause.

**Don't** switch to `pointerInput(onClick)` instead — that restarts the coroutine on every recomposition, throwing away gesture state mid-touch (drag, multi-tap counters, etc.). `rememberUpdatedState` is the correct primitive.
