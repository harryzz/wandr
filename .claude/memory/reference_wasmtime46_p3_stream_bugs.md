---
name: reference-wasmtime46-p3-stream-bugs
description: "Task 115: p3 keep-alive/WSS stalls + wait-for wedge were wit-bindgen 0.53 GUEST-side bugs (NOT wasmtime 46) — fix = wit-bindgen 0.59 guests (spawn→spawn_local); released wasmtime 46.0.1 sufficient; full matrix in repros/cma-cross-call-spike"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

**The task-115 M2 stalls were wit-bindgen 0.53's guest-side async runtime, not
wasmtime.** Symptoms with 0.53 guests (any wasmtime): every keep-alive protocol
stalls at the first read on a still-open connection — the Signal websocket
included (WSS upgrade sent, response bytes strace-verified INTO the host
socket, never surfacing in the guest) — and a bare `task::sleep` through a
SECOND `generate!` instance hard-wedges the event loop. `Connection: close`
flows all pass (EOF flushes the chain), which masks the bug in simple fetch
tests.

**Why the record matters:** this was first (wrongly) blamed on wasmtime 46
because the fix-test changed two variables at once (host → git main AND guests
→ 0.59). The clean matrix (repros/cma-cross-call-spike, gates ka-probe/E/F):
46.0.x + wb0.53 = stall · 46.0.1 + **wb0.59** = **ALL PASS** (incl. live WSS
101 + first provisioning frame from chat.signal.org, inline AND across pumps).

**How to apply:** any wasip2/p3 CM-async guest doing streaming I/O must build
with **wit-bindgen 0.59+** (`features = ["async","async-spawn"]`; `spawn` was
renamed `spawn_local`), and every crate compiled into ONE component must
resolve to the SAME wit-bindgen (0.53 vs 0.59 are semver-incompatible → two
rt instances → broken executor). Released wasmtime **46.0.1** is sufficient
(wandr-host exact-pinned to it). Related: `[[reference_wandr_wasi_tls_transport]]`;
driving rules (host must PUMP via `run_concurrent` wherever it idles;
sync-lifted exports can't block on async callees → host-started `engine-start`)
in `repros/cma-cross-call-spike/README.md`.
