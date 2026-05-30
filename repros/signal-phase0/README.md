# signal-phase0 — task-67 Phase-0 compile probe (wasm32-wasip2)

**Question:** can the Signal Rust stack compile to `wasm32-wasip2` and run its
networking over the task-66 `wasi:tls` transport (so the Signal client can be a
pure wasm guest, no per-app host code)?

**Verdict (2026-05-30): split result.**
- ✅ **Crypto/protocol half compiles cleanly to `wasm32-wasip2`.** `libsignal-protocol`,
  `zkgroup`, `signal-crypto`, `usernames`, `libsignal-account-keys`, the signal
  `curve25519-dalek` fork, `spqr`, etc. all build for wasip2 once two generic
  (non-wasip2) build-graph issues are fixed (below).
- ❌ **Transport half does NOT build for wasip2.** Current `libsignal-service-rs`
  (main `f93ec5a`) does HTTP + WebSocket via **`reqwest` 0.12 + `reqwest-websocket`**.
  On `target_arch = "wasm32"` reqwest *unconditionally* selects its **browser /
  wasm-bindgen** backend (`web-sys`, `wasm-streams`). That can't be encoded as a
  wasip2 component (needs a JS host wasmtime doesn't provide):
  `wasm-component-ld ... error: failed to encode component` / `could not compile
  wasm-streams`. reqwest has **no wasip2-native backend** —
  [reqwest#2979](https://github.com/seanmonstar/reqwest/issues/2979) (open; the
  whole tokio/socket2/mio/rustls-on-wasip2 stack is still maturing).

## How to reproduce
```
PROTOC=$HOME/tools/protoc/bin/protoc cargo build --target wasm32-wasip2
```

## Generic build-graph fixes needed (NOT wasip2-specific — host tooling)
1. **protoc too old.** System `protoc` was 3.0.0; `libsignal-protocol/build.rs`
   needs ≥3.12 (`--experimental_allow_proto3_optional`). Installed protoc 35.0 at
   `~/tools/protoc/bin/protoc`, pass via `PROTOC=...`.
2. **Two curve25519-dalek copies.** External consumers must replicate libsignal's
   `[patch.crates-io] curve25519-dalek = signalapp fork @ signal-curve25519-4.1.3`
   or `zkgroup` sees two incompatible `RistrettoPoint` types (220 errors). Done in
   this crate's `Cargo.toml`.
3. Dropped `default-features` to skip `cdsi` (libsignal-net / BoringSSL) — not
   needed for link+receive, and dodges the C/BoringSSL cross-build.

## Implication for task 67
Staying guest-side is still right (the crypto runs in-guest; `wasi:tls` is the
generic transport). The only blocker is that `libsignal-service-rs`'s transport
is reqwest, which has no wasip2 backend. Transport code is localized to
`src/push_service/` + `src/websocket/` (reqwest types leak into `push_service/mod.rs`
signatures — not a single pluggable trait). Options under discussion: fork-and-swap
the transport to `wasi:tls`, pin an older trait-based libsignal-service, or drive
`libsignal-protocol` directly + hand-write the service layer. See
`tasks/67-signal-client.md` "Phase 0 result".

Pinned revs: see `SIGNAL-REVS.txt`.
