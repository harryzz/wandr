---
name: currentNanoTime traps subsequent WIT imports on wasmWasi
description: org.jetbrains.skiko.currentNanoTime() pollutes reallocAllocator and even explicit freeAll() afterwards doesn't clear it; avoid calling it in code that subsequently makes WIT imports
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
`org.jetbrains.skiko.currentNanoTime()` is implemented via Kotlin stdlib's
`TimeSource.Monotonic.markNow().elapsedNow()`, which on wasmWasi calls
`wasi:clocks/monotonic-clock.now`. That call pollutes `reallocAllocator`
similar to `wasiRandomGet` (see `feedback_wasi_realloc_allocator.md`), but
**unlike wasiRandomGet, an explicit
`kotlin.wasm.unsafe.freeAllComponentModelReallocAllocatedMemory()` call
AFTER `currentNanoTime` does NOT clear the state**. The next WIT import
traps with "Can't create new allocators while realloc-allocated memory is
not freed" thrown from `kotlin.wasm.unsafe.createAllocatorInTheNewScope`.

Reproduced 2026-05-11 during Stage-2 scheduler smoke test in test-app/Main.kt:
- `WitWindow.Import.getDensity()` → works
- `WitCanvas.Import.logMessage(...)` → works
- `currentNanoTime()` → no crash yet
- next `WitCanvas.Import.logMessage(...)` → **traps** even with `freeAll()` between

**Why:** Probably because `TimeSource.Monotonic.markNow()` and `elapsedNow()`
together hold a non-droppable allocation reference, not just leftover realloc
state. The defensive freeAll() works for the WASI random case because that's
a single one-shot host call; `TimeSource.Monotonic` keeps internal state.

**How to apply:** Do NOT call `currentNanoTime()` (or any `TimeSource.Monotonic`
operation) in code paths that subsequently make WIT imports — especially in
`main()` or composable setup, where it'll trap render_frame #0. For elapsed
time measurements, either:
  - rely on the host's `render_frame` log timestamps (in logcat), or
  - add a `wasi:android-clock` WIT export that returns ms from the host
    (host has clean access to `Instant::now()` without polluting wasm state)

If a future investigation finds the root cause, this memory should be
updated or replaced.
