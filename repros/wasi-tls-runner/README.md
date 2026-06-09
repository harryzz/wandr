# wasi-tls Signal-CA runner

Closes the loop left open by [`../wasi-tls-probe`](../wasi-tls-probe): the stock
`wasmtime` CLI trusts only the Mozilla public roots, so the probe's
`chat.signal.org` handshake fails `UnknownIssuer` (Signal pins its own CA). This
runner instantiates the **same probe component** on wasmtime 45 with a custom
`wasmtime_wasi_tls::TlsProvider` whose trust store is **webpki public roots +
Signal's pinned CA** — the exact, minimal host-side change wandr-host would make.

TLS stays entirely host-delegated; nothing in the guest changes.

## Run

```bash
# build the probe component first (if not already):
( cd ../wasi-tls-probe && cargo build --target wasm32-wasip2 --release )

cargo run --release -- \
    ../wasi-tls-probe/target/wasm32-wasip2/release/wasi-tls-probe.wasm
```

## What it does

- `certs/signal-messenger-ca.pem` — Signal's self-signed service CA
  (`O=Signal Messenger, LLC, CN=Signal Messenger`), as served in the
  `chat.signal.org` chain. **Production wandr should pin this from Signal's own
  source**, not the live server.
- Builds `rustls::ClientConfig` with `webpki_roots::TLS_SERVER_ROOTS` + that CA.
- Grants the guest network the way wandr-host would need to:
  `WasiCtxBuilder::inherit_network().allow_ip_name_lookup(true)`.
- Wires `wasmtime_wasi_tls::p2::add_to_linker` (`opts.tls(true)`) and runs the
  probe's `wasi:cli/run`.

## Expected result

Both targets now pass — `example.com` via public roots, `chat.signal.org` via the
added Signal CA — so the probe prints `TRANSPORT PROVEN` and exits 0:

```
[runner] trust store: N public roots + 1 Signal CA cert(s)
[OK]   example.com      — ... status: HTTP/1.1 200 OK
[OK]   chat.signal.org  — ... status: HTTP/1.1 <code>
TRANSPORT PROVEN: wasi-sockets + wasi-tls handshake + HTTP exchange OK
```

(Any HTTP status line from Signal — 200/4xx — means the handshake was trusted;
the request path itself is irrelevant to the transport proof.)
