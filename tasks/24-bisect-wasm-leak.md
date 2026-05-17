# Task 24 — Bisect the WasmGC-heap leak

> **Status: 🔲 scoped, not started.** Successor to task 23 once we
> confirmed the leak is in the wasmtime GC heap and rejected periodic
> `Store::gc()` as a band-aid. The goal here is to find the **actual
> retention chain** so we can file an upstream fix or work around the
> specific pattern, not mask the symptom.
>
> Companions:
> - `feedback_indeterminate_progress_leak.md` (memory) — symptom +
>   instrumented soak numbers from task 23.
> - `tasks/scope-profiling-tools.md` — broader inventory of wasm
>   profiling tools.

## What we know going in

From the task-23 instrumentation soak on Pixel 2 XL:

- **Native Heap is flat** (57.8 → 57.9 MB across 15 min). Not the
  Rust host's fault.
- **Linear memory plateaus** (last `memory.grow` event at T+437 s;
  128 MB cap). Not Kotlin's primitive-buffer allocators.
- **Anonymous mmap ("Unknown" in dumpsys meminfo) keeps climbing**
  (~8 MB/min on the default Material3 demo with no animation
  widgets). This region is the wasmtime WasmGC heap.
- Periodic `Store::gc(None)` reclaims ~94 % of the growth on a static
  demo — so the retained objects ARE genuinely garbage from
  wasmtime's perspective; nothing is holding them strongly from the
  host side. They're held strongly from inside the wasm guest.

The retained objects therefore live on the WasmGC heap (structref /
anyref). The Kotlin/Wasm and Compose stack allocates many such
objects per frame:

- Every `suspend` function compiles to a continuation state-machine
  class instance per suspension point + captures.
- Every lambda capture, every closure, every Compose composable's
  internal `MovableContent` / `RememberObserver`, etc.
- Compose's `Snapshot` system retains a `ReadObserver` / `WriteObserver`
  list per active snapshot.

One of these is being held longer than necessary. Bisect from below.

## Plan

### Step 1 — Minimal Kotlin/Wasm repro, no Compose, no skiko

Smallest possible binary that exhibits a leak. Build a separate test
target (NOT through the wart-app pipeline) whose `main()` is just:

```kotlin
import kotlin.coroutines.*
import kotlinx.coroutines.*

suspend fun main() {
    var frame = 0L
    while (true) {
        suspendCoroutine<Unit> { cont ->
            // Resume on the next "frame" — substitute the WIT frame-clock
            // import here once we wire it in, or use a synthetic ticker.
            cont.resume(Unit)
        }
        if (frame % 600 == 0L) {
            println("frame=$frame  linmem=${currentLinmemBytes()}")
        }
        frame++
    }
}
```

- Run under wart-host with the `profile` feature on; observe
  ResourceLimiter + dumpsys meminfo.
- **Expected if Kotlin/Wasm continuation codegen is the cause:**
  steady GC-heap growth, no plateau, even though semantically each
  `suspendCoroutine` should release the previous one before the next
  resumes.
- **Expected if it's not:** flat GC heap, no leak. Means we can
  narrow to kotlinx-coroutines (next step) or Compose-specific
  retention.

This is a 1-day step. Most of the work is wart-app's gradle config
producing a non-Compose component that still wires into our WIT
canvas + frame-clock surface.

### Step 2 — Add kotlinx-coroutines.withFrameNanos

If step 1 doesn't leak, swap the body for the actual
`MonotonicFrameClock.withFrameNanos { … }` pattern Compose uses:

```kotlin
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.BroadcastFrameClock

suspend fun main() {
    val clock = BroadcastFrameClock()
    val job = launch(clock) {
        var frame = 0L
        while (true) {
            withFrameNanos { /* nothing */ }
            if (frame % 600 == 0L) report(frame)
            frame++
        }
    }
    // drive clock from the WIT renderer's on-frame callback
}
```

- If THIS leaks (and step 1 didn't), the leak is in
  `BroadcastFrameClock` / `MonotonicFrameClock.withFrameNanos`'s
  awaiter list — kotlinx-coroutines callsite that registers a
  continuation per frame and may not release the old one.

### Step 3 — Add the Recomposer + an empty composable

If step 2 doesn't leak, add Compose runtime:

```kotlin
val recomposer = Recomposer(coroutineContext)
val composition = Composition(applier, recomposer)
composition.setContent { /* empty */ }
launch { recomposer.runRecomposeAndApplyChanges() }
```

- Empty `setContent {}` shouldn't generate any per-frame state.
- If THIS leaks, the leak is in Recomposer's invalidation-tracking
  or Snapshot machinery.

### Step 4 — Add a single trivial composable with no state

```kotlin
composition.setContent {
    Text("hello")  // ← just skia text via WIT
}
```

- If THIS leaks (step 3 didn't), the leak is in the
  composable→canvas-draw path. Most likely candidates: our
  `WitCanvasApplier`, the WasiDrawable transform plumbing, or the
  skiko-wasi cache layers.

### Step 5 — Heap inspection (if 1-4 don't isolate)

If we can't narrow it by source bisect, dump the WasmGC heap and
count instances by class. wasmtime exposes some of this via the
`gc_heap_inspect` feature (rust feature flag on the crate; may
require a custom wasmtime build). Plumbing:

- Enable the feature in `wart-host/Cargo.toml`.
- Wire a host-WIT call that dumps `(class_name → instance_count)`
  pairs to logcat or to `/sdcard/Download/wart-heapdump.txt`.
- Take a snapshot at T+1 min and T+15 min; diff. Whichever class
  has the most new instances is the prime suspect.

This is the heaviest step (~3 days; wasmtime feature flag may not
compile cleanly with our existing config). Only if bisect can't
narrow.

### Step 6 — Check Kotlin/Wasm changelog + upstream issues

Independent of the steps above (can be done in parallel):

- Walk the Kotlin/Wasm release notes from our version up to current
  for any "continuation retention", "wasm GC", "coroutine leak"
  fixes.
- Search KT tracker for related issues.
- If a known fix exists, bump kotlin-gradle-plugin (procedure in
  `feedback_kotlin_version_bump.md`) and re-run the task-23 soak.

## Acceptance criteria

Pick whichever applies:

- A minimal reproducer (≤200 LOC) that exhibits the leak — file
  upstream against Kotlin/Wasm or kotlinx-coroutines-wasmWasi or
  Compose-runtime.
- OR: a found-and-patched specific retention point in one of our
  forked modules (skiko-wasi, compose-multiplatform-core,
  compose-runtime-wasi) with measurements showing the leak gone.
- OR: a documented confirmation that upstream X already fixed Y and
  we should bump to version Z.

## Out of scope

- **Periodic `Store::gc()` as a fix.** Tried in task 23, deliberately
  not shipped. See `feedback_indeterminate_progress_leak.md`.
- **Replacing kotlinx-coroutines-wasmWasi with a custom dispatcher.**
  Would mask the underlying codegen issue without fixing it.
- **Rewriting Compose's frame clock.** Same — band-aid, not fix.

## Estimate

- Step 1: ~1 day (mostly gradle config for a non-Compose
  wart-app variant).
- Steps 2–4: ~0.5 day each if previous step narrowed the location.
- Step 5 (heap inspection): ~3 days if needed.
- Step 6: 1–2 hours parallelisable.

Realistic bound: **1–3 days** to a minimal reproducer; **5–7 days**
if heap inspection needed.
