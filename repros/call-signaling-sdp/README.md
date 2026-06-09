# call-signaling-sdp — call engine Stage 3: signaling (device-verified)

The signaling payload. Two peers must agree out-of-band on the parameters the
transport + media stages produce — ICE ufrag/pwd, the DTLS-cert fingerprint, the
Opus rtpmap, ICE candidates — and WebRTC carries them in an **SDP offer/answer**.
This proves a guest can build that payload (`rtc-sdp` marshal), parse it back
(unmarshal), and extract the fields. Pure data: only `rtc-sdp` (no fork, no
sockets, no crypto).

## Result (2026-06-02) — desktop AND device, both green

Generates a complete, valid WebRTC audio SDP offer (494 bytes):
```
v=0
o=- … IN IP4 0.0.0.0
m=audio 9 UDP/TLS/RTP/SAVPF 111
a=ice-ufrag:ufrAgentA
a=ice-pwd:passwordApasswordApasswordA00
a=fingerprint:sha-256 12:34:…
a=setup:actpass
a=rtcp-mux
a=rtpmap:111 opus/48000/2
a=fmtp:111 minptime=10;useinbandfec=1
a=candidate:1 1 udp 2130706431 192.168.1.5 50000 typ host
```
…then `SessionDescription::unmarshal` parses it back and extracts every field
(`ice-ufrag`, `fingerprint`, `setup`, Opus PT 111, `rtpmap opus/48000/2`,
`candidate`). Device run via `wandr-host --run-once wandr.probe.sdp` (Pixel 2 XL).

## Run
```bash
cargo build --target wasm32-wasip2 --release
wasmtime run target/wasm32-wasip2/release/call-signaling-sdp.wasm   # desktop
# device: package as wasi:cli/command wandrpkg (wandr.probe.sdp), then
wandr-host --run-once wandr.probe.sdp
```

## Where this sits — the 3-plane assembly is now COMPLETE
1. **Media plane** — done (`../call-media-pipeline`).
2. **Transport plane** — done: ICE (`../call-ice-connect`) + DTLS-SRTP
   (`../call-dtls-handshake`).
3. **Signaling** — done (this).

Every protocol/crypto/codec/transport/signaling piece of a WebRTC call now runs,
device-verified, in a `wasm32-wasip2` guest. The remaining work is the **final
assembly / capstone**: chain them in one flow —
```
SDP exchange → ICE connectivity → DTLS over the selected pair → feed the
DTLS-derived SRTP keys into the media pipeline → wire the PCM ends to our
audio capture/playback WIT
```
— each link individually proven. That's a working call (generic/custom; a real
Signal peer = ringrtc, separate).
