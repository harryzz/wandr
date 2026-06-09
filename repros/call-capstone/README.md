# call-capstone — the call engine, assembled end-to-end (device-verified)

The capstone. Chains every individually-proven stage into one complete secure
call between two peers (A + B) in a single `wasm32-wasip2` guest:

```
1. signaling   ICE creds + DTLS fingerprint + Opus params exchanged
2. ICE         connectivity checks → selected candidate pair
3. DTLS-SRTP   handshake over the pair → exported SRTP keys (agree on both sides)
4. media       A: tone → Opus → RTP → SRTP ─wire→ B: SRTP → RTP → Opus → PCM
```

Composes `rtc-ice` + `rtc-dtls` + `rtc-srtp` + `rtc-rtp` + `opus-rs`. The wire is
in-memory (real `wasi:sockets` UDP de-risked in `../wasi-udp-probe`); `rtc-ice`
uses the mDNS-optional fork (`../webrtc-rs-wasip2`).

## Result (2026-06-02) — desktop AND device, both green
```
[1/4 signaling] ICE creds + DTLS fingerprints + Opus exchanged
[2/4 ICE] connected — selected pair 127.0.0.1:40001 ↔ 127.0.0.1:40002
[3/4 DTLS] handshake complete — SRTP keys derived + agree
[4/4 media] 25 encrypted Opus frames A→B delivered; B decoded audio (rms=0.0768)
CALL ESTABLISHED — signaling → ICE → DTLS-SRTP → encrypted Opus media,
end-to-end on wasm32-wasip2
```
Device run via `wandr-host --run-once wandr.probe.call` (Pixel 2 XL). A's 440 Hz
tone reaches B through ICE-negotiated connectivity, DTLS-derived SRTP encryption,
and the Opus codec — B decodes non-silent audio (rms 0.077; the SILK
tone-attenuation seen throughout, real speech preserved).

## Run
```bash
# apply the rtc-ice mDNS-optional patch to a webrtc-rs/rtc clone first
cargo build --target wasm32-wasip2 --release
wasmtime run target/wasm32-wasip2/release/call-capstone.wasm   # desktop
# device: package as wasi:cli/command wandrpkg (wandr.probe.call), then
wandr-host --run-once wandr.probe.call
```

## What this means

A complete WebRTC call stack — signaling, ICE/NAT traversal, DTLS-SRTP key
exchange, AES-SRTP media encryption, and Opus — runs in a sandboxed wasm guest
on real Android hardware. Every protocol/crypto/codec piece was de-risked and
device-verified individually, then composed here.

## From capstone to a real feature
- **Real network**: swap the in-memory wire for `wasi:sockets` UDP (proven), with
  STUN/DTLS/SRTP demuxed on one socket by leading byte.
- **Real audio**: wire A's capture end to our mic WIT and B's playback end to
  AAudio (both proven; note the device's input+output MMAP limit for live
  mic↔speaker on one device — fine for a real two-device call).
- **Signaling channel**: exchange the SDP (`../call-signaling-sdp`) + trickled
  candidates over a signaling server.
- **Arbiter**: the comms session / focus / routing / doze-exemption
  (`wandr-arbiter-audio`) already coordinate it.

Caveat unchanged: this is a generic/custom WebRTC call. A real **Signal** call
uses **ringrtc** + Signal's calling service — a separate protocol-interop problem.
