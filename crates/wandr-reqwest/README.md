# wandr-reqwest / wandr-reqwest-websocket — `reqwest` over `wasi:tls`

Shared guest-side libs: a drop-in `reqwest` / `reqwest-websocket` that is the real
crate on native and a `wasi:tls` implementation on `wasm32-wasip2`, so a wasip2
**guest** can do HTTPS/WebSocket with no in-guest crypto (trust is host-delegated).

Originally the transport for the `libsignal-service-rs` fork (task 67 — Signal as a
guest); now also used by `wandr.audio.player` for internet metadata / cover-art
lookups. The companion async reactor lives in `crates/wandr-step-executor`.

## How it's wired (no source rewrite)
Cargo forbids a single dependency name from having different sources per target,
so we can't just swap `reqwest` for a shim on wasm only. Instead each crate is the
**single source** for its dependency name (via `package =` rename in the consumer's
`Cargo.toml`) and **cfg-dispatches internally**:
- **native:** `pub use reqwest::*;` / `pub use reqwest_websocket::*;` — transparent
  passthrough, so consumers still build + test on desktop.
- **wasm32:** the `wasi:tls` implementation.

## Crates
- **`wandr-reqwest`** — the subset of `reqwest` consumers use: `Client` /
  `ClientBuilder` / `RequestBuilder` / `Response` / `Certificate` / `Error` /
  `multipart`, HTTP/1.1 over `tls::TlsStream`. Trust is host-delegated (the host's
  `TlsProvider`), so `Certificate` is a no-op. Owns the shared `tls` module +
  `random_bytes` (`wasi:random`).
- **`wandr-reqwest-websocket`** — RFC6455 client `WebSocket` over the same
  `TlsStream`, with the inherent async `send` / `next` / `close` + `Message` /
  `CloseCode`.

Consumed as a path dep with a `package =` rename, e.g.:
```toml
reqwest           = { path = "../../crates/wandr-reqwest",           package = "wandr-reqwest" }
reqwest-websocket = { path = "../../crates/wandr-reqwest-websocket", package = "wandr-reqwest-websocket" }
```
