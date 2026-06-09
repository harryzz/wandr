# Task 66 — Wire the Signal-CA wasi-tls TlsProvider into wandr-host

Gives guests **host-delegated TLS with Signal's pinned CA trusted**, so a future
Signal client (or any networked guest) can reach the network over
`wasi:sockets` + `wasi:tls@0.2.0-draft` with **no TLS/crypto compiled into the
guest**. Proven end-to-end first in `repros/wasi-tls-{probe,runner}`; this task
moves the runner's custom `TlsProvider` into the production host.

## Why it works on wandr-host's sync store

wandr-host instantiates synchronously (`add_to_linker_sync`, sync `instantiate` +
`call_render_frame`). `wasi-tls`'s host functions are **sync** — `finish()`
spawns the async TLS connect via `wasmtime_wasi::runtime::spawn` onto the ambient
tokio runtime, and the sync `wasi:io/poll` linker blocks on readiness. Same model
sync `wasi:sockets` already uses, so **no `async_support` / async store needed**.

## Changes

- `runtime/wandr-host/Cargo.toml` — add `wasmtime-wasi-tls = "45"` +
  `rustls`/`tokio-rustls`/`webpki-roots`/`rustls-pemfile` + common `tokio` (rt).
  Versions mirror `wasmtime/crates/wasi-tls`.
- `runtime/wandr-host/certs/signal-messenger-ca.pem` — Signal's self-signed
  service CA (`O=Signal Messenger, LLC`), embedded via `include_bytes!`.
- `runtime/wandr-host/src/signal_tls.rs` (new) — single source of truth:
  - `SignalTlsProvider` (`TlsProvider`): rustls `ClientConfig` with root store =
    `webpki_roots::TLS_SERVER_ROOTS` + Signal's CA; `SignalTlsStream` newtype
    bridges the rustls stream to wasi-tls's `TlsStream` (orphan rule).
  - `grant_network(builder)` — `inherit_network()` + `allow_ip_name_lookup(true)`.
  - `wasi_tls_ctx()` — builds the per-store `WasiTlsCtx`.
  - `add_to_linker(linker)` — registers `wasi:tls` (`opts.tls(true)`).
- `runtime/wandr-host/src/lib.rs` — `mod signal_tls;`; `HostState.wasi_tls` field;
  `impl WasiTlsView for HostState`; grant + ctx at the lib.rs store build.
- `standalone.rs` + `run_once.rs` — grant + `wasi_tls` field at their HostState
  builds (the 3 construction sites).
- `app_loader.rs` — `signal_tls::add_to_linker` after `add_to_linker_sync` in
  both `instantiate` and `instantiate_command`.

## Capability / security note

`grant_network` opens **outbound TCP/TLS to all addresses for every guest**. It is
latent — only guests importing `wasi:sockets`/`wasi:tls` can use it, and current
skiko-UI guests don't — but it is a real posture change for a "no system
modification" runtime. **Follow-up:** gate per-app via `package.toml` (e.g.
`allow_network` / host allowlist) using `WasiCtxBuilder::socket_addr_check`
instead of blanket `inherit_network`.

## Status

✅ **Done — device-verified 2026-05-30.**
- Host-target `cargo check`: `signal_tls.rs` + wasi-tls/rustls API resolve clean
  (only the pre-existing android-only `sf_surface` desktop-build gap remains).
- aarch64-android cross-build: **succeeds** — `ring v0.17.14` compiles under the
  NDK, wasi-tls integrates, 56 MB binary (the cross-compile was the main risk).
- On-device: the launcher child logged
  `signal_tls: trust store = 119 public roots + 1 Signal CA` →
  `SignalTlsProvider::new()` ran on-device (ring installed, embedded Signal CA
  parsed, rustls config built); component instantiated (wasi-tls linker OK);
  rendered frames 0/1/2; wandr launcher renders; **no trap, no regression**.

**On-device network proof — done 2026-05-30.** Packaged `repros/wasi-tls-probe`
as a `wasi:cli/command` wandrpkg (`wandr.probe.wasitls`, `package.toml` beside the
probe), installed it (`wandr-host --install` → precompiled to cwasm), and launched
it headless via the zygote (`--zygote-launch`). Through the production host the
forked child logged the Signal-aware trust store and reached the network over
host-delegated wasi:sockets + wasi:tls — **no crypto in the guest**:

```
signal_tls: trust store = 119 public roots + 1 Signal CA
[wasi-tls-probe] [OK]   example.com     - ... | HTTP/1.1 200 OK
[wasi-tls-probe] [OK]   chat.signal.org - ... | HTTP/1.1 404 Not Found
TRANSPORT PROVEN ...   (call_run returned Ok, exit=0)
```

`chat.signal.org` handshakes only because the host trusts Signal's CA — the full
chain works in production. (Also fixed a real wandr bug found here: the
LogcatStderr sink only surfaces the first `write()` of a multi-write line.)

Remaining for an actual Signal client is guest-side: async/tokio→wasi-poll
refactor of presage's I/O, websocket/HTTP framing, and libsignal crypto wasm
compat — see `[[reference-signal-on-wandr-feasibility]]`.
