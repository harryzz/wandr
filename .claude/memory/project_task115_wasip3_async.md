---
name: project-task115-wasip3-async
description: "✅ Task 115 COMPLETE (M0–M4): Signal transport on native CM-async (wasip3) end-to-end on device — zero step-executor, chat + audio/video calls verified; the driving rules, dep-proxy/pump gotchas, and remaining p2 consumers"
metadata: 
  node_type: memory
  type: project
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

**✅ Task 115 (2026-07-08): the Signal transport runs on native Component-Model
async (WASI 0.3) end-to-end on the Pixel 2 XL** — zero `wandr-step-executor` in
the Signal build (p3 is the DEFAULT flavor; `P2=1 build.sh` = rollback), live
both-ways chat, audio + video calls verified, whole device stack (chrome,
Compose, Swift, Avalonia) on the `p3-async` host. Ledger: `tasks/STATUS.md`
#115; full narrative + recipes: `tasks/115-signal-transport-wasip3-async.md`;
mechanics spike (ALL GATES PASS): `repros/cma-cross-call-spike/README.md`.

**The five load-bearing rules (device-verified):**
1. A sync-lifted export CANNOT block on an async-lifted callee
   (`CannotBlockSyncTask`) → the HOST starts the engine via a re-exported
   `engine-start.start: async func` (`wac compose` script — plug can't add
   exports; UI byte-identical across flavors; host probes the export like
   bg-tick).
2. `call_async` does NOT advance unrelated pending futures → the host must
   PUMP (`store.run_concurrent(sleep)`) wherever it idles: standalone nap-pump
   (`requires_async_drive()` gate) + a per-frame pump in the winit desktop loop.
3. Under `p3-async` the store becomes async-required → p2 links ASYNC
   everywhere (sync wasi impls nest-panic via `in_tokio`), and cross-app dep
   proxies (`wire_dep_into_linker`) must instantiate deps async + forward via
   `func_new_async`/`call_async` — sync proxies took out the IME + Demo (the
   two dep-consuming apps) at first render.
4. Guest toolchain floor = **wit-bindgen 0.59** (`spawn_local`; ONE unified
   copy per component). 0.53's guest runtime never surfaced partial stream
   data on open connections (all keep-alive protocols incl. the Signal WSS
   stalled) — see [[reference_wasmtime46_p3_stream_bugs]]. Released wasmtime
   **=46.0.1** suffices.
5. Config-hash flip (`wasm_component_model_async`) needed NO mass reinstall —
   the loader auto-re-precompiles on cache-key mismatch; components too big
   for on-device AOT (swiftui.demo 65 MB) get cross-AOT'd on the PC
   (`WANDR_AOT_TARGET=aarch64-linux-android` with the p3 x86 host → hash
   matches → "cache fresh").

**Why:** this is the template for retiring frame-stepped reactors everywhere —
background guest async now natively spans export calls on the shipped runtime.

**How to apply / follow-ups:** fg pump CPU tuning (Signal fg idles ~11.5% from
~21 `run_concurrent` sweeps/s vs 1–3% fg baseline; BACKGROUND = 0–1% parity) —
must re-verify CALLS when touching pump cadence (the in-call 10 ms tick is
pump-driven). **`wandr.audio.player` migrated to p3 2026-07-09** (device-verified: cover-art/
metadata fetch runs live over the native-async transport, zero step-executor).
It's a SINGLE Slint component — the crux was wit-bindgen UNIFICATION: bumped
slint-wandr 0.57.1->0.59 (so the UI + transport share ONE runtime) and gated
wandr-reqwest's `wasip2` dep to the `p2` feature. bg-tick is async-lifted via a
separate `wit-p3/` copy declaring `bg-tick: async func` (wasmtime rejects an
async LIFT of a sync-typed func; the `async:` generate! filter alone fails device
precompile). Remaining before deleting the crate: `repros/signal-link` (STALE —
delete) + dropping all `p2-legacy` rollback flavors, then `wandr-step-executor` +
`wandr-reqwest`'s `p2` feature/tls.rs can go. M5 (streaming
`subscribe()`) unplanned. Rollback assets: `wandr-host.p2.bak` on device,
`P2=1` flavor, `.signal-state-backups/`.
