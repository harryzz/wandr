# Artifacts for the wasmtime DRC sweep-cost issue

Companion artifacts for the upstream issue at
`bytecodealliance/wasmtime` (link added once the issue is filed).

## Contents

| File | What it is |
|---|---|
| `01-instrumentation.patch` | The diagnostic patch against wasmtime 44.0.1 — adds two accessor methods on `FreeList` and one `log::info!` call at the end of `DrcHeap::sweep` reporting `(N, F, sweep_dur, freed_bytes)`. ~40 lines. No semantic changes. Off-by-default. |
| `02-negative-result-first-fit-size-indexed.patch` | The size-indexed `BTreeSet` optimization for `first_fit` we tested. Made our workload measurably worse. Documented so reviewers know why allocator-side speedups alone don't help here, and so the wasmtime team can evaluate it for *other* workloads if useful. Applies on top of patch 01. |
| `wandr-leak-repro.wasm` | A ~200 KB Kotlin/Wasm module that exercises the leaked allocation pattern: a bare `suspendCoroutine` loop with no Compose, no kotlinx-coroutines, no UI. The minimum-possible reproducer for the underlying SafeContinuation accumulation. |
| `Main.kt` | The ~60-line Kotlin source that compiles to `wandr-leak-repro.wasm`. Self-contained; reviewers can read it inline. |
| `logcat-full-2026-05-18.log` | Full Android logcat from a soak that produced three sweep events. Captured for raw provenance. |
| `logcat-wandr-only-2026-05-18.log` | Subset filtered to `wandr-drc-sweep`, `wandr-profile`, `InputDispatcher` mentions of our app, and `ANR` events. ~4400 lines; the parts useful for triage. |
| `sweep-trajectory-2026-05-18.log` | Just the three `wandr-drc-sweep` events (one line each) — the trajectory data table in the upstream issue is derived from this. |

## Trajectory data (matches the issue body)

```
11:33:56  sweep 1: dur=478 ms,   N=1,223,135, F:1→37,269
11:41:52  sweep 2: dur=1248 ms,  N=2,333,452, F:9,966→67,394
12:18:20  sweep 3: dur=3000 ms,  N=4,585,421, F:29,538→67,394
```

| | Sweep 1 | Sweep 2 (+8 min) | Sweep 3 (+37 min) |
|---|---|---|---|
| N | 1.22M | 2.33M | 4.59M |
| Sweep duration | **478 ms** | **1248 ms** | **3000 ms** |
| Per-entry sweep cost | 0.39 μs | 0.53 μs | **0.65 μs** |
| Bytes reclaimed | 56 MB | 116 MB | 249 MB |

Both N and per-entry sweep cost grow over time. Per-entry cost
climb is the memory-bandwidth-bound linked-list walk losing
cache locality as the working set grows.

## Reproducing locally

`wandr-leak-repro.wasm` is now **self-driving** — `main()` runs the
`suspendCoroutine` suspend/resume cycle itself in an unbounded loop, so
a plain `wasmtime run` reproduces the accumulation directly. No
embedder, no host imports, no component model.

```bash
wasmtime run -Wgc,function-references,exceptions -Ccollector=drc \
    wandr-leak-repro.wasm
```

RSS climbs ~100 MB/s as garbage accumulates with no sweep, plateaus at
the 4 GB wasm32 ceiling within ~50 s, then enters the steady-state
sweep cycle (the `tick #` counter keeps advancing past the plateau).
Apply `01-instrumentation.patch` to see the per-sweep `(N, F, dur)`
trajectory. Let it run a few minutes, then Ctrl-C.

> Earlier revisions of this artifact were a WASI **component** driven
> by an external `render-frame` export — a plain `wasmtime run` invoked
> `main()` once, which parked at the first `suspendCoroutine` and
> returned, so nothing accumulated. That is fixed in this revision.

Source: `Main.kt` in this directory (~70 lines, zero dependencies).
Project: <https://codeberg.org/harryzz/wart-leak-repro.git>.

## How the trajectory was captured

Apply `01-instrumentation.patch` to a local copy of wasmtime
44.0.1 (e.g. via `[patch.crates-io]`). Build, deploy. The
`wandr-drc-sweep:` log line fires once per sweep. Plot duration
vs N over time.

The Android-specific cascade-to-ANR symptom requires running
the full Compose UI under the same instrumentation; the wandr-app
(not included here) plus an active 5–10 minute interaction
session reproduces the >5 s sweep cascade.
