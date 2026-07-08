# CM-async cross-call spike (task 115 / M2a) — ALL GATES PASS

Proves the mechanics M2 needs beyond the M1 transport spike: a **background guest
task that survives and advances BETWEEN component-export calls**, in the exact
production topology (engine wac-composed into a sync UI, host driving frames).

## Result (2026-07-08)

```
start() returned — engine task spawned
frame  1  t=   202ms  ticks=  1  (+1)
frame  2  t=   403ms  ticks=  3  (+2)     <- +2 ticks per 200ms pumped nap @100ms cadence
...
gate A (task survives + advances between calls): PASS
gate B (quiescent when host does not pump):      PASS   <- +0 during thread::sleep naps
gate C (sync instantiate+call on same engine):   PASS   <- pure-p2 app unaffected
```

## Topology

- **engine/** (wit-bindgen 0.53 = the real Signal engine's resolution):
  exports `demo:cma/chat` = `start: async func()` + `poll: func() -> u32`.
  `start` does `async_support::spawn(loop { wait_for(100ms).await; ticks+=1 })`
  — the pattern that replaces `wandr_step_executor::spawn(run()).detach()`.
- **ui/** (wit-bindgen 0.57 = dioxus-canvas's resolution): sync `run-frame`
  export that calls `chat.poll()` (sync→sync across the composed boundary).
- **compose.wac**: engine plugged into ui's chat import AND chat re-exported on
  the composite so the host can call `start`.
- **host/**: clone of wandr-host `make_config()` + `async_support(true)` +
  `wasm_component_model_async(true)`; dual-serve linkers (`p2::add_to_linker_sync`
  + `wasmtime_wasi::p3::add_to_linker` + `wasmtime_wasi_tls::p3::add_to_linker`);
  one current-thread tokio runtime owns every store op; nap =
  `store.run_concurrent(async |_| tokio::time::sleep(nap).await)`.

## Findings (all load-bearing for M2)

1. **A sync-lifted export may NOT block on an async-lifted callee** — wasmtime
   trap `CannotBlockSyncTask` (`may_block = task.async_function ||
   returned_or_cancelled()`, concurrent.rs). So the UI can never call an async
   `init`. **Production shape: the HOST starts the engine** via a re-exported
   `chat.start: async func()` (probe the composite for it, like the bg-tick/ime
   export probes). UI stays byte-identical across p2/p3 flavors; every UI↔engine
   edge stays sync→sync.
2. `wac plug` can't add exports; use a **`wac compose` script** (compose.wac)
   to both plug the engine AND re-export its chat. wac 0.10.0 handles the
   async-lifted types fine.
3. `wasm-tools validate` needs **`-f cm-async`** (not in the default feature set
   of wasm-tools 1.245) — the production `apps/user/wandr.signal/build.sh`
   validate line needs this too once the engine goes p3.
4. **`async_support(true)` does NOT force async entrypoints per store** —
   asyncness is per-instance from matched host imports (sync p2 host fns keep
   `Asyncness::No`). Gate C proves existing sync apps run unchanged on the same
   Engine. Only the p3-importing composite requires `instantiate_async`/`call_async`.
5. Timer granularity: a `wait-for` whose deadline expires during an UNPUMPED nap
   does not fire on the next short `call_async` — it fires at the next pump.
   Same-or-better than today's `step()` semantics (nothing runs between polls).
6. `call_async` on ANY export advances all pending store futures while in
   flight; `run_concurrent` is the between-frames pump. One current-thread tokio
   runtime must own instantiate + every call + every pump (host tokio IO objects
   bind to the driver active at creation).
7. wasmtime 46 has its **own `Error` type** (not anyhow) — `?` converts
   wasmtime→anyhow fine, but don't let inference pick `wasmtime::Error` as a
   closure/block error type that anyhow errors then fail to convert into.

## Phase-3 gates (added 2026-07-08) — the real wandr-reqwest p3 backend, live

```
gate D1 fetch (Client→http1→tls_p3): PASS — 200 OK (559 body bytes)
gate D2 select-drop torture: PASS — HTTP/1.1 200 OK (559 body bytes, 26 dropped reads)
```

- **D1**: the engine component does a live HTTPS GET through the FULL
  production stack — `wandr_reqwest::Client` → `http1` → `tls_p3` (the p3
  backend behind the crate's `p3-async` feature) — zero step-executor.
- **D2**: cancellation-safety torture. Raw `TlsStream`, the pending read
  dropped on every 1 ms tick (the engine's `select_biased!` shape); 26 reads
  dropped mid-flight, response still byte-perfect (Content-Length verified).
  This is safe **by construction**: tls_p3's dedicated reader task owns the
  `StreamReader`; `read_*` futures only wait on the shared pushback buffer.
- Extra finding: wasmtime 46 deprecated `Config::async_support` (no-op) —
  async availability is always-on; per-instance asyncness governs.

## Build + run

```bash
./build.sh
./host/target/release/cma-cross-call-spike-host composite.wasm \
    p2sync/target/wasm32-wasip2/release/cma_p2sync.wasm [host-to-fetch]
```
