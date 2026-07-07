# wasi:tls p3 transport spike (task 115 / M1) — END-TO-END VERIFIED

Proves **async TLS-over-TCP with NO `wandr-step-executor`**, over the real **p3
(WASI 0.3) `wasi:tls` + `wasi:sockets`** contracts, host↔guest, against a **live**
endpoint. The whole flow is native async (`resolve.await` / `connect.await` /
handshake `.await`) driven by the host's shared event loop.

## Result (2026-07-07)

```
$ host/target/release/wasi-tls-p3-spike-host \
    target/wasm32-wasip2/release/wasi_tls_p3_spike.wasm example.com
LIVE TLS OK (example.com) -> HTTP/1.1 200 OK
```

A real TLS 1.3 handshake + HTTPS GET, decrypted through the async p3 wasi:tls
receive pipe — zero step-executor.

## Pieces

- **Guest** (`src/lib.rs`, `wasm32-wasip2`) — imports `wasi:tls@0.3.0-draft` +
  `wasi:sockets@0.3.0`; DNS resolve → tcp create+connect → wire the TLS
  connector's `send`/`receive` stream transforms to the socket streams →
  handshake → HTTP GET → read. All `.await`, no reactor.
- **Host** (`host/`, native) — wasmtime 46; links **both** `wasmtime_wasi::p2`
  (std stdio/io @0.2) **and** `::p3` (sockets/io @0.3) + `wasmtime_wasi_tls::p3`;
  runs the `Store` on `call_async`; invokes the async `run` export.

## Build + run

```bash
cargo build --release --target wasm32-wasip2                 # guest
cargo build --release --manifest-path host/Cargo.toml       # host (compiles wasmtime; ~4 min first time)
host/target/release/wasi-tls-p3-spike-host \
  target/wasm32-wasip2/release/wasi_tls_p3_spike.wasm example.com
```

## Gotchas hit (all real, all captured)

1. `generate!` needs **`generate_all`** for multi-package imports.
2. **Don't** pass `async: true` — it forces *every* fn async, breaking the sync
   `create`/`send`/`receive`. Omit it; wit-bindgen respects per-WIT `async func`.
3. Stream API: `StreamWriter::write_all(Vec<T>).await`, `StreamReader::next()`
   yields ONE `T`, `collect() -> Vec<T>`.
4. Host: the p3 modules are behind a **`p3` cargo feature** on `wasmtime-wasi` /
   `wasmtime-wasi-tls`, and wasmtime needs **`component-model-async`**.
5. **Dual-serve is mandatory even for one guest:** the guest imports p3 (my code)
   AND p2 `wasi:cli`/`wasi:io@0.2.6` (pulled in by Rust std). The host must link
   **p2 AND p3** (`wasmtime_wasi::p2::add_to_linker_async` + `::p3::add_to_linker`)
   — a live confirmation of the task-115 blast-radius dual-serve rule.
6. **Do NOT `drop` the cleartext write stream before reading** the response —
   dropping it tears the connection down; the HTTP/1.0 server responds on headers
   while the stream stays open.
