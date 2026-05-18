# Issue draft for bytecodealliance/wasmtime

**Status:** POSTED 2026-05-18 as
[bytecodealliance/wasmtime#13403](https://github.com/bytecodealliance/wasmtime/issues/13403).
Kept on disk as the canonical local copy.

**Title:** DRC: per-GC sweep cost grows unbounded over time on steady-state Kotlin/Wasm workload (Android ANR after ~10–48 min)

---

## Summary

On a long-running Kotlin/Wasm + WasmGC guest (~9 MB/s steady allocation
rate driven by `suspendCoroutine`), per-`Store::gc` cost grows
super-linearly over time. After enough sustained use, a single sweep
exceeds 5 s of host-thread time, triggering Android's input-dispatch
ANR. We patched wasmtime to log the over-approximation list size `N`,
free-block count `F`, and sweep duration at every gc; **the trajectory
is reproducible and the scaling is clear**: sweep duration roughly
doubles every few minutes of typical UI interaction, driven primarily
by `N`'s linear growth.

The root cause is **architectural**: nothing in wasmtime auto-triggers
gc, so `N` (over-approximated-stack-roots list) grows monotonically
between sweeps, while sweep itself is O(`N`) with poor cache locality.
Embedders that allocate at a steady rate and let `N` grow eventually
hit unworkable pause times.

This is **distinct from #9701 / PR #10401**. That fix was about
transitive ref-decrementing (correctness). We're running on top of it,
on 44.0.1. The remaining issue is pure scaling cost in the *sweep*.

## Environment

- **wasmtime 44.0.1**, features = `["component-model"]`, sync host calls
- Target: `aarch64-linux-android`, AOT (`--wasm gc --wasm function-references --wasm exceptions`)
- Host: single-threaded `Store` on the render thread (winit android-native-activity)
- Guest: Kotlin/Wasm 2.4.0-RC compiled to a wasm-component
- Device: Pixel 2 XL / Android 15 / API 35

## Reproducer

Minimal: https://codeberg.org/harryzz/wart-leak-repro

A bare `suspendCoroutine` loop driven by per-frame host calls — no
Compose, no UI. Reproduces the underlying allocation pattern at ~9 MB/s
WasmGC-heap growth, OOM in ~6:37 on a Pixel 2 XL with no manual
`Store::gc` triggers.

The full Compose-UI guest (which we actually run) drives a much larger
*retained live set* per gc (Composition tree, Snapshot records,
LaunchedEffect jobs, Recomposer invalidation list). The
super-linear-growth symptom is visible there but not in the minimal
repro, because the linear-scan cost is dominated by fragmentation,
which Compose's diverse object sizes produce far better than a tight
loop.

## Root cause — sweep is O(`N`) and `N` grows unbounded between gcs

`crates/wasmtime/src/runtime/vm/gc/enabled/drc.rs`, lines 124-128:

```rust
/// The head of the over-approximated-stack-roots list.
///
/// Note that this is exposed directly to compiled Wasm code through the
/// vmctx, so must not move.
over_approximated_stack_roots: Box<Option<VMGcRef>>,
```

The over-approximation set is a singly-linked list threaded through
the `next_over_approximated_stack_root` field in each object's
`VMDrcHeader` (line 645). `sweep` (lines 524-618) walks the entire
list every gc:

- Unmarked entries → spliced out, dec_ref'd via
  `dec_ref_and_maybe_dealloc`, possibly deallocated through
  `FreeList::dealloc` (O(log F)).
- Marked entries → **stay in the list** across gcs (lines 578-588).

So `|list|` at sweep time = (refs currently on the precise wasm stack)
+ (refs newly borrowed since last sweep). For a steady-state
high-allocation guest with no explicit `Store::gc` calls, this
monotonically grows. Each entry's `VMDrcHeader` is ~40 B, scattered
across the GC heap → walking is memory-bandwidth-bound (cache miss
per entry on a 95 MB working set at `N` = 2.4M).

**Cost per gc:** O(`N`) linked-list walk, plus N × O(log F) for the
deallocation phase, plus PR #10401's transitive dec-ref. Both `N` and
`F` grow over time → super-linear total cost.

There is no automatic scheduling — wasmtime's `grow_or_collect_gc_heap`
fallback path (the only auto-trigger) only fires on memory.grow
failure. An embedder that returns `Ok(true)` from
`ResourceLimiter::memory_growing` never sees a gc unless it calls
`Store::gc` itself. Embedders that *do* return false on threshold
also get the cascade-on-recomposition behavior described below.

## Secondary — `FreeList::first_fit` is O(F), but not the dominant cost

`crates/wasmtime/src/runtime/vm/gc/enabled/free_list.rs`, lines 180-195:

```rust
fn first_fit(&mut self, alloc_size: u32) -> Option<(u32, u32)> {
    let (&block_index, &block_len) = self
        .free_block_index_to_len
        .iter()                                  // ← walks BTreeMap in key order
        .find(|(_idx, len)| **len >= alloc_size)?;
    ...
}
```

The free-block map is keyed by **block index** (address), not by size.
`iter().find(...)` walks entries in address order returning the
lowest-address block that fits. **O(F) per allocation.**

We initially thought this was the dominant cost. **It isn't** — see the
"What we tried" section. In our workload, frame-level allocation cost
is dwarfed by Compose's render/layout/draw work; switching to a size-
indexed O(log F) lookup didn't move the steady-state frame mean. The
sweep itself is the load-bearing cost, and replacing `first_fit`
without also addressing the sweep makes things *worse* (see below).

## Direct measurements from a patched wasmtime build

I patched 44.0.1 to log `N` (over-approximation list size), `F`
(free-block count), sweep duration, and freed bytes at every sweep.
Three consecutive sweeps during a 45-min soak with active
scrolling/tapping in a Material3 LazyColumn:

```
11:33:56  sweep 1: dur=478 ms,   N=1,223,135, F:1→37,269
11:41:52  sweep 2: dur=1248 ms,  N=2,333,452, F:9,966→67,394
12:18:20  sweep 3: dur=3000 ms,  N=4,585,421, F:29,538→67,394
```

| | Sweep 1 | Sweep 2 (+8 min) | Sweep 3 (+37 min) |
|---|---|---|---|
| `N` | 1.22M | 2.33M | **4.59M** |
| `F_after` | 37,269 | 67,394 | 67,394 |
| Sweep duration | **478 ms** | **1,248 ms** | **3,000 ms** |
| Per-entry sweep cost | 0.39 μs | 0.53 μs | **0.65 μs** |
| Bytes reclaimed | 56 MB | 116 MB | 249 MB |

All three `kept` counts were 0 — sweeps fired from a host-side
`Store::gc(None)` invoked between frames, so the precise-stack-roots
set was empty at sweep time.

Notable observations:

1. **`F` jumps to 37K on the very first sweep.** Sweep 1 deallocates
   1.2M scattered objects; the BTreeMap merge-with-neighbors logic in
   `FreeList::dealloc` only coalesces *immediately address-adjacent*
   blocks. With live retained objects sandwiching dead ones, ~37K of
   the freed blocks survive as discrete entries. From that point on,
   every guest allocation pays an O(37K) `first_fit` walk.

2. **`F` partially reverses between sweeps** (37K → 10K before sweep
   2; 67K → 30K before sweep 3) — the allocator does merge some
   blocks as it satisfies new allocations. But each new sweep then
   re-bloats `F`. Net direction is monotonically up over the soak.

3. **Per-entry sweep cost is rising monotonically** (0.39 → 0.53 →
   0.65 μs/entry, +67% over 45 min). At N=4.6M the over-approximation
   linked list's 40-byte headers occupy ~180 MB of GC heap scattered
   across distant addresses. Walking them is bandwidth-bound; the
   working set has clearly blown L1/L2 and is likely thrashing L3 too.

4. **Sweep duration grows faster than N** — N grows ~2× per
   inter-sweep interval, but duration grew ~3× from sweep 2 to 3 even
   though that gap was 4× longer than sweep 1 to 2. The two effects
   (more entries to walk, each entry costs more) compound.

Extrapolating the trend: at the observed growth rate, sweep durations
push past 5 s within an hour or two. A single sweep at that scale, or
a small cascade of them inside one `render_frame`, exceeds Android's
5 s input-dispatch ANR threshold. We've observed this in practice in
multiple soaks (one ANR at ~48 min on the unpatched build, another at
~10 min on a heavier-interaction soak with the instrumentation
adding small per-sweep overhead). Typical ANR cascade pattern in
logcat:

```
last frame log
+0.0 s   wart-profile: frame N t+X ms  win[N=60 ... ]
+0.0 s   InputDispatcher: "spent 2005 ms processing MotionEvent"
+1.0 s   InputDispatcher: "spent 2350 ms processing MotionEvent"
+4.0 s   InputDispatcher: "spent 2247 ms processing MotionEvent"
+5.0 s   ANR — "Waited 5000 ms for MotionEvent"
```

Mean steady-state frame time stays flat at ~16 ms outside of
sweep/cascade windows — the problem isn't the normal allocation path,
it's the sweep + the post-sweep allocation cost driven by elevated F.

## What we tried that did NOT work

We patched `FreeList` to add a parallel size-indexed
`BTreeSet<(len, index)>`, replacing the O(F) linear scan in `first_fit`
with an O(log F) `range((alloc_size, 0)..).next()` lookup. All 12
existing `FreeList` unit tests pass. On device, this **made things
worse**:

| | Unpatched | With FreeList fix | Δ |
|---|---|---|---|
| Sweep 1 duration | 472 ms | 823 ms | **+74%** |
| Sweep 2 duration | 1,222 ms | 2,010 ms | **+64%** |
| Sweep 3 duration | (ANR'd before) | **4,314 ms** | new |
| Per-frame mean (steady) | ~16 ms | ~16 ms | no change |
| Guest alloc rate | 15K refs/sec | **19K refs/sec** | **+27%** |

Two reasons it failed:

1. **`first_fit` was not the bottleneck.** Frame-mean cost is dominated
   by Compose's render/layout/draw work; the alloc-path microseconds
   we shaved off didn't manifest in any observable metric.

2. **Faster allocs made the leak worse.** With cheaper allocs, the
   Kotlin/Wasm guest churned through `suspendCoroutine` cycles faster,
   so `N` accumulated faster between sweeps. Each subsequent sweep had
   *more* to walk and *more* to deallocate. The per-dealloc overhead
   from maintaining a second data structure (one extra BTreeSet op per
   `add_block` / `remove_block` / `update_block_len`) then compounded
   over `N`+ deallocations per sweep.

The lesson: any allocator-side optimization must be paired with a
sweep-side fix, otherwise it accelerates the pathology. Posting this
finding in case it's useful — happy to share the patch diff if you want
to evaluate it for other workloads where alloc cost *is* the
bottleneck.

## What would help

In order of expected impact:

**1. Automatic sweep scheduling.** The root cause of the unbounded
growth is that nothing triggers a gc until memory.grow fails. A
"schedule a sweep after N additions to the over-approximation list"
heuristic, or a "background-sweep budget" embedders can opt into,
would prevent `N` from ever reaching the millions. This is the
load-bearing fix — everything else is a constant-factor improvement on
top.

**2. Replace the over-approximation linked-list with a `Vec<VMGcRef>`
or similar contiguous structure.** The current intrusive linked-list
through object headers is the worst possible layout for a hot walk:
~40 B headers at scattered addresses, cache miss per entry. A flat
vector (or `SmallVec`-style chunked vector) would give the same
algorithmic complexity with sequential memory access — empirically
10–100× faster walk at the same `N`. The "marked stays" semantic still
works: on sweep, compact in place (write index = read index when
marked). The change does touch the JIT-compiled path that inserts into
the list (currently via vmctx-exposed pointer), so it's not trivial,
but it's the single biggest constant-factor speedup available.

**3. Incremental / time-bounded sweep API.** Something like
`Store::gc_for_at_most(Duration)` that does as much sweep as fits in a
budget and leaves the rest for the next call. Lets embedders amortize
across many host calls rather than absorbing one 5+ s pause.

**4. `FreeList::first_fit` O(log F) lookup.** Worth doing on its own
merits for workloads where alloc cost matters, but per our negative
result, do not ship it without (1) — otherwise embedders with our
workload regress.

## Artifacts attached

Available as a tarball with this issue (also at
`codeberg.org/harryzz/wart-leak-repro` for the reproducer + at the
issue's linked gist for the patches and logs):

- **`01-instrumentation.patch`** — the diagnostic patch that produced
  the trajectory above. ~40 lines against wasmtime 44.0.1. Two
  accessor methods on `FreeList` (`wart_instr_free_block_count`,
  `wart_instr_total_free_bytes`) and one `log::info!` at the end of
  `DrcHeap::sweep`. No semantic changes; 12 existing `FreeList` unit
  tests pass. Happy to PR if useful as a feature-gated diagnostic.

- **`02-negative-result-first-fit-size-indexed.patch`** — the size-
  indexed `BTreeSet` for `first_fit` we tested. Made our workload
  worse for the reasons described in "What we tried that did NOT
  work." Included as a benchmark fixture, not as a proposed fix.

- **`wart-leak-repro.wasm`** + **`Main.kt`** — ~200 KB Kotlin/Wasm
  module exhibiting the bare `suspendCoroutine` allocation pattern.
  ~60-line source. No Compose, no kotlinx-coroutines, no UI.

- **`logcat-full-2026-05-18.log`** + filtered subsets — Android
  logcat from a soak that produced the three sweep events above.

A separate AOT `.cwasm` for the full Compose wart-app (which
exhibits the cascade-to-ANR symptom) is also available but ~30 MB
and not particularly informative without the matching host-side
embedder; happy to share on request.

---

*(Filed because PR #10401 fixed correctness; these symptoms reproduce
on top of that fix. Two specific O(F) / O(N) operations in `drc.rs`
and `free_list.rs` account for the growth.)*
