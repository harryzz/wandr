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

## ✅ Step 2 — Static wasm-tools dump (done 2026-05-17)

**Result: leaked type identified as `$kotlin.coroutines.SafeContinuation`
(type-id 320). Allocation site: `$"#func573
kotlin.coroutines.SafeContinuation.<init>"` called from
`$testapp.awaitNextFrame` body, line 23607 of the WAT dump. One
SafeContinuation allocated per `awaitNextFrame()` call. Size +
DRC heap header matches our ~80-byte/tick measurement.**

Process: `wasm-tools print wart-leak-repro.wasm > /tmp/leak-repro.wat`
gave a 24,929-line WAT decompilation. Filtered to:

| Type | Type-id | Fields | Est. size | `struct.new` sites | Allocated per |
|---|---|---|---|---|---|
| `$kotlin.coroutines.SafeContinuation` | 320 | vtable, itable, rtti, _hashCode, **delegate, result** (6) | ~44 B payload + ~24 B DRC header = **~68-80 B** | **2** (both in its `<init>` variants 572 + 573) | **`awaitNextFrame` call → per-tick** |
| `$testapp.$invokeCOROUTINE$` | 422 | CoroutineImpl base + `<this>` (13) | ~104 B | 1 (in `$<init>`) | one-shot — called from `main$slambda.invoke` which `startCoroutine` calls ONCE |
| `$testapp.main$slambda` | 420 | base only (4) | ~32 B | 1 (in `$<init>`) | one-shot — `fieldInitializer` (module init) |
| `$kotlin.coroutines.intrinsics.<no name provided>` | 323/326 | CoroutineImpl + 2 (14) | ~112 B | — | not directly called from our hot path |

The `awaitNextFrame()` body itself:

```wat
(func $testapp.awaitNextFrame ...
    (local $safe (ref null $kotlin.coroutines.SafeContinuation))
    ...
    ref.null none           ;; <this> = null → triggers struct.new
    local.get $~c           ;; delegate
    call $kotlin.coroutines.intrinsics.intercepted
    call $"#func573 kotlin.coroutines.SafeContinuation.<init>"
    local.tee $safe
    ...
    local.get $safe
    global.set $testapp.pendingNextFrame
    ...
    local.get $safe
    call $kotlin.coroutines.SafeContinuation.getOrThrow  ;; returns SUSPENDED
    ...
)
```

And the constructor at line 16940:

```wat
(func $"#func573 kotlin.coroutines.SafeContinuation.<init>"
    ...
    local.get $<this>
    ref.is_null
    if  ;; <this> was null → allocate
        ...
        struct.new $kotlin.coroutines.SafeContinuation
        local.set $<this>
    end
    ...
)
```

The CoroutineImpl `$invokeCOROUTINE$` is allocated once (verified
by tracing its `<init>` caller = `main$slambda.invoke` = called
once by `startCoroutine`), so the state-machine itself is reused
across iterations as expected. The leak is purely SafeContinuation.

**Where is the retention chain?** Reading
`SafeContinuation.resumeWith` (line 16978): the resumeWith call
path is `tick() → safe.resumeWith(Unit) → delegate.resumeWith(Unit)`
where `delegate` is the parent CoroutineImpl. After resumeWith
returns, the SafeContinuation should be unreachable: no `delegate`
field of any persistent object points BACK to the SafeContinuation,
and `pendingNextFrame` was nulled in `tick()`.

That means the leak is at the **wasmtime DRC-heap level**, not in
the Kotlin source-level allocations: wasmtime is failing to
decrement the SafeContinuation's refcount on function return.
Specifically, when `SafeContinuation.resumeWith` returns, its
local `$~tmp0_<this>` (containing the SafeContinuation ref) and
`$~tmp` (containing the delegate ref) should both trigger
DRC decrements. The fact that they don't = either:

(a) **Kotlin/Wasm codegen doesn't emit explicit slot-clears before
function returns**, expecting the runtime to do it; wasmtime's
DRC expects explicit clears, expecting the compiler to emit them.
Classic "who's responsible for the decrement" interface bug.

(b) **wasmtime's DRC doesn't drain locals on function return** —
it relies on the wasm runtime to clear locals, but Kotlin/Wasm
takes the responsibility-shifted position.

Either way, this is below the Kotlin source language. Step 3
(wasmtime live-object summary) will confirm which side is the
real culprit. Step 4 will either propose the Kotlin codegen patch
or a wasmtime DRC fix depending on what step 3 shows.

### Per-tick size math

`$kotlin.coroutines.SafeContinuation`:
- vtable, itable, rtti (3 × 8 B = 24 B) — Kotlin.Any base header
- _hashCode i32 (4 B)
- delegate, result (2 × 8 B = 16 B)
- Subtotal: 44 B payload
- + wasmtime DRC header (~16-24 B)
- = **60-68 B per instance**
- With heap allocation alignment to 8 B = **~80 B**

Matches our measured per-tick leak rate of ~80 B exactly.

## 🟡 Step 3 — Exploration changed the diagnosis (paused 2026-05-17)

**Before committing to the wasmtime patch, I read wasmtime DRC's
implementation more carefully. The diagnosis is not what I thought.
The wasmtime patch would still confirm SafeContinuation as the
leaked type but the FIX landscape has changed — patching wasmtime
introspection isn't on the critical path anymore.**

### Key finding from reading `runtime/vm/gc/enabled/drc.rs` preamble

> "we only mutate reference counts when storing `VMGcRef`s somewhere
> that outlives the Wasm activation: into a global or table"
> "we over-approximate the set of `VMGcRef`s that are inside Wasm
> function activations"
> "Periodically, we walk the stack at GC safe points, and use stack
> map information to precisely identify the set of `VMGcRef`s
> inside Wasm activations. Then we take the difference between this
> precise set and our over-approximation, and decrement the
> reference count for each of the `VMGcRef`s that are in our
> over-approximation but not in the precise set."

**DRC is lazy-decrement by design.** It doesn't dec-ref on every
`local.set` (that would defeat the "deferred" optimization). It
builds an over-approximation set and processes it at GC safe
points. Without explicit `Store::gc()`, the over-approximation
just grows.

### And the heap-grow logic at `runtime/store/gc.rs`

```rust
async fn grow_or_collect_gc_heap(...) {
    if let Some(n) = bytes_needed && n > 0 {
        if self.grow_gc_heap(limiter, n).await.is_ok() { return; }
    }
    self.do_gc(asyncness).await;  // gc only if grow failed
}
```

**Wasmtime tries to grow first; gc only fires when grow can't.**
The hard ceiling for the GC heap is `1 << 32 = 4 GB` (hardcoded
in `store/gc.rs:102`). On a 4 GB Pixel 2 XL, that ceiling is the
device's physical memory — the heap just keeps doubling toward
that, and the device's lowmemorykiller fires before wasmtime hits
the ceiling and would auto-gc.

### Revised diagnosis

The "leak" is NOT a missing slot-clear in Kotlin/Wasm codegen as
step 2 hypothesized. It's the **DRC over-approximation set
growing unboundedly because:**

1. DRC defers decrements (lazy by design).
2. Wasmtime's heap auto-grow policy prefers grow-over-gc.
3. The 4 GB hardcoded ceiling never gets hit on a 4 GB device —
   the OS kills us first.

This is below the Kotlin source language. It's a
**wasmtime architecture choice that's pathological for
high-allocation-rate workloads on memory-constrained devices.**

### Fix landscape (revised)

| Path | What | Effort | Trade-offs |
|---|---|---|---|
| **A** | Manual `Store::gc()` every N frames (band-aid we already tried) | Done | 16× reduction at 0.7 % CPU on static; user reported PI scenario costs higher |
| **B** | Have Kotlin/Wasm avoid per-call `SafeContinuation` allocation | Months (Kotlin compiler work) | Best long-term but huge effort |
| **C** | Patch wasmtime's heap-grow-vs-gc policy to prefer gc once heap ≥ N MB | ~1 day | Cleanest local fix, configurable. Our patch lives forever as a wasmtime fork. |
| **D** | Switch to wasmtime's mark-and-sweep collector | Investigation needed | Different latency/throughput trade-off |

The original step 3 (wasmtime live-object summary) would still
work — give us per-type counts confirming SafeContinuation —
but doesn't change which path A/B/C/D we pick. Pausing here to
let the user decide which fix path before sinking 2-3 days into
introspection.

---

## Step 3 — Patch wasmtime for live-object summary (~2-3 days, paused)

**Original goal: per-type live-count growth data — the actual smoking gun.**

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
