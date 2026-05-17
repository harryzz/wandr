# Task 25 — Diagnose the Kotlin/Wasm `suspend` state-machine leak

> **Status: 🔲 scoped, step 1 starting 2026-05-17.** Successor to
> task 24. Step 1's repro (`wart-leak-repro/`) confirmed the leak is
> in Kotlin/Wasm `suspendCoroutine` state-machine codegen at the
> source-language level. This task narrows from "in `suspendCoroutine`"
> to "in this specific allocation site emitted by this specific
> kotlinc lowering pass, leaking this specific structref type."
> Output: enough evidence to either file a focused upstream patch OR
> commit to one of the local-fix paths.
>
> Companions:
> - `feedback_kotlin_wasm_suspendcoroutine_leak.md` (memory) — symptom
>   + measurements from task 24 step 1.
> - `tasks/24-bisect-wasm-leak.md` — bisect plan that landed step 1.
> - `wart-leak-repro/` (codeberg: harryzz/wart-leak-repro) — the
>   minimum-possible reproducer this task tightens further.

## What this task is and isn't

**Is:** an investigation. Output is *evidence*, not a fix. Five days
of diagnosis ending with one of:
- "the slot-clear is obviously missing at kotlinc-wasm-backend
  line N — here's a one-file patch we'd submit",
- "the leak is structural in the lowering — here's the right local
  workaround design", or
- "ran out of signal at step 3, suggest other angles".

**Isn't:** the fix itself. Filing upstream vs restructuring
compose-runtime-wasi vs wasm-postprocess-tool are all downstream
decisions informed by this task's findings.

## ✅ Step 1 — Tighten the reproducer (done 2026-05-17)

**Result: leak rate matches the step-1-of-task-24 measurement
with WasiScheduler now ELIMINATED from the suspend cycle. Per-tick
leak drops from ~9 KB (with the WasiScheduler lambda+HashMap) to
**~80 bytes** — essentially the size of one Kotlin/Wasm
`Continuation` state-machine object. Net rate ~10 MB/s.**

Measurements (Pixel 2 XL, 30-second run):

| Metric | Task 24 step 1 (WasiScheduler) | Task 25 step 1 (tightened) |
|---|---|---|
| Per-tick allocation | ~9 KB | **~80 bytes** |
| Tick rate (host-driven) | ~985/s | **124,320/s** (uncapped renderFrame) |
| Net leak rate | ~9 MB/s | **~10 MB/s** |
| Components in cycle | suspendCoroutine + WasiScheduler + HashMap + `()->Unit` lambda | **`suspendCoroutine` ONLY** |

The 80-byte/tick figure is the smoking gun: the wart-side
HashMap-based callback and per-tick `() -> Unit` lambda accounted
for ~9 KB of the original per-tick leak (transient allocations
that DID get reclaimed eventually); the underlying retained
object is just ~80 bytes — exactly the size of one state-machine
struct with a header + a few fields.

### How the tightened repro works

`Main.kt` defines `LeakReproDriver.tick()`. `RendererExports.kt` is
patched so `__wasm_export_renderFrame` calls `LeakReproDriver.tick()`
instead of `RendererImpl.renderFrame(...)`. The suspend cycle now is:

1. Coroutine calls `awaitNextFrame()` — `suspendCoroutine` allocates
   a state-machine instance, stores `cont` in `pendingNextFrame`,
   returns.
2. Host's render loop fires `render-frame(nanos)`.
3. Our patched `__wasm_export_renderFrame` calls
   `LeakReproDriver.tick()`.
4. `tick()` drains `pendingNextFrame`, calls `cont.resume(Unit)`.
5. Step 1 repeats.

ZERO wart-side allocation in the cycle. No HashMap, no `() -> Unit`
lambda, no WasiScheduler ID generation. Just `suspendCoroutine`.

### Native Heap stayed flat

8.5 MB before, 8.5 MB after 30 seconds. The host side is not in
the picture at all.

### ResourceLimiter saw zero `memory.grow` events

After the cold-start burst, wasm linear memory plateaued. Every
byte of the +292 MB growth in 30 s lives in the wasmtime WasmGC
heap (anonymous mmap, dumpsys "Unknown" category).

### Conclusion of step 1

**The leak is in `kotlin.coroutines.suspendCoroutine`'s
state-machine codegen on the wasmWasi target.** Every call
allocates ~80 bytes of structref that the WasmGC collector never
reclaims, even after `cont.resume(Unit)` completes the suspension
and the source-level call frame returns.

Steps 2-4 of this task remain — we now know *that* a state-machine
struct leaks; we still need to identify *which* type-index it is
in the .wasm and *where* in the kotlinc-wasm lowering it gets
allocated without slot-clear.

---

## Step 1 — Tighten the reproducer (original plan, kept for reference)

**Goal: provably 100% Kotlin/Wasm. Eliminate WasiScheduler.**

The task-24 reproducer uses `WasiScheduler.schedule(1u, lambda)` to
park-and-wake the continuation. `WasiScheduler` is wart-side code
(skiko-wasi); even though we read it and it looks clean, an upstream
report should not have to take our word for it.

Replace with a direct in-Kotlin park/resume that needs no callback
registration at all:

```kotlin
// Drive ticks straight from the WIT renderer's render-frame export.
// No HashMap, no callback IDs, no host-side scheduler involvement.
private var pendingNextFrame: Continuation<Unit>? = null

internal object LeakReproDriver {
    fun tick() {
        val c = pendingNextFrame
        pendingNextFrame = null
        c?.resume(Unit)
    }
}

private suspend fun awaitNextScheduledTick(): Unit = suspendCoroutine { cont ->
    pendingNextFrame = cont
    // NOTHING ELSE. The wart-host already calls our @WasmExport
    // renderFrame every frame; that's our wake.
}
```

Then in `generated/RendererExports.kt`, replace the body of
`__wasm_export_renderFrame` to call `LeakReproDriver.tick()` instead
of `RendererImpl.renderFrame(...)`.

After this, the entire suspend cycle is:
1. Coroutine calls `awaitNextScheduledTick()`.
2. `suspendCoroutine` allocates a state-machine instance, stores
   `cont` in `pendingNextFrame`, returns.
3. Host's next renderFrame call hits our `__wasm_export_renderFrame`.
4. That calls `LeakReproDriver.tick()` which calls `cont.resume(Unit)`.
5. Coroutine resumes, loops back to step 1.

No HashMap. No WasiScheduler.fire. Pure Kotlin/Wasm + a single
global `Continuation<Unit>?`. If THIS leaks at the same rate as
step 1, the bug is irrefutably in Kotlin/Wasm codegen.

Acceptance: 60-second on-device run shows the same PSS growth slope
as the step-1 reproducer (~9 MB/s).

## Step 2 — Static wasm-tools dump (~half day)

**Goal: identify candidate struct types declared by Kotlin/Wasm for
the suspend state machine.**

```sh
wasm-tools dump wart-leak-repro.wasm > leak-repro.dump.txt
# Look for `(struct ...)` declarations near function-name suspend-state-machine
# patterns: SafeContinuation, CoroutineImpl, $main$1, etc.
grep -nE '\(struct|SafeContinuation|CoroutineImpl|\$lambda\$' leak-repro.dump.txt
```

Cross-reference with `struct.new` instructions inside the body of
`__wasm_export_renderFrame` and the `awaitNextScheduledTick` lowered
function. Output: short list (≤5) of candidate Kotlin classes the
leak could be backed by.

Acceptance: a list of class-name → type-index pairs, with brief
notes on which are most likely the leaker based on call-site
proximity to the suspend boundary.

## Step 3 — Patch wasmtime for live-object summary (~2-3 days)

**Goal: per-type live-count growth data — the actual smoking gun.**

Wasmtime's DRC (deferred-reference-counting) heap impl at
`runtime/vm/gc/enabled/drc.rs` maintains internal tables of all
live `VMGcRef` headers. Add a small extension:

```rust
// In wasmtime/src/runtime/vm/gc.rs or similar.
impl GcStore {
    /// Walk the live set, tally instances per type index.
    /// Cheap because the DRC heap already has this as an internal map.
    pub fn live_summary(&self) -> HashMap<u32 /*type_index*/, usize> {
        // ...
    }
}

// In wasmtime/src/runtime/store.rs.
impl<T> Store<T> {
    pub fn gc_live_summary(&mut self) -> Result<HashMap<u32, usize>> {
        // force a gc first so we don't tally garbage
        self.gc(None)?;
        Ok(self.inner.gc_store_mut()?.live_summary())
    }
}
```

`wart-host` already pulls wasmtime via cargo registry. To use our
patched version, point `[dependencies] wasmtime = { path = "..." }`
at a local checkout. Easy revert after the experiment.

Then wire it into the `profile` cargo feature: every 60 frames, log
the top 5 type-indices by count and their deltas vs the previous
log. The type with monotonically-growing count is the leaker.

Cross-reference with the type-name list from Step 2 (we'll need to
map wasm type-index back to Kotlin class name — wasm-tools dump
lists them in order, so this is a lookup table).

Acceptance: a single class name (e.g.
`kotlin.coroutines.intrinsics.SafeContinuation` or
`Main$awaitNextScheduledTick$1`) with growth rate matching the
~9 KB/tick math.

## Step 4 — Read kotlinc-wasm-backend codegen (~1 day)

**Goal: locate the lowering pass that allocates the leaked type and
diagnose what's missing.**

Kotlin compiler source layout (open-source, github.com/JetBrains/kotlin):
- `compiler/ir/backend.wasm/` — wasm backend
- `compiler/ir/backend.wasm/src/org/jetbrains/kotlin/backend/wasm/lower/` — IR-to-wasm lowering passes

Likely passes that allocate suspend-state-machine structs:
- `SuspendFunctionsLowering`
- `CoroutineIntrinsicsLowering` (or whatever the wasm equivalent is named)
- `JsIrInliner` (analogously for wasm)

For each candidate pass, find where it emits the `struct.new`
allocation. Look for a paired slot-clear / null-write at the
suspension boundary. If absent — that's our bug.

The JVM backend has the equivalent slot-clear (search for
`uncheckedNullOut` or `clearLocalsBeforeReturn` in jvm-lower). The
wasm backend may be missing it.

Acceptance: a `git log -p` diff against the kotlin repo showing
either (a) "this file is missing the slot-clear that exists at
jvm-lower line N" with a proposed one-file patch, or (b) "the
lowering is structurally different — here's why slot-clearing
isn't trivially applicable, and here's the design alternative."

## Out of scope

- **Implementing the fix.** This task identifies; downstream tasks
  fix. Whether to file upstream vs restructure compose-runtime-wasi
  vs wasm-postprocess is a separate decision informed by this task's
  output.
- **GC-heap inspection in production.** The wasmtime patch is for
  diagnosis only — not shipped in the default `wart-host` build.
- **General-purpose Kotlin/Wasm profiler.** We're using just enough
  tooling to identify this one leak; not building a permanent
  profiler infrastructure.

## Estimates

Realistic bound: 5-7 days of focused work. Step 3 is the biggest
unknown — wasmtime internals are well-documented but DRC heap impl
may have subtleties. If step 3 stalls past 3 days, fall back to
"step 2 + step 4 only" and accept higher uncertainty in which type
is the leaker.

Step 1 (today): tightened reproducer. Acceptance is just "same leak
rate as step-1 of task 24". If THAT fails (no leak with the
direct-call driver), we accidentally indicted Kotlin when the bug
was in WasiScheduler — restart task 24's bisect from there.
