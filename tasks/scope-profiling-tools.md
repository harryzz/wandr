# Scope: Profiling tools for the WASM runtime

> Preparatory analysis, written 2026-05-17. What profiling exists for
> wasm + wasmtime in 2026, what each tool actually answers, and what
> the known issues (the ProgressIndicator memory leak from
> `feedback_indeterminate_progress_leak.md`, and the unmeasured
> per-frame CPU budget) tell us about which to wire up first.
>
> The implementation lives in task 23
> (`tasks/23-profiling-hooks.md`). This doc is the why; that one is
> the how.

## Why this matters

Two known performance concerns on the current PoC:

1. **Indeterminate ProgressIndicator memory leak** — the
   `feedback_indeterminate_progress_leak.md` memory documents a
   ~0.4 MB/s growth in wasm linear memory whenever an indeterminate
   ProgressIndicator (or any `while(true){ withFrameNanos {} }`
   loop) is active. Mitigation is "use static widgets"; the actual
   cause is Kotlin/Wasm continuation retention. We have anecdotal
   measurements but no instrumented confirmation.

2. **Per-frame CPU budget is uncharacterized.** CLAUDE.md notes
   "~10–20 ms/frame" overall but doesn't break that down across
   Compose recomposition, Skiko draw, WIT marshalling, wasmtime
   overhead, host-side Skia/EGL. Optimization without a profile is
   guessing.

Wiring up profiling answers (1) quantitatively and (2) by
attribution. Both before any further perf work is worth doing.

## Tool inventory (2026)

### Inside wasmtime (the runtime we use)

| Tool | What it answers | Cost |
|---|---|---|
| `ResourceLimiter` trait (`Store::limiter()`) | "When does linear memory grow, by how much?" Every `memory.grow` traps through `memory_growing(current, desired)`. Log with wall-clock to confirm leak rate is steady vs spiky. | ~30 LOC |
| `Store::data_size()` / `Memory::data_size(&store)` | Same data, polled per-frame rather than event-driven. Easy to plot against `Store::gc_async()`. Won't catch growth-then-free patterns since gc reduces size. | ~5 LOC |
| `GuestProfiler` (`wasmtime::GuestProfiler`) | "Which guest functions burn the most CPU?" Emits Firefox-Profiler-compatible JSON. Stack samples cross host↔guest boundary. Best tool for the per-frame budget question. | ~50 LOC + `Config::epoch_interruption(true)` |
| `Config::profiler(ProfilingStrategy::JitDump)` | "Which JIT-compiled function burns CPU?" — fed to Linux `perf`. Different layer: wasmtime overhead vs guest body vs host call overhead. Less useful on Android (no `perf` ready-to-hand); on desktop dev it's gold. | 1-line config |
| `Store::call_hook` | "How many host calls (WIT imports) per frame?" Catches accidental N+1 patterns. `feedback_currentnanotime_pollutes.md` hints we already have one such gotcha; would surface more. | ~10 LOC |
| `Engine::increment_epoch()` + epoch interruption | Sampling without modifying guest code. Drives GuestProfiler. | Trivial once GuestProfiler is wired. |

### Outside wasmtime

| Tool | What it answers | Notes |
|---|---|---|
| **Firefox Profiler** (`https://profiler.firefox.com`) | Visualization of GuestProfiler output. Drag-and-drop. Best wasm profile viewer that exists. | No setup; ingests the JSON we emit. |
| **`twiggy`** | "Why is wandr-app.wasm 11 MB?" Static .wasm size breakdown. | Not useful for the runtime leak; cheap to run when codegen size matters. |
| **`dhat-rs`** | Rust heap profiler for the host side. Catches retain cycles in rsbinder, EGL surface leaks, ndk handles. | NOT useful for guest linear-memory leak; that's wasm-side. |
| **`adb shell dumpsys meminfo com.example.wasmruntime`** | Total process memory split by category. Tells us whether growth is in `Other dev mmap` (wasm linear memory typically), GPU VRAM (Skia GL), or native heap (host code). | Already available; should be the first measurement. |
| **Kotlin/Wasm intrinsics** (kotlinx-coroutines-debug) | Inspect continuation retention itself. | Doesn't currently work on wasmWasi target. Forking kotlinx-coroutines = weeks of upstream work; not happening. |

## What it tells us about the ProgressIndicator leak

Per `feedback_indeterminate_progress_leak.md`, the root cause is
Kotlin/Wasm continuation retention in indefinite `withFrameNanos`
loops. The wasmtime tools above:

- ✅ confirm the leak rate **quantitatively** (ResourceLimiter
  timestamps)
- ✅ confirm growth is steady-state vs spiky (event-by-event log)
- ✅ show which guest function the wasm engine is in during growth
  (GuestProfiler — likely
  `kotlin.coroutines.intrinsics.IntrinsicsKt__IntrinsicsJvmKt` or a
  Compose recomposer frame)
- ✅ show whether host calls are also implicated (call_hook counts)
- ❌ **won't** show which Kotlin-source-level retain cycle is at
  fault — that's inside Kotlin/Wasm-generated continuation classes,
  which look opaque to wasmtime
- ❌ **won't** fix the leak — that's an upstream Kotlin/Wasm
  codegen change

So profiling **characterizes** the leak (confirming it's a real
linear-memory growth, not some other metric); it doesn't **fix** it.
Mitigation remains "use static widgets."

## What it tells us about CPU usage

Much more directly tractable than the leak. GuestProfiler captures
the actual per-frame cost breakdown across:

- Compose recomposer + diff
- Skiko draw + WIT marshalling
- wasmtime epoch/instance/function-call overhead
- Host-side (skia-safe + EGL) work via call_hook + JitDump

Likely high-value findings ahead of any optimization work:

- Where the per-frame budget actually goes (we suspect Compose
  recomposition is the dominant cost; profiler will confirm or refute)
- Whether the snapshot-comparison hot path from
  `feedback_transition_animate_to_bug.md` is still on the path
- Whether our WIT marshalling has accidental copies (lots of
  `list<f32>` / `list<u8>` calls per frame would show up)
- Whether the `freeAllComponentModelReallocAllocatedMemory` call
  pattern (one call per WIT import) is a hot path itself

## Plan summary (what task 23 does)

Wire up a small `wandr-host/src/profiling.rs` behind a `profile`
cargo feature flag. Estimated 2–4 hours total:

1. **ResourceLimiter** logging `memory.grow` events with wall-clock
   timestamps
2. **Per-frame `Memory::data_size()`** snapshot, logged every N
   frames
3. **`Store::call_hook`** counting WIT host calls per frame
4. **GuestProfiler** sampling a 10-second window after first render,
   dumping JSON to `/sdcard/Download/wandr-profile.json` for Firefox
   Profiler ingestion
5. **`Config::profiler(JitDump)`** on desktop dev only (Android
   doesn't have `perf` ready)

All gated by a `profile` cargo feature so production APK builds
remain unaffected.

Verification: deploy the profile-enabled APK, watch
ProgressIndicator scenario, retrieve the JSON, drop into Firefox
Profiler. Plus a meminfo snapshot before/after a 60-second leak run.

## Open questions

- Should ResourceLimiter log to logcat (cheap but log-spammy under
  heavy growth) or to a file in `/sdcard/Download/` (cleaner but
  needs WRITE_EXTERNAL_STORAGE which we already declare)? Probably
  the latter.
- GuestProfiler's sample rate — wasmtime allows configuring via
  epoch_deadline_async_yield_and_update. 1 ms is typical; might be
  too aggressive for the slower frames on Pixel 2 XL.
- Is there a way to capture a profile that spans an Android
  Activity lifecycle (warm-resume across suspend) cleanly? Probably
  needs a "start/stop" signal we trigger from the guest via a
  debug-only WIT call.
