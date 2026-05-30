---
name: worker-thread-for-store-backfires
description: "Moving wasmtime Store to a worker thread (task 26) successfully eliminated ANR but introduced WORSE input-lag accumulation. Conclusively reverted 2026-05-18. Don't try this design again."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ade59596-71ca-44d3-bc3e-26f4f4ba5671
---

Don't move the wasmtime `Store` to a worker thread with an mpsc
event channel + EventLoopProxy redraw round-trip. We tried this end
to end in task 26 (2026-05-18); it works on cold start, eliminates
ANR, but degrades input responsiveness severely over a few minutes
of use — tap-to-display latency grows from instant to 5-6 s.

**Why:** the worker's `recv` + inner-`try_recv` drain pattern, in
steady-state vsync rendering, never returns to the blocking
`recv`. The outer-loop iter counter stays at 1 across 5000+
rendered frames (verified via instrumentation). Effectively the
worker is in a tight inner loop processing the channel
continuously. As Compose's per-frame work grows over time
(SafeContinuation accumulation per wasmtime#13403, plus retained
snapshot/recomposer state), the channel's producer rate exceeds
consumer rate; a backlog accumulates linearly with time; new
events wait behind the backlog. After minutes, queue depth
reaches hundreds → effective input latency = queue × per-cmd-time.

The OLD main-thread design has no such queue. `WindowEvent::Touch`
calls `dispatch_pointer_v2` synchronously and returns to winit;
winit holds the next `RedrawRequested` until the next vsync. No
asynchronous producer-consumer relationship means no queue means
no backlog means no latency accumulation.

**How to apply:** if a future task proposes moving render or
hot-path work to a worker thread for ANR mitigation:

1. Recognize that the gain is asymmetric — it trades ANR (a rare,
   recoverable, Android-handled symptom) for input latency
   accumulation (a frequent, irrecoverable-during-session, user-
   perceived symptom). ANR is the better failure mode.
2. If you must move work off-thread anyway, the channel must be
   *bounded* AND the per-event work must reliably stay under the
   producer's send interval; otherwise backpressure compounds.
3. Coalescing helps for moves but NOT for redraws — vsync produces
   redraws faster than complex Compose recomposition can consume
   them when retained-state is high.

For the wart-app POC specifically: stick with the main-thread
Store, accept the ANR risk, wait for upstream wasmtime
(#13403) to ship a tracing collector, or take one of the
architecture pivots in `post-art-roadmap.md` §12.

**Negative-result diff and per-step findings live in
`tasks/26-store-worker-thread.md`.** Don't redo the attempt
without re-reading that.
