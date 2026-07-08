---
name: reference-wasmtime46-p3-stream-bugs
description: "Task 115 M2 blocker: released wasmtime 46.0.x p3 streams never complete partial reads on open connections (keep-alive/WSS stall) + second-bindgen-instance wait-for wedges; both fixed on main (48-dev) with wit-bindgen 0.59 — full repro matrix in repros/cma-cross-call-spike"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

**Task 115 M2 is code-complete but run-gated on the next wasmtime release.**
Released wasmtime **46.0.x** (latest as of 2026-07-08) has two CM-async/p3 bugs,
both isolated with minimal repros in `repros/cma-cross-call-spike`:

1. **A pending guest stream read never completes on partially-available data
   while the stream stays open** — only on buffer-full or EOF. Every
   `Connection: close` flow works (EOF flushes the chain), every keep-alive
   protocol stalls — including the Signal websocket (provisioning WSS hangs
   after the 101 request; `ka-probe` gate = HTTP/1.1 keep-alive to example.com
   hangs at read-headers). strace-verified: bytes reach the host TCP socket,
   never delivered to the guest.
2. **`wait-for` through a SECOND wit-bindgen `generate!` instance hard-wedges
   the event loop** (bare `task::sleep` via wandr-reqwest's bindings froze the
   process; the same WIT fn via the engine crate's own bindings worked).

Both fixed on wasmtime **main** (48-dev, rev `30ea2ab5b41`, spike host pinned
to it). Guest side must pair with **wit-bindgen 0.59** (`spawn` renamed
`spawn_local`; 0.53 stalls sporadically between adjacent pure-code lines on
48-dev). On 48-dev + 0.59 the FULL gate suite passes, incl. live WSS 101 +
first provisioning frame from chat.signal.org, inline AND spawned-across-pumps.

**Why:** these gate the entire p3-async Signal transport (task 115). Engine
tree already bumped to wit-bindgen 0.59.

**How to apply:** when wasmtime >46 releases, bump wandr-host's `=46.0.0` pins
(+ spike host git pin → release), rebuild `--features p3-async`, run the
task-115 phase-5 desktop gates. Related: `[[reference_wandr_wasi_tls_transport]]`,
the M2 driving rules in `repros/cma-cross-call-spike/README.md` (host must PUMP
via `run_concurrent` wherever it idles — `call_async` does not advance
unrelated pending futures; sync-lifted exports cannot block on async callees →
host-started engine via re-exported `engine-start`).
