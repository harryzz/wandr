# Task 26 — Move wasmtime `Store` to a worker thread

> **Status: ❌ ATTEMPTED → REVERTED 2026-05-18 — net regression.**
> The refactor was completed end-to-end (worker thread,
> mpsc-channel-based event delivery, EventLoopProxy-driven vsync
> redraw, lifecycle handshake). It successfully eliminated the
> input-dispatch ANR. But device testing revealed it introduced a
> *worse* UX problem: tap-to-display latency accumulates from
> instant on cold-start to 5-6 s after a few minutes of
> interaction. The user comparison was conclusive — the original
> main-thread `Store` design is responsive throughout a session;
> the worker-thread design is not. Reverted before commit.
>
> The original goal (avoid ANR) remains unsolved. The honest
> conclusion: there is no host-side ANR avoidance that doesn't
> trade off responsiveness more painfully than the ANR itself.
> Long-term fix is upstream wasmtime (#13403) shipping a tracing
> collector, or one of the architectural pivots documented in
> `post-art-roadmap.md` §12.
>
> Companions:
> - **Upstream issue: [bytecodealliance/wasmtime#13403](https://github.com/bytecodealliance/wasmtime/issues/13403)** — filed 2026-05-18. This task is the local mitigation while it works through their queue.
> - [[wasmtime-drc-no-autoschedule]] (memory) — why we can't fix
>   this at the wasmtime level today; upstream fix is months out.
> - [[drc-first-fit-alone-backfires]] (memory) — why we already
>   ruled out an allocator-side fix as a shortcut.
> - `tools/triage/wasmtime-issues/issue-draft.md` — final body posted as #13403.
> - `wasmtime-issue-artifacts/` (repo root) — diff patches, logcat, reproducer attached to the upstream issue.
> - `tasks/25-diagnose-suspend-leak.md` — original leak diagnosis.

## What this task is and isn't

**Is:** an architectural refactor of `wandr-host/src/lib.rs` that
moves the wasmtime `Store<HostState>`, `bindings::SkikoUi`, and
`SkiaRenderer` (incl. EGL/GL context) off the Android main thread
onto a dedicated worker thread. Main thread becomes a thin event
pump that forwards winit/Android events to the worker via an
`mpsc` channel and returns immediately, so Android's input
dispatcher always sees a fast ack.

**Isn't:** a fix for the underlying wasmtime DRC scaling problem.
Sweeps still take seconds, get more expensive over time, and
still freeze the visible UI while running. They just no longer
trip Android's 5 s ANR threshold because the main thread is free
to acknowledge input even while the worker is busy.

## Why now

The patched-build trajectory data (2026-05-18) confirms a single
`Store::gc` cascade reaches 5+ s of wall time after ~10 min of
active interaction. With the cascade on the main thread, that's an
ANR. With the cascade on a worker thread, the main thread keeps
acknowledging input and the UI freezes-but-survives. Until
wasmtime ships a tracing collector (estimated 6–18 months — see
the wasm-gc RFC), this is the only available mitigation that
preserves our component-model architecture.

## Architecture target

```
┌──────────────────────────────────┐       ┌──────────────────────────────────┐
│  Main (Android NativeActivity)   │       │  Worker (NEW)                    │
│                                  │       │                                  │
│  winit event loop                │       │  Store<HostState>                │
│  Window (ANativeWindow handle)   │  cmd  │  bindings::SkikoUi               │
│  EventLoopProxy<UserEvent>       │ ────▶ │  SkiaRenderer (EGL ctx, GL ctx)  │
│                                  │       │  scheduler, lifecycle, clipboard │
│  on every callback:              │  evt  │                                  │
│    push WorkerCmd onto mpsc      │ ◀──── │  main loop:                      │
│    return immediately            │       │    match recv() {                │
│                                  │       │      Redraw → render_frame ...   │
│                                  │       │      PointerMove → on_pointer_   │
│                                  │       │      Resumed → init renderer ... │
│                                  │       │    }                             │
└──────────────────────────────────┘       └──────────────────────────────────┘
```

Channels:

```rust
// main → worker
enum WorkerCmd {
    Resumed { window: Arc<Window>, component: Arc<Component> },
    Suspended { ack: oneshot::Sender<()> },
    Resize { w: u32, h: u32 },
    Redraw,
    PointerDown { x: f32, y: f32, id: u32 },
    PointerMove { x: f32, y: f32, id: u32 },
    PointerUp { x: f32, y: f32, id: u32 },
    KeyEvent { kind: KeyEventKind, code_point: u32, key_id: u32 },
    Lifecycle(LifecycleState),
    Shutdown,
}

// worker → main
enum UserEvent {
    RequestRedraw,
    // future: profile metrics, error reports, etc.
}
```

Coalescing rule for `PointerMove`: if the worker queue already
contains an unconsumed `PointerMove` for the same `id`, replace
its position with the new one rather than appending. Compose
mostly cares about the latest position; the loss of intermediate
samples is acceptable for the POC. Fling-velocity tracking will
be coarser; if that bites we revisit. (`PointerDown` and
`PointerUp` are NEVER coalesced — those edges matter.)

## Steps

### Step 1 — Worker module + channel types (~1 h)

New file: `wandr-host/src/worker.rs`.

- Define `WorkerCmd`, `UserEvent`, `WorkerHandle`.
- `WorkerHandle::spawn(engine: Engine, proxy: EventLoopProxy<UserEvent>) -> Self`
  spawns the thread, returns a handle holding the `mpsc::Sender`.
- The thread's main loop owns `Store<HostState>` and `bindings`
  internally — `App` no longer touches them directly.
- Coalescing happens in a small wrapper around `mpsc` (look at
  the queue's contents on `send`; if last entry is `PointerMove`
  for the same id, replace; else append).

### Step 2 — Move `HostState` construction into worker (~1 h)

The existing cold-resume path in `lib.rs::resumed` constructs
`HostState`, `Store`, `bindings`, `SkiaRenderer`. That entire
block moves into `WorkerCmd::Resumed` handling on the worker.

Main-thread `resumed` keeps:
- `event_loop.create_window(...)` (must happen on main)
- `binder::init()`, `display_impl::probe()` (one-shot cold init)
- A `worker.send(WorkerCmd::Resumed { window: Arc::clone(&window), ... })`

### Step 3 — Rewrite every `ApplicationHandler` callback to forward (~2 h)

Every callback in `lib.rs` that currently does
`if let (Some(b), Some(s)) = (&self.bindings, self.store.as_mut())`
becomes `self.worker.send(WorkerCmd::...)`. List of touch points
(grep `bindings, self.store.as_mut`):

- `window_event` → many: PointerDown/Move/Up, Resize, KeyEvent,
  RedrawRequested
- `suspended` → Lifecycle(Stopped) + Suspended (with ack)
- The warm-resume branch of `resumed` → its own `WorkerCmd` so
  the worker can do `inherit_caches_from`

### Step 4 — Vsync: worker→main `RequestRedraw` (~30 min)

Currently `window.request_redraw()` is called from `resumed` and
from inside redraw handling (some impls call it to keep the
animation loop alive). After the move, those call sites are on
the worker; they need to send `UserEvent::RequestRedraw` to main
via `EventLoopProxy`, which then calls `window.request_redraw()`.

### Step 5 — Lifecycle handshake (~30 min)

`Suspended` is the only synchronous cross-thread point. Main
needs the guest to finish dispatching `Stopped` BEFORE the window
is dropped (otherwise wasm-side observers see an invalidated
renderer). Implementation: `WorkerCmd::Suspended { ack }` carries
a `oneshot::Sender<()>`; worker dispatches Lifecycle::Stopped,
releases the EGL surface, signals `ack`. Main waits on the
receiver with a bounded timeout (~2 s) before proceeding. If the
timeout fires, log and proceed — better stale-render than
deadlock.

### Step 6 — Device verify (~2 h, includes iteration)

- Build with `--features profile` so we keep the per-frame logger
  for trajectory.
- Cold-start, interact for 5+ min, induce the same load that
  produced the ANR in the unpatched run.
- **Pass criteria:** no ANR even when sweep duration logs >5 s.
  UI freezes during sweep are expected and acceptable.
- **Fail criteria:** any ANR; any panic; warm-resume broken;
  pointer events lost or out-of-order beyond the documented
  coalescing.

### Step 7 — Commit + memory update (~30 min)

Two commits:
1. Worker module + lib.rs refactor + task doc.
2. `feedback_store_worker_thread.md` memory documenting the
   pattern (what threading model, what semantics to preserve,
   what tripped us up).

Update `.task-state` to TASK=26 STEP=verify-done STATUS=complete.

## Out of scope

- Reducing sweep cost itself — that's an upstream wasmtime issue.
- Replacing the deferred-gc trigger — the existing
  `profiling::check_and_run_deferred_gc` between-frames pattern
  still works inside the worker loop.
- Improving warm-resume — already works; just runs on a different
  thread.
- Making rendering async/parallel — single-threaded inside the
  worker is fine for now; the goal is ANR avoidance, not
  throughput.
- Profile feature changes — the existing `wandr-profile: frame N`
  per-frame logger keeps working on the worker thread.

## Known risks

1. **EGL/GL thread affinity.** The Skia GL context is bound to
   the thread that calls `eglMakeCurrent`. Creating the renderer
   on the worker means EGL init happens there. Verified safe in
   principle; needs device confirmation. If the GL context can't
   be created from a non-main thread for some Android-specific
   reason, we'd need plan B: render on main, wasm on worker, with
   a `SkPicture` handoff. Significantly more work.

2. **`Send` bounds.** `Store<T>` is `Send` only if `T: Send`.
   `HostState` contains `SkiaRenderer` which contains
   `skia_safe::gpu::DirectContext` etc — need to verify these are
   `Send`. If not, must restructure. Most skia-safe handles are
   `Send` but the GL context wrapping may not be.

3. **rsbinder + tokio runtime.** `sensors_impl.rs` uses a tokio
   runtime for binder callbacks (per the Cargo.toml comment).
   Need to verify the tokio runtime works on the worker thread
   it's instantiated on, not the main thread.

4. **Coalescing dropping events users care about.** Mitigation:
   keep `PointerDown`/`PointerUp` non-coalescable. If fling
   velocity becomes wrong, expose a config knob and revisit.

## Verification checklist

- [ ] `cargo apk build --features profile` succeeds
- [ ] Cold-start to first frame in <1 s (matches unpatched)
- [ ] Warm-resume after app background still works
- [ ] Pointer interaction (scroll, tap) feels equivalent to
      unpatched in steady state (between sweeps)
- [ ] During a sweep cascade, UI freezes but app does NOT ANR;
      logcat shows `wandr-drc-sweep: dur=N ms` but no
      `WindowManager: ANR`
- [ ] Audio + haptic still fire (tasks 21/18 unaffected)
- [ ] No `signal 11` / panic in adb logcat across a 10-min soak

## Estimates

| Step | Wall time |
|------|-----------|
| 1. Worker module | 1 h |
| 2. HostState move | 1 h |
| 3. Callback forwarding | 2 h |
| 4. Vsync proxy | 30 min |
| 5. Lifecycle handshake | 30 min |
| 6. Device verify + iterate | 2–4 h |
| 7. Commit + memory | 30 min |
| **Total** | **~half day to full day** |

## When to abandon and pick plan B

If step 6 reveals EGL/GL can't be created on a non-main thread on
this Pixel 2 XL build, abandon the "everything-on-worker"
approach. Switch to:

- Wasm Store on worker; SkiaRenderer + EGL on main.
- Wasm-side render call records into a `SkPicture` on the worker.
- Worker sends the picture across to main via a channel.
- Main replays the picture into the GL surface.

This adds an SkPicture copy per frame (~few ms at most) but keeps
GL on main. Cost: another full day of refactor. Document the
finding before committing to the switch.

---

## Findings from the device test (2026-05-18)

The full refactor was implemented per the steps above and worked
functionally on cold start. ANR was indeed avoided across long
soaks. But three failed hypotheses + one positive control later,
the lag emerged as a hard regression:

| Tested | Result |
|---|---|
| Worker thread + 64 MB gc threshold + 10 s cooldown | No ANR. Big lag (5-6 s). |
| Lower gc threshold to 32 MB + 3 s cooldown | No ANR. Same lag. |
| Add time-based gc trigger (5 s) | No ANR. Same lag. |
| Drop gc interval to 1 s | No ANR. Same lag. |
| Rebuild all compose-*-wasi against latest skiko | No ANR. Same lag. |
| **Disable gc entirely** (host returns Ok(true), no Store::gc) | **No ANR. Same lag.** |
| **Revert worker thread (Store back on main)** | **Responsive throughout. ANR risk returns.** |

The conclusive A/B was the last two rows. With gc fully disabled
but worker thread still in place: lag persists. With worker thread
removed but everything else equal: instant taps even after long
use. The worker thread architecture itself is the cause.

### Probable mechanism

`worker_main` uses an outer `recv()` + inner `try_recv()` drain
pattern. In steady-state rendering (60 Hz Redraws) the inner loop
finds another Redraw waiting before it can return to the outer
loop's blocking `recv()`. Verified empirically: instrumentation
counting outer-loop iterations never logged across 5000+ rendered
frames — the worker was continuously inside the inner drain. With
Compose's per-frame work growing over time (the underlying
`SafeContinuation` retention / snapshot subscriber accumulation
diagnosed in task 25 / wasmtime#13403), each Redraw takes a few ms
longer than the previous, so the channel's send rate (~60 cmds/sec
from vsync + ~100 cmds/sec from active touch) eventually exceeds
the worker's process rate, and a backlog accumulates.

Backlog of N queued cmds × per-cmd-time = perceived input latency.
At growth rates we observed, N reaches several hundred within a
minute, producing the 5-6 s tap-to-display lag the user reported.

The original main-thread design is naturally rate-limited by vsync
— `WindowEvent::Touch` directly calls `dispatch_pointer_v2` and
returns to winit, which holds the next `RedrawRequested` until the
next vsync. No queue grows because there's no asynchronous
producer-consumer relationship.

### Why this wasn't predictable from the design doc

The task plan correctly identified ANR avoidance as the goal and
the worker thread as the mechanism. It did NOT anticipate that
**event-channel backpressure under steady-state load** would
produce a *worse* UX failure mode. The mpsc-with-drain-loop
pattern is fine for sporadic event streams but pathological for
continuous-vsync workloads. A bounded channel (or a "single-pending
Redraw" coalescer) might mitigate, but at that point the
architecture is approximating the synchronous main-thread design
with extra hops.

### What's worth keeping from the attempt

- The wasmtime instrumentation patches (saved at
  `/home/harry/wandr/wasmtime-issue-artifacts/`) and the upstream
  issue (#13403) — these are independent of the worker-thread
  approach and remain valuable.
- The skiko `CharProperties.wasi.kt` stub — added because the
  compose-*-wasi rebuild (one of the failed-hypothesis tests)
  needed it. Keep it; without it future compose-foundation
  republishes fail.
- The roadmap entry (`post-art-roadmap.md` §12) — the
  fallback-path analysis is independent of the worker thread.

### Closing thought

The whole task is a worked example of *non-obvious load-induced
regressions* — instrumented in isolation looked clean, instrumented
end-to-end revealed the queue dynamics. Future tasks that move
hot-path work onto a different thread need a "backpressure under
sustained load" verification step before they're considered done.
