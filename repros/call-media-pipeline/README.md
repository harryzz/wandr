# call-media-pipeline — call engine Stage 1: the media plane (device-verified)

First assembly of the call engine: composes the three individually de-risked
pieces — **Opus** (`../opus-wasip2`), **SRTP/RTP** (`../webrtc-rs-wasip2`), and the
**audio f32 format** (mic capture + AAudio) — into the full media pipeline a
WebRTC call runs, in a `wasm32-wasip2` guest:

```
PCM ─Opus enc→ payload ─RTP packetize→ pkt ─SRTP protect→ srtp ──┐
                                                                  │ (wire = UDP/ICE later)
PCM ←Opus dec─ payload ←RTP depacketize─ pkt ←SRTP unprotect─ srtp┘
```

`opus-rs` + `rtc-rtp` + `rtc-srtp` all build for wasip2 and compose. Input is a
synthetic 440 Hz tone (a pure pipeline test; live mic↔speaker is blocked by this
device's input+output MMAP limit — see project_audio_mic_capture). A fixed
SRTP_AES128_CM_HMAC_SHA1_80 key + two loopback contexts stand in for the keys the
DTLS-SRTP handshake derives in Stage 2.

## Result (2026-06-02) — device-verified

```
[media] opus=160B → srtp=182B (+22 auth/hdr overhead)        # 12B RTP hdr + 10B HMAC tag
[media] in_rms=0.3543 out_rms=0.0780                          # SILK tone-attenuation (expected)
[media] full pipeline (opus+rtp+srtp, both ways):
          desktop x86  : 0.274 ms / 20ms frame  (73x real-time)
          Pixel 2 XL   : 0.533 ms / 20ms frame  (38x real-time)
MEDIA PLANE OK
```

The 0.533 ms includes **SRTP AES-CM running in-wasm (software AES, no hardware
offload)** — so even unoptimized crypto is comfortably real-time. The host ARMv8
AES offload (`project_crypto_hw_offload`) would make it cheaper / lower-battery,
but it's confirmed *not a blocker*.

## Run
```bash
cargo build --target wasm32-wasip2 --release
wasmtime run target/wasm32-wasip2/release/call-media-pipeline.wasm   # desktop
# device: package as wasi:cli/command warpkg (war.probe.callmedia), then
wart-host --run-once war.probe.callmedia
```

## Where this sits — the 3-plane assembly
1. **Media plane — THIS (done).** Codec + secure-RTP + packetization compose + run real-time.
2. **Transport plane (next).** Two `rtc` endpoints do ICE + DTLS over loopback
   UDP (`wasi:sockets`, `../wasi-udp-probe`) → establish connectivity + derive the
   real SRTP keys (replacing the fixed key here). `rtc-ice` needs the
   mDNS-optional fork (`../webrtc-rs-wasip2`).
3. **Signaling (after).** SDP offer/answer + ICE-candidate exchange with a peer.

Then wire the media plane's PCM ends to our audio capture/playback WIT, and it's
a call (generic/custom; a real Signal peer = ringrtc, separate).
