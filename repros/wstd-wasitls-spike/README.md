# wstd-wasitls-spike — task-67 async transport foundation (PROVEN)

**Question:** can `wstd`'s `wasi:io/poll` async executor drive an **async**
HTTP/1.1 GET over task-66 `wasi:tls`? (The libsignal-service-rs transport swap
needs a real reactor — `futures::select!`, channels, spawn, timers — not the
sync blocking helpers `repros/wasi-tls-probe` used.)

**Answer: YES (2026-05-30).**
```
# plain wasmtime (no Signal CA) — proves the async pipeline, Signal blocked on pinning:
$ wasmtime run -S inherit-network -S allow-ip-name-lookup -S tls \
    target/wasm32-wasip2/release/wstd-wasitls-spike.wasm
[OK]   example.com    - HTTP/1.1 200 OK         (837 bytes)
[FAIL] chat.signal.org - tls handshake: UnknownIssuer   <- expected; CA not trusted

# through the task-66 runner that injects Signal's CA (host-side TlsProvider):
$ cd ../wasi-tls-runner && cargo run --release -- ../wstd-wasitls-spike/target/wasm32-wasip2/release/wstd-wasitls-spike.wasm
[runner] trust store: 119 public roots + 1 Signal CA cert(s)
[OK]   example.com     - HTTP/1.1 200 OK         (837 bytes)
[OK]   chat.signal.org - HTTP/1.1 404 Not Found  (235 bytes)   <- TLS+HTTP OK; 404 = GET / isn't an endpoint
ASYNC TRANSPORT PROVEN: wstd reactor drove wasi:tls HTTP/1.1
```
The whole DNS → TCP → TLS-handshake → HTTP-write → HTTP-read pipeline ran under
`#[wstd::main]`'s async executor, awaiting each step via the `wasi:io/poll`
reactor. `chat.signal.org` reachability matches task 66 exactly (blocked only on
cert pinning, which wandr-host already fixes via its custom `TlsProvider`).

## Key integration facts (carry into the transport swap)
- **Bindings split that works:** `wstd 0.6.6` (executor, built on the
  `wasip2 1.0.3+wasi-0.2.9` crate — self-contained, no `wit-bindgen-rt`) + our own
  `wit_bindgen::generate!` (0.53) for the `wasi:tls@0.2.0-draft` draft (+ sockets/
  io/clocks). The two runtimes **coexist with no `cabi_realloc` collision.**
- **Pollable bridge:** our wit-bindgen `wasi:io/poll.pollable` and wstd's
  `wasip2::io::poll::Pollable` wrap the same component resource; move the raw
  handle (`take_handle()` → `unsafe from_handle()`) and feed
  `wstd::runtime::AsyncPollable::new(p).wait_for().await`. See `wait_for()` in
  `src/main.rs`. (Could later switch to wit-bindgen `with:` mapping for type-level
  unification, but the handle bridge is robust and version-skew-free.)
- **Drop order matters (component model):** a parent resource must outlive its
  children or you get a runtime `resource has children` trap. Keep the
  `ClientConnection` alive until after its tls input/output streams; keep the
  `TcpSocket` alive until after the connection. Bind, don't `_`-drop, the parent.
- **World:** imports-only (`include wasi:cli/imports` + `wasi:tls/imports`); NO
  exports, so it doesn't clash with `#[wstd::main]`'s `wasi:cli/run` export.
  `cli/imports` is what makes `generate_all` emit sockets.

## Next
Build the transport module in `external/libsignal-service-rs/src/transport/` on
this exact foundation (reqwest-shaped `Client`/`RequestBuilder`/`Response` over
HTTP/1.1 + a `WebSocket` over RFC6455, all on wstd + wasi:tls). See
`tasks/67-signal-client.md` and [[project_signal_wasip2_transport_swap]].
