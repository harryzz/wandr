# Task 23 — Profiling hooks (`ResourceLimiter` + `GuestProfiler`)

> **Status: 🔲 scoped, not started.** Companion to
> `tasks/scope-profiling-tools.md` which justifies the tool choices
> and rules out paths that don't pay off (kotlinx-coroutines-debug,
> dhat for guest memory, etc).

## Why this task is essential

We've been doing perf work blind:

- The ProgressIndicator memory leak (`feedback_indeterminate_progress_leak.md`)
  is "characterized" by anecdotal "~0.4 MB/s" measurements with no
  instrumentation — we can't tell whether mitigations help without a
  proper instrument.
- The per-frame budget noted in CLAUDE.md ("~10–20 ms/frame") is a
  single number with no attribution. Optimization without knowing where
  the cost goes is guessing.

Both are unblockable without instruments. This task wires the
instruments.

## Scope

Add a `wart-host/src/profiling.rs` module gated by a `profile` cargo
feature. Production APK builds (no `--features profile`) stay
unaffected. With the feature enabled, four hooks turn on:

### A — `ResourceLimiter` for `memory.grow` events

Custom struct implementing `wasmtime::ResourceLimiter`. Logs each
`memory_growing(current, desired)` call with a wall-clock timestamp.
Wired in via `Store::limiter()`.

Output: per-event log line — `[ms since first grow]  current_pages →
desired_pages (Δ N KB)`. After a 60-second ProgressIndicator scenario
the log gives a quantitative leak rate.

### B — Per-frame `Memory::data_size()` snapshot

In `lib.rs::RedrawRequested` after the existing `render_frame` call,
log `Memory::data_size(&store)` every N frames (N tuneable, default
60 → once per second). Cheap, low log volume, gives the gc-aware
size curve.

### C — `Store::call_hook` host-call counter

Install a hook that increments a counter per host-call entry. Reset
per frame; log the count alongside the frame time. Catches accidental
N+1 patterns à la `feedback_currentnanotime_pollutes.md`.

### D — `GuestProfiler` sampling window

Driven by `Config::epoch_interruption(true)` + a thread that ticks
`Engine::increment_epoch()` every ~1 ms. Profile starts on the first
`render_frame` call; runs for 10 seconds (configurable); dumps JSON
to `/sdcard/Download/wart-profile-<unix-ms>.json` for Firefox
Profiler ingestion.

Optional bells:
- `Config::profiler(ProfilingStrategy::JitDump)` enabled on the
  `not(target_os = "android")` build only — desktop dev gets perf
  integration; Android skips since `perf` isn't readily available.

## Implementation sketch

```
wart-host/
  Cargo.toml                 (+ [features]   profile = [])
  src/
    profiling.rs             (new — ResourceLimiter, call-hook
                              counter, guest profiler driver)
    lib.rs                   (cfg-gated wires through profiling::* )
```

Skeleton of `profiling.rs`:

```rust
#![cfg(feature = "profile")]

use std::sync::atomic::{AtomicU64, Ordering};
use wasmtime::{ResourceLimiter, CallHook, Store};

pub struct GrowthLogger {
    started_at: std::time::Instant,
    last_size:  std::cell::Cell<usize>,
}

impl ResourceLimiter for GrowthLogger {
    fn memory_growing(&mut self, current: usize, desired: usize, _maximum: Option<usize>)
        -> wasmtime::Result<bool>
    {
        let delta_kb = (desired - current) / 1024;
        log::info!(
            "wasm.memory.grow: t+{:>7}ms  {} -> {} pages  (Δ {} KB)",
            self.started_at.elapsed().as_millis(),
            current / 65536, desired / 65536, delta_kb,
        );
        Ok(true)
    }
    fn table_growing(&mut self, _current: u32, _desired: u32, _maximum: Option<u32>)
        -> wasmtime::Result<bool>
    { Ok(true) }
}

pub static HOST_CALLS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);

pub fn install_call_hook<T>(store: &mut Store<T>) { ... }
pub fn frame_tick<T>(store: &Store<T>, frame_no: u64) { ... }
pub fn start_guest_profile(...) -> GuestProfileHandle { ... }
pub fn finish_guest_profile(handle: GuestProfileHandle, path: PathBuf) { ... }
```

`lib.rs` integration is `#[cfg(feature = "profile")]` blocks
around the existing `resumed()` / `RedrawRequested` paths — no
behavior change when the feature is off.

## Build verify

- `cargo apk build --release` (no feature) → APK identical in
  behavior + size to today.
- `cargo apk build --release --features profile` → APK with the
  hooks. Should also build clean.

## Device verify

1. Deploy the `--features profile` APK to Pixel 2 XL.
2. Run with the demo's ProgressIndicator route active for 60 seconds.
3. `adb pull /sdcard/Download/wart-profile-<unix-ms>.json`,
   drag-and-drop into `https://profiler.firefox.com`.
4. Snapshot:
   - Memory growth log: roughly 60 s × 0.4 MB/s ≈ 24 MB growth
     across ~hundreds of `memory.grow` events
   - GuestProfiler: stack samples in the
     `kotlin.coroutines.intrinsics.*` / `withFrameNanos` /
     Compose recomposer area dominate
   - Host-call counter: steady-state count per frame
5. Also capture `adb shell dumpsys meminfo com.example.wasmruntime`
   at t=0 and t=60s for a process-wide memory snapshot.

## Out of scope (deferred)

- **Fixing the ProgressIndicator leak.** The root cause is in
  Kotlin/Wasm's generated continuation classes; can't be fixed from
  wart-host. This task only characterizes the leak; mitigation
  remains "use static widgets" until upstream Kotlin/Wasm changes.
- **Continuous always-on profiling.** The `profile` feature is for
  debugging sessions; production APKs build without it.
- **Profile-guided optimization of wasmtime AOT.** wasmtime supports
  this in principle; would need to be a separate task informed by
  the data this one produces.

## Estimate

2–4 hours total:

- ~30 min — `Cargo.toml` feature flag + module skeleton
- ~30 min — ResourceLimiter wiring
- ~30 min — per-frame data_size + call_hook counter
- ~1 h    — GuestProfiler sampling thread + JSON dump
- ~1 h    — device verify + sanity-check the Firefox Profiler
            output looks sensible (correct stack frames, sample
            rate matches what we configured, no missing host frames)
