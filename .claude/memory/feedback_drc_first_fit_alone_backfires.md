---
name: drc-first-fit-alone-backfires
description: "Don't fix wasmtime's FreeList::first_fit O(F) scan alone — tested it, made the wandr-app workload measurably worse. Sweep got slower and the leak rate went UP."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ade59596-71ca-44d3-bc3e-26f4f4ba5671
---

Replacing wasmtime's `FreeList::first_fit` O(F) linear scan with an
O(log F) size-indexed `BTreeSet` lookup, on its own, makes the
wandr-app workload **worse**, not better.

Measured on device 2026-05-18 (Pixel 2 XL, three sweeps over ~6
minutes of active interaction):

|              | Unpatched | With first_fit fix | Δ      |
|--------------|-----------|--------------------|--------|
| Sweep 1 dur  | 472 ms    | 823 ms             | +74%   |
| Sweep 2 dur  | 1222 ms   | 2010 ms            | +64%   |
| Sweep 3 dur  | (ANR'd)   | 4314 ms            | new    |
| Frame mean   | ~16 ms    | ~16 ms             | none   |
| Alloc rate   | 15K refs/s | 19K refs/s        | +27%   |

**Why this backfires:**

1. Frame-mean cost is dominated by Compose's render/layout/draw, not
   by `first_fit` microseconds. The alloc-side speedup doesn't move
   any frame-level metric.
2. Cheaper allocs let the Kotlin/Wasm guest churn `suspendCoroutine`
   cycles faster → N grows faster between sweeps → each sweep has
   more to walk + dec_ref. The per-dealloc overhead from maintaining
   a second data structure (one extra BTreeSet op per
   `add_block`/`remove_block`/`update_block_len`) then compounds over
   N+ deallocations per sweep.

**How to apply:** the upstream load-bearing fix is wasmtime DRC
auto-scheduling — see [[wasmtime-drc-no-autoschedule]]. Allocator-side
optimizations are constant-factor improvements ON TOP, but shipping
them without the scheduling fix accelerates the pathology. Don't
reach for `first_fit` as our fix.

The negative-result patch + trajectory data is recorded in
`/home/harry/wandr/wasmtime-issue-draft.md` "What we tried that did NOT
work" — useful as a benchmark fixture if anyone wants to evaluate
future allocator changes against this workload.
