# wasi-tls reachability probe

De-risks the **transport layer** for a possible Signal-messenger wart app: can a
`wasm32-wasip2` guest reach a TLS server using only **host-delegated**
`wasi:sockets` + `wasi:tls@0.2.0-draft` — i.e. with **no TLS/crypto compiled into
the guest**?

The guest (`src/main.rs`) does, per target host: DNS resolve → TCP connect →
`wasi:tls` ClientHandshake → HTTP/1.1 `GET /` → read response. It imports only
`wasi:sockets/*` + `wasi:tls/types` (verify: `wasm-tools component wit`).

## Build + run (desktop, wasmtime 45)

```bash
cargo build --target wasm32-wasip2 --release           # wasip2 emits a component directly
wasmtime run -S inherit-network -S allow-ip-name-lookup -S tls \
    target/wasm32-wasip2/release/wasi-tls-probe.wasm
```

(`wit-bindgen = "0.53"` — the 0.46 macro failed to parse the multi-package
`wit/` dep tree; the 0.53.1 CLI/lib parse it fine.)

## Result (2026-05-30, wasmtime 45.0.0)

```
[OK]   example.com      — resolved 104.20.x.x · 838 bytes · status: HTTP/1.1 200 OK
[FAIL] chat.signal.org  — tls handshake io-error: invalid peer certificate: UnknownIssuer
```

**Transport is proven.** The control host completes the full DNS→TCP→TLS→HTTP
round-trip entirely host-side. Signal reaches certificate verification and fails
only on the trust anchor: `chat.signal.org` is **certificate-pinned** to Signal's
own private CA, and wasmtime's default `RustlsProvider` trusts only the Mozilla
public bundle (`webpki_roots::TLS_SERVER_ROOTS`, `wasi-tls/src/providers/rustls.rs:34`).

## What this means / next step

Not a wart or wasi-tls limitation — it's Signal's pinning. `wasi:tls`'s
`ClientHandshake::new(server-name, in, out)` has **no** way for the guest to pass
a custom CA; the trust store lives entirely in the host provider. Fix is
**host-side and small**: `wasmtime_wasi_tls::TlsProvider` is a public trait
(`wasi-tls/src/lib.rs:174`) — wart-host supplies a provider whose `RootCertStore`
= public roots **+ Signal's pinned CA** (the same PEMs presage/libsignal-service-rs
bundle). TLS stays host-delegated; no crypto in the guest.

**To fully close the loop:** a ~40-line host runner using
`wasmtime` + `wasmtime-wasi` + `wasmtime-wasi-tls` with a custom provider that
adds Signal's CA, then re-run this same component and expect Signal → `200`/`4xx`
(any HTTP status line = handshake trusted).
