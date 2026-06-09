---
name: kotlin-wasm-suspendcoroutine-state-machine-leak
description: "wandr-leak-repro shows ~9 MB/s WasmGC-heap growth from a bare `suspendCoroutine` loop, OOM in 6:37 on Pixel 2 XL. REVISED 2026-05-18: root cause is NOT Kotlin/Wasm codegen — same code runs fine on wasmJs because browsers self-schedule GC. Real cause is wasmtime DRC has no automatic sweep trigger; see [[wasmtime-drc-no-autoschedule]]."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ade59596-71ca-44d3-bc3e-26f4f4ba5671
---

## REVISION 2026-05-18 — root cause is NOT Kotlin codegen

The "missing slot-clear in Kotlin/Wasm codegen" hypothesis below was
**wrong**. The same `suspendCoroutine` pattern runs fine on wasmJs in
browsers — V8's tracing GC reclaims the unreachable SafeContinuations
on its own cadence. wasmtime DRC, by contrast, *never* auto-triggers a
sweep (only on memory.grow failure). With our `ResourceLimiter` always
returning `Ok(true)`, sweeps never happen unless we call
`Store::gc(None)` manually.

Codegen is correct. SafeContinuations DO become unreachable. Wasmtime
just doesn't collect them.

This was confirmed by patching wasmtime to log over-approximation list
size N and sweep duration (2026-05-18). Full chain documented in
[[wasmtime-drc-no-autoschedule]]; on-device measurements + code
analysis at `/home/harry/wandr/wasmtime-issue-draft.md`. Do NOT file
upstream against Kotlin — file against wasmtime (issue draft ready).

Keep the original analysis below for historical reference of how we
got to step 1 of task 25.

---

## What

`kotlin.coroutines.suspendCoroutine { cont -> … cont.resume(Unit) }`
called in a hot loop on the wasmWasi target leaks the
compile-generated continuation state-machine instance per call.
Each call adds a structref to the wasmtime WasmGC heap that the
collector never reclaims even after the suspension has cleanly
resumed and the source-level call frame is gone.

## Minimum reproducer

`wandr-leak-repro/` in the repo. Build deps: only
`org.jetbrains.skiko:skiko-wasm-wasi:0.0.0-SNAPSHOT`. No
Compose runtime, no Compose UI, **no kotlinx-coroutines**. The
`Main.kt` (≈ 60 LOC):

```kotlin
import kotlin.coroutines.*
import org.jetbrains.skiko.wasi.WasiScheduler

private var pendingNextFrame: Continuation<Unit>? = null
private val resumeLambda: () -> Unit = {
    val c = pendingNextFrame
    pendingNextFrame = null
    c?.resume(Unit)
}

private suspend fun awaitNextScheduledTick(): Unit = suspendCoroutine { cont ->
    pendingNextFrame = cont
    WasiScheduler.schedule(1u, resumeLambda)
}

fun main() {
    suspend {
        var frame = 0L
        while (true) {
            awaitNextScheduledTick()
            frame++
        }
    }.startCoroutine(object : Continuation<Unit> {
        override val context = EmptyCoroutineContext
        override fun resumeWith(result: Result<Unit>) = Unit
    })
}
```

`WasiScheduler.schedule(delayMs, block)` is wandr-side: stores the
lambda in a `HashMap<UInt, () -> Unit>`, calls a host import to
ask for a wake `delayMs` ms later. When the host fires back,
`WasiScheduler.fire(id)` does `callbacks.remove(id)?.invoke()`.
The HashMap doesn't grow over time (verified by reading).

## Measurements (Pixel 2 XL, LineageOS / Android 15, 2026-05-17)

Two variants tested:

| Variant | T+10 s PSS | T+2 min PSS | Leak rate | OOM time |
|---|---|---|---|---|
| Fresh `() -> Unit` lambda per tick | 146 MB | (died first) | ~9 MB/s | T+6:37 (LMK adj=0) |
| Single reused lambda | 144 MB | 1.22 GB | ~9 MB/s | killed at T+2:43 |

Tick rate: ~985 ticks/sec (no vsync — the wandr-host scheduler fires
immediately when `delayMs=1`).

**Per-tick growth: ~9 MB / 985 ticks ≈ 9 KB per `suspendCoroutine`
call.** That's the size of one Kotlin/Wasm `Continuation`-shaped
GC object plus its captures.

Native Heap stayed flat at ~8.7 MB across the entire run. `wandr-host`'s
`ResourceLimiter` saw **zero** `memory.grow` events after cold start —
the WASM linear memory does NOT grow; the leaked structref objects
are entirely in the WasmGC heap (dumpsys "Unknown" category, anonymous
mmap).

## What this isolates / rules out

The two-variant comparison rules out per-tick `() -> Unit` lambda
allocation as the leak source — same rate with one shared lambda.

Therefore the leaked object MUST be allocated by `suspendCoroutine`'s
implementation itself, not by user-level code.

`suspendCoroutine`'s impl creates the state-machine instance + a
`SafeContinuation` wrapper. Both are reachable from the parent
coroutine's continuation graph during execution; they should become
unreachable after `cont.resume(Unit)` returns control to the parent.
They don't — wasmtime's WasmGC sees them as live and never collects.

Most likely cause: **missing slot-clear / null-write in
Kotlin/Wasm-generated state-machine code** before suspension. The
JVM target nulls the slot containing the child continuation when
the parent resumes, allowing the JVM GC to collect. The Kotlin/Wasm
codegen for this target may be skipping that null-write, leaving
the slot holding a strong reference to the completed child
continuation forever.

Same class of bug previously seen in our codebase as the
`identityHashCode` reading-mutating-global bug
(`feedback_transition_animate_to_bug.md`, resolved 2026-05-13).
Kotlin/Wasm intrinsics / codegen are still maturing on this target.

## What this DOESN'T isolate

The repro uses `WasiScheduler.schedule()` to drive the wakes. If
the suspend state-machine isn't leaked but instead some other
internal reference held by the wasmtime GC keeps it alive (e.g.,
the wandr-host's binding-import argument vec), the symptom would
look identical from outside.

To rule out the wandr-side scheduler 100%, the next narrowing would
be: bypass `WasiScheduler` entirely; wire a new dedicated WIT
import that the host calls with a raw transaction code each frame,
and have our Kotlin side just block on a different continuation
slot. That's ~2 hours of additional plumbing; probably worth doing
before filing upstream so the bug report is clean.

## When this matters

- **Any continuous-animation app on wasmWasi.** Compose's
  `withFrameNanos` runs a `suspendCancellableCoroutine` per frame
  that bottoms out in the same kind of state-machine codegen.
  Every wasmWasi Compose app with continuous animation is hitting
  this leak — the wandr task-23 numbers (~8 MB/min on a static
  Material3 demo, faster with PI on screen) are this bug
  multiplied by Compose's per-frame allocation pressure.
- **Tasks 21's audio impl** uses kotlinx-coroutines internally; the
  HAL callback path will accumulate. Not currently noticeable
  because Audio is short-lived; would matter for long playback.

## Mitigation until upstream fix

- Treat any long-running `suspend` loop on wasmWasi as a memory
  hazard. Prefer non-suspending designs (`val` polling, raw
  closures, host-callback registration without `suspend`).
- Periodic `wasmtime::Store::gc(None)` masks the symptom 16× at
  ~1 % CPU on static UIs but the cost rises sharply when there's
  more live data (see `feedback_indeterminate_progress_leak.md`).
- Don't ship "use static widgets" as a permanent answer — it papers
  over the real bug.

## Filing upstream

Acceptance criteria from task 24 met: minimal reproducer < 200
LOC, reproducible OOM on a real device. Suitable for KT- /
KotlinLang YouTrack ticket with the `wasm` and `wasm-wasi`
labels. Bug class is "memory" / "GC".

Reproducer location: `wandr-leak-repro/` in the wandr repo.
Task spec: `tasks/24-bisect-wasm-leak.md`.

## When to revisit

- After filing the upstream ticket — track its status.
- When Kotlin/Wasm ships a fix for the slot-clear (look in 2.4.0
  GA / 2.4.10 / 2.5.0 release notes for "continuation retain",
  "suspend slot clear", or similar).
- If we need to ship a wasmWasi app with sustained animations
  before upstream lands a fix — would have to design around
  `suspend` loops entirely.
