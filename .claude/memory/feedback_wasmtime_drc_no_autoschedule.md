---
name: wasmtime-drc-no-autoschedule
description: "Wasmtime DRC had no auto-sweep — the wandr-app \"memory leak\" is GC scheduling, not Kotlin/Compose. RESOLVED 2026-05-30: wasmtime 45.0.0 (#13422 auto-GC + #13450 array.copy fix + heuristics) bounds the leak AND — combined with task 64 cutting Compose render 60→1fps — no longer ANRs. Re-tested device 60fps+scroll: RSS flat ~220MB, smooth, no ANR. ADOPT 45. (Earlier May-21 #13422-only test ANR'd; superseded.)"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ade59596-71ca-44d3-bc3e-26f4f4ba5671
---

The wandr-app "memory leak" is not in Kotlin codegen and not in Compose
retained state. It is in wasmtime DRC scheduling.

Causation chain (top to bottom):

1. Compose's `withFrameNanos` (animations, recomposition) compiles to
   `suspendCoroutine`.
2. Kotlin/Wasm codegen allocates a fresh
   `kotlin.coroutines.SafeContinuation` per call (~80 B incl. DRC
   header). Confirmed via wasm-tools dump (task 25 step 2). See
   [[kotlin-wasm-suspendcoroutine-leak]].
3. SafeContinuation becomes unreachable when the frame resumes
   (`pendingNextFrame` global cleared).
4. Wasmtime DRC *defers* the decrement to the next sweep — by design.
5. **Wasmtime DRC has no automatic sweep trigger.** The only auto-path
   is `grow_or_collect_gc_heap`, which fires only on `memory.grow`
   *failure*. `ResourceLimiter::memory_growing` returning `Ok(true)`
   (safe default) → grow always succeeds → sweep never runs → dead
   SafeContinuations pile up indefinitely.
6. Result without manual gc: ~9 MB/s WasmGC heap growth + ~15–19K
   refs/s growth in the over-approximation linked list. Eventually
   OOMs the device.

**Why:** confirmed on-device via patched-wasmtime instrumentation
(2026-05-18) logging N (over-approx list size), F (free-block count),
and sweep duration. Sweep trajectory: 478 → 1248 → 3000 ms over 45
min, N grew 1.2M → 2.3M → 4.6M, per-entry sweep cost climbed 0.39 →
0.65 μs/entry (cache-miss bound walk of scattered headers). Same
allocation pattern runs fine on wasmJs in browsers because V8's
tracing GC self-schedules. Upstream issue filed as
[bytecodealliance/wasmtime#13403](https://github.com/bytecodealliance/wasmtime/issues/13403);
full trajectory data + code analysis at
`/home/harry/wandr/wasmtime-issue-draft.md`; artifacts (patch diffs,
logcat, reproducer) at `/home/harry/wandr/wasmtime-issue-artifacts/`.

**How to apply:** treat any "Kotlin/Wasm leaks memory" report as a
scheduling problem first, not a Kotlin or Compose bug. The mitigation
is to call `Store::gc(None)` periodically — our `profiling.rs`
deferred-gc trigger does this, keyed off
`ResourceLimiter::memory_growing` ≥ 64 MB threshold. Each manual sweep
is expensive and grows in cost over time (O(N) linked-list walk on
object headers scattered across the GC heap, plus N × O(log F)
deallocations), but without it the heap grows unbounded. The
load-bearing upstream fix is auto-scheduling — see the issue draft's
"What would help" item 1. Allocator-side fixes alone make things
worse: see [[drc-first-fit-alone-backfires]]. But auto-scheduling
alone is also not enough — see the UPDATE below.

---

## UPDATE 2026-05-21 — PR #13422 (the auto-scheduling fix) tested: NOT a clean fix

fitzgen's [wasmtime#13422](https://github.com/bytecodealliance/wasmtime/pull/13422)
adds the auto-GC trigger this memory called the "load-bearing upstream
fix": the DRC collector force-triggers a GC when the over-approximated-
stack-roots (OASR) list reaches 2× its size after the last GC (floor
1024). Tested in two stages.

- **Stage 1 (desktop, wasmtime CLI):** fixes the leak cleanly. The
  standalone `wandr-leak-repro.wasm` goes from the 4 GB-ceiling climb to
  RSS flat at ~43 MB. Sweeps stay tiny.
- **Stage 2 (device — wandr-host rebuilt on wasmtime 46 + #13422; the
  44→46 bump needed zero wandr-host code changes):** fixes the *original*
  ANR (sweep-duration growth — sweeps stay flat at ~1 ms) **but
  reintroduces an ANR via a new mechanism.** The PR emits an inline
  `force_gc` call in the wasm codegen; on the Kotlin/Wasm+Compose guest
  it fires very frequently. The sweep is cheap, but each GC's *root
  scan* — `trace_vmctx_roots` → per GC-typed global `Global::trace_root`
  → `RegisteredType::root`/`from_parts` (type-registry work) — is not,
  and the guest has many GC globals. Cumulatively the render thread
  spends most of its time in `force_gc` → lag; a heavy frame's clustered
  `force_gc` calls block it > 5 s → input-dispatch ANR. Confirmed by the
  ANR trace: render thread stuck in
  `force_gc → do_gc → trace_vmctx_roots → RegisteredType::root`.

**Conclusion:** auto-scheduling is *necessary but not sufficient*. As-is,
#13422 trades unbounded memory for unbounded GC-frequency overhead — the
2×-OASR trigger is too aggressive for a heavy-allocation WasmGC guest
with many globals, and/or `trace_vmctx_roots` is too costly to run at
that rate. Same shape as [[indeterminate-progress-leak]] and task 26
([[worker-thread-for-store-backfires]]): frequent GC trades memory for
latency, net worse for sustained sessions. A real fix needs the per-GC
root scan cheaper or the trigger adaptive — not just "GC more often."

**How to apply:** do not expect #13422 (or any pure "GC more often"
change) to be a drop-in fix. The device was tested then reverted to the
known-good 2.4.257 / wasmtime-44 build. If revisiting: measure GC
*frequency × per-GC root-scan cost* and render-frame latency — not just
sweep duration (sweep duration stayed flat and falsely looked fine in a
scripted soak; real interaction surfaced the ANR).

---

## UPDATE 2026-05-30 — RESOLVED: wasmtime 45.0.0 is adoptable

Re-tested on the released **wasmtime 45.0.0**, which carries #13422 PLUS
two things the May-21 test lacked: **#13450** (the `array.copy` stack-map
correctness fix that #13422 *required* — a GC mid-copy could free arrays
incorrectly; we ran #13422 *without* it on May-21) and an **improved
grow-vs-collect heuristic (#12942)** + DRC tracing-memory reductions
(#13192). Crucially, **task 64 (on-demand rendering) now caps the Compose
guest at ~1 fps idle instead of 60 fps**, so the auto-GC fires far less in
normal use — directly attacking the "GC frequency × root-scan cost"
product that caused the May-21 ANR.

Results (branch `wasmtime-45-test`; host `wasmtime`/`wasmtime-wasi` 44→45,
**zero API breaks**):
- **Desktop** `wandr-leak-repro.wasm` (`wasmtime run -Wgc,... -Ccollector=drc`):
  RSS **flat 39 MB** on 45 vs **568 MB+ and climbing** on 44 — leak bounded
  at max churn.
- **Device idle** wandr-app: flat ~253 MB, ~7% CPU — no regression.
- **Device 60 fps + active scrolling, 3 min** (frame-pacing temporarily
  forced to 0 to recreate the May-21 condition): **RSS flat ~220 MB**, frames
  steadily advancing, **no ANR/crash**, user reports **smooth scroll with
  only minor occasional glitches** — the May-21 render-thread-blocking ANR is
  GONE.

**Verdict: ADOPT wasmtime 45.** The earlier "necessary but not sufficient /
trades memory for latency" conclusion was for #13422 *alone* at 60 fps;
#13450 + heuristics + task 64's lower churn close the gap. Drop the periodic
`Store::gc(None)` mitigation once 45 lands. **Adoption caveat:** the loader
self-heals an app's own cwasm on a wasmtime version bump but NOT its
dependency cwasms (`Component::deserialize_file` fails "incompatible version
'44'" → fallback test-frame / blank screen) — reinstall all apps on upgrade,
or fix `app_loader.rs` to recurse re-precompile into deps. See
[[reference_on_demand_rendering]].
