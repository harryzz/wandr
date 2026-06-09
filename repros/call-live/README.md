# call-live — the call-engine capstone (live call, real mic → real speaker)

Every plane of `wandr-call` composed at once: a **live call** between two
`PeerSession`s over a **real UDP socket** with **real DTLS-SRTP keys**, carrying
**real microphone audio** to a **real speaker**.

```
mic → A: Opus → RTP → SRTP(DTLS keys) ─real UDP→ B: SRTP → RTP → Opus → speaker
```

This is `../call-udp-loopback` (the networked call: ICE + DTLS-SRTP over real
`wasi:sockets` UDP) composed with `../call-audio-wire` (the mic/AAudio wiring) —
no loopback shortcuts. The SRTP keys come from a real DTLS-SRTP handshake and the
encrypted media crosses a real socket; only then does B decode it to the speaker.

Two `PeerSession`s run in one guest (A the caller, B the callee) over loopback
UDP — a real two-device call simply puts A and B on two phones over the LAN
(`PeerSession::new(role, SocketAddr::new(local_lan_ip()?, port))`). It's
**record-then-play** because one device can't hold input + output MMAP at once;
across two phones that limit vanishes (each captures *or* plays).

## Build + run (device)

```bash
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/call-live.wasm components/probe.wasm
wandr-host --install <wandrpkg>                 # app_id wandr.probe.calllive
wandr-host --run-once wandr.probe.calllive      # speak during "on the call"
```

## Result (2026-06-02) — device-verified ✅

```
[calllive] signaling exchanged; connecting over real UDP…
[calllive] CONNECTED — ICE + DTLS-SRTP over real UDP, fingerprint verified
[calllive] on the call — speak now (~3 s)… mic → A ─UDP→ B
[calllive] 150 frames A→B over UDP; B decoded 144000 samples from the wire
[calllive] playing what B received ×30.0 (peak …) — listen…
[calllive] DONE — live call: mic → ICE/DTLS-SRTP/UDP → speaker, on real hardware
```

The mic audio is encoded, encrypted with DTLS-derived SRTP keys, sent across a
real UDP socket, decrypted and decoded by the peer, and played out the speaker —
the complete call, on a Pixel 2 XL. (Audible playback uses the call-audio-wire
recipe: stereo output, write-then-start DMA prime, mic amplified off the noise
floor.)

## What this completes

`wandr-call` is now proven as a full call engine end-to-end on real hardware:
signaling · ICE · DTLS-SRTP · RTP/SRTP · Opus · real UDP · real mic/speaker — and
separately interoperable with a real browser (`../call-browser`). The remaining
work to a product is a **signaling channel** between two real devices (exchange
the SDP + trickle candidates) and the app/UX — not engine work.
