---
name: project-wandr-step-executor
description: "Persistent frame-stepped wasi:io/poll executor that lets a wasm guest's async tasks survive across component-export calls (task 67 Phase 2 engine)"
metadata: 
  node_type: memory
  type: project
  originSessionId: 81538868-ab9d-48a4-8de3-a56739b11c3e
---

**RETIREMENT STATUS (task 115 M3, 2026-07-08):** the Signal build no longer
contains this crate AT ALL (native CM-async / p3 is its default flavor;
`cargo tree` = 0 hits). Remaining consumers: `wandr.audio.player` (real — its
own migration later) and `repros/signal-link` (STALE, superseded by the
engine, scheduled for deletion). Delete this crate only after audio.player
migrates. The p2 backend now lives behind wandr-reqwest's explicit `p2`
feature (default-on). See [[reference_wasmtime46_p3_stream_bugs]].

`wstd::runtime::block_on` builds a reactor, runs a future to completion, then
clears it — so spawned tasks die between calls and **no guest code runs between
component-export invocations**. For an export-driven engine (the Signal
`wandr:signal/chat` engine: each `poll-events` is a separate call that must let a
background receive loop / websocket keepalive make progress), that's fatal. wstd's
stepping methods (`block_on_pollables`, `nonblock_check_pollables`,
`pop_ready_list`) are `pub(crate)`, so you can't drive its reactor yourself.

**Fix (DONE 2026-05-30, desktop-verified):** new crate `wandr-step-executor`, now
at `crates/wandr-step-executor` (relocated 2026-06-15 out of the libsignal fork —
see below). A **persistent**
thread-local reactor installed at `init()` (never torn down) advanced by a
**non-blocking `step()`** — the `wasi:io/poll` 0-duration-timer trick (append a
`subscribe_duration(0)` pollable so `poll()` returns immediately), copied from
wstd's `nonblock_check_pollables`. Bookkeeping (Registration/AsyncPollable/WaitFor/
Reactor, async-task + slab) mirrors wstd 0.6.6's `runtime/reactor.rs`; only the
lifecycle + non-blocking step differ (~280 lines). API: `init()` / `spawn()` /
`step()` / `sleep()` / `AsyncPollable`.

The libsignal fork's three wstd touchpoints were rebound onto it (off wstd):
the reqwest shim's `tls.rs` (`AsyncPollable`), `src/push_service/mod.rs`
(background ws-process `spawn().detach()`), `src/websocket/mod.rs` (keepalive
`sleep`); `wstd` dep replaced in both shim Cargo.tomls + the fork's wasm32 deps.

**RELOCATED to crates/ (2026-06-15, commits parent `765fa156` + submodule
`5164118f6`).** The three transport shims were never Signal-specific, so they
moved out of the fork into the wandr tree as first-class shared libs:
`wandr-wasi-shims/{reqwest,reqwest-websocket,wandr-step-executor}` →
`crates/{wandr-reqwest,wandr-reqwest-websocket,wandr-step-executor}`, dropping the
`-shim` suffix (packages `wandr-reqwest` / `wandr-reqwest-websocket`). All-or-nothing
(chain `reqwest-websocket → reqwest → step-executor`). Consumers repointed: audio
player, Signal engine, repros/signal-link, and the fork itself (`../../crates/`,
since it only ever builds inside a wandr checkout). `wandr.audio.player` uses
`wandr-reqwest` + `wandr-step-executor` for internet cover-art lookups — the
original non-Signal consumer that motivated the move.

Engine pattern (`apps/user/wandr.signal/engine`, promoted out of repros 2026-05-31): `init` spawns+**detaches** one root task
(spawn returns `async_task::Task` which **cancels on drop** — must `.detach()`);
`poll-events` = `step()` then drain a shared `Rc<RefCell<VecDeque<event>>>`. Driven
by `repros/signal-engine-smoke` (Rust CLI importing chat) `wac plug`'d onto the
engine, run under `repros/wasi-tls-runner`: produced a real Signal `link-url` QR
across repeated `poll-events`. See [[project_signal_client_architecture]],
[[reference_wandr_wasi_tls_transport]], [[feedback_wasi_threading]].
