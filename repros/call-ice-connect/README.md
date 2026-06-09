# call-ice-connect — call engine Stage 2b: ICE connectivity (device-verified)

The transport plane's connectivity half (the other half is DTLS-SRTP,
`../call-dtls-handshake`). Two sans-IO `rtc-ice` Agents (controlling +
controlled) exchange host candidates + ICE credentials and run connectivity
checks (STUN binding request/response) until a candidate pair is selected —
proving the ICE agent reaches connectivity in a `wasm32-wasip2` guest.

The checks flow over an **in-memory loopback wire** (each agent's `poll_write` is
handed to the other's `handle_read`, swapping the transport context). Real
`wasi:sockets` UDP is de-risked separately (`../wasi-udp-probe`); the final
assembly swaps this wire for it. DTLS then runs over the selected pair.

## Needs the mDNS-optional fork
`rtc-ice` pulls `rtc-mdns` (socket2/tokio) which doesn't build for wasip2, so this
uses the **mDNS-optional fork** (`../webrtc-rs-wasip2/rtc-ice-mdns-optional.patch`)
with `default-features = false`. The Cargo.toml path-deps a patched clone of
webrtc-rs/rtc; apply the patch first.

## Result (2026-06-02) — desktop AND device, both green
```
[ice] two agents started (A=controlling, B=controlled)
[ice] connectivity in 0 rounds, 8 STUN datagrams
[ice] A state=Connected B state=Connected
[ice] A selected pair: local=127.0.0.1:40001 ↔ remote=127.0.0.1:40002
[ice] B selected pair: local=127.0.0.1:40002 ↔ remote=127.0.0.1:40001
ICE OK
```
Device run via `wandr-host --run-once wandr.probe.ice` (Pixel 2 XL). 8 STUN
datagrams = the controlling/controlled binding-request/response exchange + the
nomination. No sans-IO ICE example existed upstream — drove `Agent` via
add_local/remote_candidate + start_connectivity_checks + the poll_write/handle_read
pump.

## Run
```bash
# apply the rtc-ice mDNS-optional patch to a webrtc-rs/rtc clone first
cargo build --target wasm32-wasip2 --release
wasmtime run target/wasm32-wasip2/release/call-ice-connect.wasm   # desktop
# device: package as wasi:cli/command wandrpkg (wandr.probe.ice), then
wandr-host --run-once wandr.probe.ice
```

## Where this sits — the 3-plane assembly
1. **Media plane** — done (`../call-media-pipeline`).
2. **Transport plane — COMPLETE**: ICE connectivity (this) + DTLS-SRTP key
   exchange (`../call-dtls-handshake`).
3. **Signaling (next)** — SDP offer/answer + ICE-candidate exchange with a real
   peer (here done in-process; a real call exchanges these over a signaling
   channel).

Then: run ICE → DTLS over the selected pair → feed the DTLS-derived keys into the
media plane → wire the PCM ends to our audio WIT. That's a call (generic/custom;
a real Signal peer = ringrtc, separate).
