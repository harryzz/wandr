---
name: reference-wasmtime-version-status
description: "wandr's wasmtime version tracking. PINNED =46.0.1 (host + wasi + wasi-tls; harryzz/wasmtime fork for KT-86415). 47.x major = worth-it-not-urgent (GC/exceptions default, smaller cwasm; cost = fork rebase + full cwasm rebuild). 47.0.3/46.0.2 = 2 Low security GHSAs, wandr NOT affected. Bumping ANY version invalidates AOT cwasm."
metadata:
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-08-01T14:14:47.731Z
---

Tracking home for wandr's wasmtime version decisions.

## Current pin
`runtime/wandr-host/Cargo.toml`: **`wasmtime = "=46.0.1"`** (+ `wasmtime-wasi`,
`wasmtime-wasi-tls` same pin), features `component-model` + `all-arch`; the
`p3-async` feature adds `component-model-async` + `wasmtime-wasi/p3` +
`wasmtime-wasi-tls/p3`. Uses the **`github.com/harryzz/wasmtime` fork** for the
KT-86415 WASI-adapter State-pin ([[wasi-adapter-state-corruption]]). **AOT `.cwasm`
on Android arm64 / JIT desktop** → *any* wasmtime bump changes the precompile hash
and invalidates every guest's cwasm (full rebuild + device reverify).

## 47.x major evaluation (checked 2026-08-01, latest 47.0.3)
Worth it eventually, **not urgent**. Wins: **Wasm GC + exception-handling default-ON**
(our explicit `wasm_gc/function_references/wasm_exceptions(true)` at `lib.rs:691-693`
become redundant — Kotlin/Compose needs exactly these); **"GC roots on fiber stacks
now traced"** fix (relevant — we run WasmGC under component-model-async fibers);
**smaller `.cwasm`** (traps/addrmap sections + symbol removal); component-model-async
+ WASIp3 socket hardening (IPV6_V6ONLY UDP, p2/p3 consistency). Breaking: **wasi-threads
+ `wasi-common` REMOVED** — we use NEITHER (verified). **Cost of the bump:** rebase the
`harryzz/wasmtime` KT-86415 patch onto 47 (main risk) + full cwasm rebuild + Android/
macOS reverify. p3 still behind the unstable `component-model-async` flag on 47 too.

## 47.0.3 / 46.0.2 security patch (2026-07-31) — wandr NOT affected
Coordinated security-only release (no features), backported to our line as **46.0.2**.
Two advisories, both **Low**:
- **GHSA-hgjw-h833-99q9** (CVSS 3.8): "Stores mix up type indices between engines."
  Needs **2+ Engines + cross-mixing objects** via `Instance::new`/`InstancePre::
  instantiate`/etc.; **NOT guest-triggerable**. wandr keeps each Engine's
  components/stores together (zygote = one Engine forked; the audio.player bg engine
  isn't cross-instantiated) → **not affected**.
- **GHSA-2hw9-mc66-jc2q** (CVSS 2.0): "Preemption/traps during bulk ops break VM
  state." Needs **`Store::epoch_deadline_callback` + store mutation / resume-after-
  timeout**. Host does **not** use `epoch_deadline_callback` (0 matches); the
  `epoch_interruption` mentions in `lib.rs`/`profiling.rs` are "would be needed"
  comments, i.e. NOT enabled → **not affected**.
**Action: none.** Both Low + non-applicable. **46.0.2** is the low-churn option if ever
staying current (patch on our major → no fork rebase, security-only), but even a patch
likely changes the cwasm hash (guest rebuild), so not worth it for these two.

## Threading (wasm/wasi) — will NOT stay single-threaded forever, but is TODAY
Checked 2026-08-01. Three layers, mid-migration:
- **Base `threads` proposal** (shared linear memory + atomics) = approved, but only the
  *substrate* (shared mem + `atomic.wait/notify`); **no thread *spawning***.
- **`wasi-threads`** (old spawn mechanism) = officially **LEGACY** ("retained for
  engines that can only support WASI v0.1"); **removed from wasmtime 47**. Never fit
  the component model.
- **`shared-everything-threads`** = the SUCCESSOR (component-model-native: thread
  spawn + TLS + **component-model lifecycle builtins**). Stated plan: **"WASI v0.2 and
  following will use shared-everything-threads once fully implemented."** But **draft**,
  unshipped (wasmtime #9466 open, active 2026). Likely multi-year.
- **Current reality:** the component/P2+ world is **effectively single-threaded NOW**
  (old path gone, new path not ready — a transition gap).
- **async ≠ threads:** p3 = single-thread async *concurrency* (shipping); threads =
  *parallelism* (shared-everything-threads, future). Orthogonal tracks.
- **wandr:** guests are single-threaded BY DESIGN (reactor / frame-stepped / one p3
  loop / zygote-fork-per-app). Parallelism already comes from **separate processes**
  (zygote fork per app) + **host native threads** (Skia/GStreamer/MediaCodec/SRTP).
  In-guest wasm threading isn't needed, and if it landed it would NOT change the
  `bsky-sdk` `Send` issue ([[reference_bluesky_atproto_wandr]]) — guests stay
  single-threaded so the client stays `!Send`; fix is `?Send` regardless
  ([[feedback_wasi_threading]]).

## Re-check trigger
A wasmtime bump becomes worth it when: p3/component-model-async stabilizes, OR a
security advisory that DOES apply to us lands (guest-triggerable / single-engine /
non-epoch), OR the 47 GC-default + smaller-cwasm wins are wanted for a release. Then
sequence: rebase fork onto target → confirm KT-86415 patch applies → bump the 3 pins
→ rebuild host + all cwasm → device smoke (Signal p3 call, jellyfin playback).

Related: [[reference_wasmtime46_p3_stream_bugs]], [[feedback_wasmtime_compile]] (gc/
function-refs/exceptions flags), [[reference_kotlin_wasm_component_model_status]] (KT-86415),
[[reference_host_build_scripts]].
