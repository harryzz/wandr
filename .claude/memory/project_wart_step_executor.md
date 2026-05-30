---
name: project-wart-step-executor
description: "Persistent frame-stepped wasi:io/poll executor that lets a wasm guest's async tasks survive across component-export calls (task 67 Phase 2 engine)"
metadata: 
  node_type: memory
  type: project
  originSessionId: 81538868-ab9d-48a4-8de3-a56739b11c3e
---

`wstd::runtime::block_on` builds a reactor, runs a future to completion, then
clears it — so spawned tasks die between calls and **no guest code runs between
component-export invocations**. For an export-driven engine (the Signal
`wart:signal/chat` engine: each `poll-events` is a separate call that must let a
background receive loop / websocket keepalive make progress), that's fatal. wstd's
stepping methods (`block_on_pollables`, `nonblock_check_pollables`,
`pop_ready_list`) are `pub(crate)`, so you can't drive its reactor yourself.

**Fix (DONE 2026-05-30, desktop-verified):** new crate `wart-step-executor` at
`external/libsignal-service-rs/wart-wasi-shims/wart-step-executor`. A **persistent**
thread-local reactor installed at `init()` (never torn down) advanced by a
**non-blocking `step()`** — the `wasi:io/poll` 0-duration-timer trick (append a
`subscribe_duration(0)` pollable so `poll()` returns immediately), copied from
wstd's `nonblock_check_pollables`. Bookkeeping (Registration/AsyncPollable/WaitFor/
Reactor, async-task + slab) mirrors wstd 0.6.6's `runtime/reactor.rs`; only the
lifecycle + non-blocking step differ (~280 lines). API: `init()` / `spawn()` /
`step()` / `sleep()` / `AsyncPollable`.

The libsignal fork's three wstd touchpoints were rebound onto it (off wstd):
`wart-wasi-shims/reqwest/src/tls.rs` (`AsyncPollable`), `src/push_service/mod.rs`
(background ws-process `spawn().detach()`), `src/websocket/mod.rs` (keepalive
`sleep`); `wstd` dep replaced in both shim Cargo.tomls + the fork's wasm32 deps.

Engine pattern (`repros/signal-engine`): `init` spawns+**detaches** one root task
(spawn returns `async_task::Task` which **cancels on drop** — must `.detach()`);
`poll-events` = `step()` then drain a shared `Rc<RefCell<VecDeque<event>>>`. Driven
by `repros/signal-engine-smoke` (Rust CLI importing chat) `wac plug`'d onto the
engine, run under `repros/wasi-tls-runner`: produced a real Signal `link-url` QR
across repeated `poll-events`. See [[project_signal_client_architecture]],
[[reference_wart_wasi_tls_transport]], [[feedback_wasi_threading]].
