# wasi:tls p3 transport spike (task 115 / M1)

Proves a guest can do **async TLS-over-TCP with NO `wandr-step-executor`**, over
the real **p3 (WASI 0.3) `wasi:tls` + `wasi:sockets`** contracts. The whole flow
is native async — `resolve.await`, `connect.await`, handshake `.await` — meant to
be driven by the host's shared event loop.

Status (2026-07-07):
- ✅ **Guest half PROVEN** — `src/lib.rs` compiles to `wasm32-wasip2`, is a valid
  component, and imports the **0.3** interfaces (`wasi:sockets@0.3.0`,
  `wasi:tls@0.3.0-draft`) — not p2. Zero step-executor.
- 🔲 **Host + live run** — next: a wasmtime-46 host linking
  `wasmtime_wasi::p3::add_to_linker` + `wasmtime_wasi_tls::p3::add_to_linker`,
  running the `Store` async (`call_async`), invoking `run("example.com")` against
  a real endpoint (sandbox network is open: TCP 443 + DNS work).

## The p3 API this uses (recon — read-source-first)

`wasi:tls@0.3.0-draft` `client.connector` is a **stream transform** (bring your
own transport):
```
resource connector {
  constructor();
  send(cleartext: stream<u8>)    -> (stream<u8> /*ciphertext*/, future<result>);
  receive(ciphertext: stream<u8>)-> (stream<u8> /*cleartext*/,  future<result>);
  connect: static async func(this, server-name) -> result;   // handshake
}
```
`wasi:sockets@0.3.0` is async too: `tcp-socket.create(family)`,
`connect: async func(addr)`, `send(stream<u8>) -> future`, `receive() ->
(stream<u8>, future)`, `ip-name-lookup.resolve-addresses: async func(name)`.

Flow: resolve → tcp create+connect → wire the TLS connector's send/receive
transforms to the socket's byte streams → handshake → write HTTP over cleartext →
read decrypted response.

## Gotchas hit
- `generate!` needs **`generate_all`** for multi-package imports (else "missing
  `with` mapping"). See `[[project_wasi_canvas_migration]]`.
- **Do NOT** pass `async: true` (forces *every* fn async, breaking the sync
  `create`/`send`/`receive`). Omit it — wit-bindgen respects per-WIT `async func`.
- Stream API: `StreamWriter::write_all(Vec<T>).await`, `StreamReader::collect()
  .await -> Vec<T>` (note `next()` yields ONE `T`).

## Reproduce
```bash
cargo build --release --target wasm32-wasip2
wasm-tools component wit target/wasm32-wasip2/release/wasi_tls_p3_spike.wasm \
  | grep 'import wasi:'          # -> wasi:sockets@0.3.0 + wasi:tls@0.3.0-draft
```
