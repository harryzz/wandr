---
name: Continuous-animation memory leak in wasm linear memory
description: A real WasmGC-heap reference leak in the Kotlin/Wasm + kotlinx-coroutines-wasmWasi + Compose stack. Indeterminate ProgressIndicators amplify it, but task-23 instrumentation (2026-05-17) confirmed the leak also happens with the static default Material3 demo at ~8 MB/min. Manual `Store::gc()` masks it 16× but is a band-aid, not a fix. Bisect plan to find the real root in tasks/24-bisect-wasm-leak.md.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
## Symptom

Under continuous animation (e.g., Material3 `LinearProgressIndicator()` /
`CircularProgressIndicator()` without `progress` parameter = indeterminate;
or any `LaunchedEffect { while(true) withFrameNanos { ... } }`), the app's
RSS grows linearly at ~0.4 MB/s. Doesn't plateau in 60+ second observations.

**Updated 2026-05-17 with task-23 instrumentation findings:** the leak is
NOT specific to ProgressIndicator. It also happens with the static default
Material3 demo (no PI on screen) at a more modest ~8 MB/min (~480 MB/hour
≈ 1 GB over 2 hours), matching field reports.

With multiple indicators / heavier draw paths, leak scales with
draw-calls-per-frame:

| Test | Leak rate | CPU |
|------|-----------|-----|
| **Default Material3 demo (no PI)** (added 2026-05-17) | **~0.13 MB/s** | **~15 %** |
| Static UI (Material3 `progress = { 0.5f }`) | ~0.12 MB/s | ~14% |
| `LaunchedEffect { withFrameNanos { } }`, no draw | ~0.46 MB/s | ~17% |
| Custom `Canvas` + `rememberInfiniteTransition` + 1 drawLine/frame | ~1.0 MB/s | ~24% |
| Material3 indeterminate Linear+Circular (~5 draws/frame) | ~2.0 MB/s | ~28% |

`dumpsys meminfo` localizes the growth to the **Unknown** anonymous-mmap
region — the wasmtime **WasmGC heap**, not the wasm linear memory. Native
Heap and Graphics stay flat. So the leak lives **inside the wasm GC heap**
(structref / anyref objects retained beyond their useful lifetime), not in
the Rust host or Skia GPU.

## Investigation (2026-05-13)

1. Bisect ruled out Material3-specific code (custom Canvas with one
   drawLine leaks ~half as much — leak scales with WIT draw calls).
2. Bisect ruled out the draw path being the only culprit
   (`LaunchedEffect { withFrameNanos { _ -> } }` with NO drawing leaks at
   0.46 MB/s on its own).
3. Wasmtime's `Store::gc(None)` API reduces the leak from 0.46 → 0.08 MB/s
   when called every 60 frames (1s), confirming much of the growth IS
   collectable garbage. **But see 2026-05-17 update below — the per-call
   CPU climb claim from 2026-05-13 was not reproduced.**
4. Most likely root cause: **Kotlin/Wasm codegen retaining references** in
   continuation state-machines / lambda captures during recomposition. The
   pattern matches a known class of suspending-coroutine bugs where the
   state-machine object captures more than it strictly needs and outlives
   the suspension. Not fixable from our Rust host.

## Update 2026-05-17 — task-23 instrumentation soak

Wired ResourceLimiter (`memory.grow` events with timestamps) + per-frame
host-call counter + a separately-toggleable periodic `Store::gc(None)`
behind a `profile` cargo feature, then ran two 15-minute soaks on Pixel
2 XL with **only the default Material3 demo on screen** (no PI, no
custom animation, no interaction).

### Run A — no manual gc

| Time | TOTAL PSS | Native Heap | linmem (`memory.grow` events) | Unknown (anon mmap = WasmGC heap) |
|------|-----------|-------------|-------------------------------|-----------------------------------|
| T+10s    | 165 MB | 57.8 MB | 14 events (all in first 7 s, cold-start doubling 1→512 pages) | ~93 MB |
| T+10min  | 244 MB | 57.9 MB | 16 events (+2 events: 512→1024, 1024→2048 pages = ~+96 MB) | ~156 MB |
| T+15min  | 288 MB | 57.9 MB | **16 events — NO new growth after T+437s** | **~197 MB** |

Net PSS growth: **+123 MB over 15 min (~8.2 MB/min)**. Crucially,
ResourceLimiter shows linear memory **stops growing at T+437s**
(plateaus at 2048+4 pages ≈ 128 MB), but PSS keeps climbing through
the Unknown / WasmGC-heap category. Confirms the leak is in
GC-managed objects, not in linmem allocations.

### Run B — `Store::gc(None)` every 300 frames (5 s)

| Time | TOTAL PSS | Native Heap | linmem events | Unknown | gc calls |
|------|-----------|-------------|---------------|---------|----------|
| T+10s    | 164.8 MB | 57.8 MB | 13 (last at T+261 ms) | 80 MB | 3 |
| T+10min  | 172.7 MB | 58.6 MB | 13 (unchanged) | 80.7 MB | 138 |
| T+15min  | 172.7 MB | 58.6 MB | 13 (unchanged) | 80.7 MB | 214 |

Net PSS growth: **+7.7 MB over 15 min (~0.5 MB/min)** — a **16× reduction**.

Each gc call took ~30 ms (stable across the entire 15-min run — gc #2 was
30.6 ms, gc #214 was 31.7 ms). Total gc CPU: 6.6 s over 15 min = **0.73 %
steady-state overhead** on the default demo.

### Where the 2026-05-13 note was wrong

The earlier note claimed "GC's own CPU cost climbs monotonically (17% →
75% in a minute)". Our 2026-05-17 measurements show **stable per-call
cost**: 30.6 ms at gc #2, 31.7 ms at gc #214 (15 min in). No monotonic
climb on the static default demo.

**Caveat (user-reported, 2026-05-17):** with ProgressIndicator on
screen the per-gc cost rises substantially (well above 0.7 % steady-
state overhead). The earlier note's "CPU climbs over time" observation
likely came from a PI-active soak, not the default static UI we measured
in Run B. So the GC cost is highly scenario-dependent — cheap for
static UIs, expensive for PI/animation-heavy scenarios.

## What works as mitigation (kept in tree)

- **Prefer static widgets where possible.** `LinearProgressIndicator(progress = { 0.5f })`
  instead of `LinearProgressIndicator()`. Same for any continuous-animation pattern.
- **Avoid `LaunchedEffect { while(true) withFrameNanos { ... } }`** unless
  absolutely needed; coalesce or use `produceState` / `animateFloatAsState`
  with finite durations instead of `infiniteRepeatable`.
- The cursor blink in `BasicTextField` uses `delay(500)` (suspends cleanly
  via `WasiFrameDispatcher.Delay`) — its allocation pressure is two wakeups
  per second, not 60. So it's tolerable.

## What does NOT work — periodic gc is a band-aid

Periodic `Store::gc(None)` was **tried in task 23 (2026-05-17) and
deliberately not shipped**. It reduces measured PSS growth ~16× on the
static default demo at <1 % CPU, BUT:

- **It does not fix the root cause** — Kotlin/Wasm + kotlinx-coroutines-wasmWasi
  + Compose are accumulating retained objects on the WasmGC heap that
  shouldn't be retained. Hiding the symptom doesn't address it.
- **Per-gc cost is scenario-dependent.** Static demos: cheap (~30 ms).
  PI / animation-heavy: much higher (user-reported). So a fixed cadence
  is wrong; an adaptive one is more work than the right fix.
- **The leak still wins eventually** — even 16× reduced, growth is
  ~0.5 MB/min on the static demo. Over many hours that's still trouble.

The code path was implemented behind `--features profile` (commit `bfa5992`,
2026-05-17) then reverted in the same task before merge. The diagnostic
data from the experiment is in this memory note + task 23.

## Real diagnostic path (tasks/24-bisect-wasm-leak.md)

Stop masking; find the actual retention chain. Bisect from below:

1. **Minimal Kotlin/Wasm + kotlinx-coroutines repro, NO Compose.** If a
   bare `suspend main() { while(true) { withFrameNanos {} } }` leaks,
   the bug is in continuation codegen / coroutines-wasmWasi — file
   upstream against Kotlin.
2. If (1) doesn't leak, add Compose runtime only (no composables) →
   narrows to Snapshot / Recomposer.
3. If (2) doesn't leak, add a single `LaunchedEffect` → narrows to
   Compose × coroutines integration.
4. Wasmtime gc_heap_inspect to dump live-object class counts.
5. Diff against latest Kotlin release for relevant continuation /
   coroutine fixes.

## When to revisit

- Once task 24's bisect identifies the actual retention point.
- If/when Kotlin/Wasm 2.4.x or 2.5.x ships fixes for coroutine
  state-machine retention (watch KT-* / KT-Wasm tracker for "wasm
  coroutine retain", "continuation leak", or similar).
- If wasmtime ships an incremental / generational GC option (current
  DRC is full-heap scan-on-demand and gets slower per call as the
  live set grows — though this didn't manifest in our 15-min soak).
- If a real-world app session repeatedly hits OOM after long animation.

For now the test app uses static progress (`progress = { 0.5f }`) so
it can run indefinitely without unbounded growth on the PI screen,
but the default Material3 demo still leaks ~8 MB/min on its own —
mitigation is "don't leave the app on for hours" until task 24
finds + fixes the root.
