---
name: Continuous-animation memory leak in wasm linear memory
description: Indeterminate Material3 ProgressIndicators (and any LaunchedEffect+withFrameNanos loop) leak ~0.4 MB/s in wasm linear memory. Likely Kotlin/Wasm codegen retaining continuation/state-machine objects. Manual Store::gc() reduces leak ~5x but its own CPU cost grows over time, making it worse than the leak for sustained sessions. Practical mitigation: prefer static widgets.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
## Symptom

Under continuous animation (e.g., Material3 `LinearProgressIndicator()` / `CircularProgressIndicator()` without `progress` parameter = indeterminate; or any `LaunchedEffect { while(true) withFrameNanos { ... } }`), the app's RSS grows linearly at ~0.4 MB/s. Doesn't plateau in 60+ second observations. With multiple indicators / heavier draw paths, leak scales linearly with draw-calls-per-frame:

| Test | Leak rate | CPU |
|------|-----------|-----|
| Static UI (Material3 `progress = { 0.5f }`) | ~0.12 MB/s | ~14% |
| `LaunchedEffect { withFrameNanos { } }`, no draw | ~0.46 MB/s | ~17% |
| Custom `Canvas` + `rememberInfiniteTransition` + 1 drawLine/frame | ~1.0 MB/s | ~24% |
| Material3 indeterminate Linear+Circular (~5 draws/frame) | ~2.0 MB/s | ~28% |

`dumpsys meminfo` localizes the growth to the **Unknown** anonymous-mmap region (~365 KB/s in pure withFrameNanos case = WASM linear memory / wasm-gc heap), while Native Heap and Graphics stay flat. So the leak lives **inside the WASM instance**, not in the Rust host or Skia GPU.

## Investigation (2026-05-13)

1. Bisect ruled out Material3-specific code (custom Canvas with one drawLine leaks ~half as much — leak scales with WIT draw calls).
2. Bisect ruled out the draw path being the only culprit (`LaunchedEffect { withFrameNanos { _ -> } }` with NO drawing leaks at 0.46 MB/s on its own).
3. Wasmtime's `Store::gc(None)` API reduces the leak from 0.46 → 0.08 MB/s when called every 60 frames (1s), confirming much of the growth IS collectable garbage. But the GC's own CPU cost climbs monotonically over time (17% → 75% in a minute) regardless of GC frequency (1s vs 5s gave the same climb). That pattern says the DRC collector scans growing numbers of **retained-but-reachable** objects — i.e., a genuine reference leak, not just unfreed garbage.
4. Most likely root cause: **Kotlin/Wasm codegen retaining references** in continuation state-machines / lambda captures during recomposition. The pattern matches a known class of suspending-coroutine bugs where the state-machine object captures more than it strictly needs and outlives the suspension. Not fixable from our Rust host.

## What works as mitigation (kept in tree)

- **Prefer static widgets where possible.** `LinearProgressIndicator(progress = { 0.5f })` instead of `LinearProgressIndicator()`. `CircularProgressIndicator(progress = { 0.5f })`. Same for any continuous-animation pattern.
- **Avoid `LaunchedEffect { while(true) withFrameNanos { ... } }`** unless absolutely needed; coalesce or use `produceState` / `animateFloatAsState` with finite durations instead of `infiniteRepeatable`.
- The cursor blink in `BasicTextField` uses `delay(500)` (suspends cleanly via `WasiFrameDispatcher.Delay`) — its allocation pressure is two wakeups per second, not 60. So it's tolerable.

## What does NOT work (tried + rejected)

- **`s.gc(None)` every 60 frames** (1Hz): reduces leak 5x to 0.10 MB/s but CPU climbs 31% → 75% in a minute. Worse than the leak for sustained sessions.
- **`s.gc(None)` every 300 frames** (5s): same trade-off — 0.08 MB/s leak, CPU climbs 25% → 78%.
- **Long observation window without manual GC**: wasmtime's automatic GC heuristic doesn't kick in within 60s. The doc claims "GC will automatically happen according to various internal heuristics" but those heuristics are tuned for short-running batch workloads, not 60fps render loops.

## Soak test 2026-05-13: Kotlin 2.4.0-Beta2 → 2.4.0-RC bump did NOT help

Re-ran the indeterminate-Material3-progress test after bumping the Kotlin Gradle Plugin from `2.4.0-Beta2` (Apr 22) to `2.4.0-RC` (May 13). Full chain rebuilt: 11 `compose-*-wasi` modules → skiko → test-app → cwasm.

Result (120 s soak, no manual GC):
- RSS: 254 MB → 550 MB over 115 s = **~2.57 MB/s** linear leak, no plateau
- CPU: stable 25-32%

Beta2 baseline measured earlier: ~2.0 MB/s. The RC's slight extra ~0.5 MB/s is likely measurement noise / slightly different codegen.

**Conclusion: the leak is NOT specific to one prerelease.** It's structural in the Kotlin/Wasm runtime's continuation-state-machine retention and Compose's recompose-per-frame allocation pattern. Bumping to RC didn't help; bumping further (when 2.4.0 final ships) is unlikely to help without an explicit fix landing for continuation retention.

## When to revisit

- If/when Kotlin/Wasm 2.4.x or 2.5.x ships fixes for coroutine state-machine retention (watch KT-* / KT-Wasm tracker for "wasm coroutine retain", "continuation leak", or similar).
- If wasmtime ships an incremental / generational GC option (current DRC is full-heap scan-on-demand and gets slower per call as the live set grows).
- If a real-world app session repeatedly hits OOM after long animation.

For now the test app uses static progress (`progress = { 0.5f }`) so it can run indefinitely without unbounded growth. The host has a long comment in `host/src/lib.rs` near the render-frame counter pointing at this memory.
