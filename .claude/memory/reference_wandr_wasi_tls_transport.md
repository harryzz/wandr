---
name: reference_wandr_wasi_tls_transport
description: wasi-sockets + wasi-tls give wandr guests host-delegated network/TLS on wasmtime 45; Signal needs its pinned CA injected via a custom TlsProvider
metadata:
  type: reference
---

Transport de-risk for a Signal-on-wandr app (probe: `repros/wasi-tls-probe/`,
run 2026-05-30 on wasmtime 45.0.0). A `wasm32-wasip2` guest can reach a remote
TLS server with **no crypto/TLS compiled into the guest** — using only
host-delegated `wasi:sockets` (DNS+TCP) + `wasi:tls@0.2.0-draft` (Phase 1).
Proven: example.com did full DNS→TCP→handshake→HTTP `200 OK` host-side.

Key facts (verified in `external/wasmtime/crates/wasi-tls` + `wasmtime-wasi-45`):
- **Host must grant network.** `wasmtime-wasi`'s default `SocketAddrCheck` is
  deny-all (`sockets/mod.rs:177`); `AllowedNetworkUses` defaults `tcp:true,
  udp:true, ip_name_lookup:FALSE`. A TCP connect checks both the coarse bool AND
  the per-address callback. Unlock = `WasiCtxBuilder::inherit_network()` (or
  `.socket_addr_check(..)`) **plus** `.allow_ip_name_lookup(true)` for DNS. The
  wandr host currently sets neither (only stdio + RO preopens) → guests have zero
  egress today. CLI equivalent: `-S inherit-network -S allow-ip-name-lookup -S tls`.
- **wasi-tls ships in wasmtime 45.** Crate `wasmtime-wasi-tls` (45.0.0, lockstep);
  host does the TLS (rustls provider), guest gets pollable `tls_input/tls_output`
  over a wasi-sockets stream. API: `ClientHandshake::new(server-name, in, out)` →
  `finish()` → poll future → `(client-connection, in, out)`.
- **Signal-specific blocker = cert pinning, not transport.** `chat.signal.org`
  → `invalid peer certificate: UnknownIssuer`: it's pinned to Signal's private CA,
  and the default `RustlsProvider` trusts only `webpki_roots::TLS_SERVER_ROOTS`
  (`providers/rustls.rs:34`). `wasi:tls` gives the guest **no** way to pass a CA.
  Fix is host-side + small: `TlsProvider` is a public trait (`lib.rs:174`) — wandr
  supplies a provider whose RootCertStore = public roots + Signal's CA (the PEMs
  presage/libsignal-service-rs already bundle). **PROVEN 2026-05-30** via
  `repros/wasi-tls-runner/` — a host runner with a custom `TlsProvider`
  (webpki roots + Signal's self-signed `O=Signal Messenger, LLC` CA) ran the same
  probe component and `chat.signal.org` handshook + returned `HTTP/1.1 404`
  (trusted; 404 = wrong path, irrelevant to transport). **WIRED INTO wandr-host
  (task 66) + on-device-PROVEN 2026-05-30:** the probe packaged as a wandrpkg
  (`wandr.probe.wasitls`) + `--zygote-launch`ed through the production host reached
  chat.signal.org (HTTP/1.1 404) over host-delegated wasi:sockets+wasi:tls, no
  guest crypto. Host wiring lives in `runtime/wandr-host/src/signal_tls.rs`
  (`grant_network` opens outbound to ALL addresses — per-app allowlist is the
  follow-up). Gotcha found: wandr's LogcatStderr sink only surfaces the FIRST
  `write()` of a multi-write line, so guest `eprintln!("{}",x)` truncates —
  build the whole line + emit one write.

Bindgen gotcha: the multi-package wasi `wit/deps` tree needs **wit-bindgen 0.53**
(0.46's macro errors `failed to resolve directory while parsing WIT`).

Bigger Signal picture in [[reference-signal-on-wandr-feasibility]]: transport is
now host-side primitives + a CA injection; residual guest work is the
async/tokio→wasi-poll refactor of presage's I/O, websocket/HTTP framing
(pure-Rust, portable), and verifying libsignal's protocol-crypto crates compile
to wasm. wandr guest drive-model (persistent connection vs render-loop) also open.
