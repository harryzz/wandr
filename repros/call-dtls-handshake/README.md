# call-dtls-handshake — call engine Stage 2: the DTLS-SRTP transport (device-verified)

The transport plane's crypto core: two sans-IO `rtc-dtls` `Endpoint`s (client +
server) complete a real **DTLS handshake** over a loopback "wire", then export the
**SRTP keying material** (RFC 5764) — the REAL keys that replace Stage 1's fixed
key (`../call-media-pipeline`). Proves DTLS-SRTP runs in a `wasm32-wasip2` guest
and produces agreeing, usable keys.

**No fork needed** — `rtc-dtls` builds for wasip2 as-is (`ring`/`rustls`/`rcgen`
all compile, per `../webrtc-rs-wasip2`), and `export_keying_material` is a method
of the public `rtc_shared::crypto::KeyingMaterialExporter` trait (just import it).

## What it does
1. One self-signed cert (rcgen), shared; `with_insecure_skip_verify(true)` = the
   WebRTC fingerprint trust model (peer cert checked out-of-band via SDP, not a CA).
2. Client `Endpoint::connect()` → ClientHello; pump each side's `poll_transmit()`
   into the other's `read()` until both yield `EndpointEvent::HandshakeComplete`.
3. `state.export_keying_material("EXTRACTOR-dtls_srtp", &[], 2*(16+14))` on BOTH
   sides → split RFC-5764 layout `[client_key|server_key|client_salt|server_salt]`.
4. Assert the two independently-derived key sets are IDENTICAL, then build
   `rtc-srtp` Contexts from them and do a protect→unprotect to prove they work.

## Result (2026-06-02) — desktop AND device, both green
```
[dtls] handshake complete in 2 rounds, 6 datagrams
[dtls] negotiated SRTP profile = Srtp_Aes128_Cm_Hmac_Sha1_80
[dtls] exported 60 bytes keying material on each side
[dtls] client and server keying material AGREE ✓
[dtls] SRTP protect/unprotect with the DTLS-derived keys: OK (33→43 bytes)
DTLS-SRTP OK
```
Device run via `wart-host --run-once war.probe.dtls` (Pixel 2 XL). The full DTLS
crypto — ECDHE, certificate sign/verify, the PRF — runs on aarch64. Keys differ
each run (fresh ephemeral handshake), as they should.

## Run
```bash
cargo build --target wasm32-wasip2 --release
wasmtime run target/wasm32-wasip2/release/call-dtls-handshake.wasm   # desktop
# device: package as wasi:cli/command warpkg (war.probe.dtls), then
wart-host --run-once war.probe.dtls
```

## Where this sits — the 3-plane assembly
1. **Media plane** — done (`../call-media-pipeline`).
2. **Transport plane**: **DTLS-SRTP — THIS (done).** Produces the real SRTP keys.
   The remaining half is **ICE (Stage 2b)** — two `rtc-ice` agents establish
   connectivity over `wasi:sockets` UDP (`../wasi-udp-probe`), and the DTLS
   handshake then runs over the selected path instead of the direct loopback here.
   (`rtc-ice` needs the mDNS-optional fork — `../webrtc-rs-wasip2`.)
3. **Signaling** — SDP offer/answer + ICE-candidate exchange with a peer.

Then plug these DTLS-derived keys into the media plane and wire the PCM ends to
our audio WIT, and it's a call (generic/custom; a real Signal peer = ringrtc).
