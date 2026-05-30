---
name: project_signal_client_architecture
description: Signal client on wart is a wasm32-wasip2 GUEST using the generic task-66 wasi:tls transport — NOT a native host daemon (that design was rejected; app logic belongs in the guest)
metadata: 
  node_type: memory
  type: project
  originSessionId: 48a1a9e9-e5d8-4f1f-8fc4-ac9c8bb50eb9
---

Building a simple text-only Signal client as a wart app (task 67). The
load-bearing architecture decision (corrected 2026-05-30):

**The Signal client is a single `wasm32-wasip2` GUEST component. All client logic
lives in the guest.** It's just another wart app, using the generic host
capabilities every app shares: `skiko-gfx` to render, **`wasi:tls`/`wasi:sockets`
(task 66) to reach Signal's servers**, WASI fs to persist. `libsignal` /
`libsignal-service-rs` + dioxus UI in the same guest; networking via a wasip2
transport implemented over `wasi:tls`.

**REJECTED: the native-host-daemon design.** An earlier scaffold put the
connection in a bespoke native daemon (`wart-signal`, presage-based, mirroring
wart-arbiter). Discarded. **Why:** app logic belongs in the app; the host stays
generic. A per-app native host extension doesn't scale and breaks the host/guest
boundary the whole project is built on. **Task 66 wired `wasi:tls` precisely so a
guest can do its own networking** — putting Signal host-side would make that work
pointless. Do not reintroduce a per-app host service without explicit user
direction.

**Background trade-off:** backgrounding freezes the guest (on-demand rendering
gates all guest calls, `standalone.rs:890`), so a guest-held websocket can't keep
alive in the background. v1 is therefore **foreground-only** (matches task scope).
If background delivery is ever needed, the fix is a **generic** host capability for
*all* apps (host-managed keep-alive / background-task primitive), never a
Signal-specific daemon — a separate future task.

**How to apply:** Phase 0 (feasibility gate) is "does the Signal Rust stack
compile to `wasm32-wasip2` and drive its networking over `wasi:tls` instead of
native tokio/hyper?" — NOT an aarch64-android cross-compile. `libsignal` has an
official wasm build (crypto half plausible); `libsignal-service-rs` hides its
transport behind traits (the seam where we inject `wasi:tls`). See
[[reference_wart_wasi_tls_transport]], [[feedback_wasi_threading]], and
`tasks/67-signal-client.md`.

**Phase-0/1 DONE (link+receive+send+persist, desktop+on-device). Phase-2 UI
architecture (decided 2026-05-30):** split into **two composed components** with a
WIT contract — `signal-engine` (exports `wart:signal/chat`: init/poll-events/send/
history/state; owns net+store+history) and `signal-ui` (imports `chat` + skiko-gfx;
dioxus now, Compose later), composed via WAC. UI is toolkit-agnostic; engine stays
the only thing touching the network. **Gating runtime finding:** `wstd::block_on`
recreates+clears its reactor per call, and no guest code runs between component
exports — so the engine needs a PERSISTENT step-executor (built in `init`, stepped
non-blocking per `poll-events`) with the wart-wasi-shims pollable-await bound to it
(not wstd's block_on reactor). Full design + contract in `tasks/67-signal-client.md`.
