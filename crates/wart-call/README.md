# wart-call

A guest-side Rust library that establishes a **secure real-time audio call** from a
`wasm32-wasip2` component. WebRTC is the first (and current) backend: ICE + DTLS-SRTP
+ RTP/SRTP + Opus + SDP. It runs in the wart runtime's Signal app today and interops
with a real browser (libwebrtc) — `CONNECTED` + audio, device-verified on a Pixel 2 XL.

## WIT-agnostic

Like `dioxus-canvas`, wart-call touches **no host WIT**. It deals only in:
- **PCM f32 frames** — the consumer wires these to the host audio interface
  (capture → `send_audio`, `recv_audio` → playback).
- **opaque UDP datagrams** — the consumer pumps these to/from a socket
  (`wasi:sockets`), and carries the SDP over its own signaling channel.

This keeps the engine portable and unit-testable off-device (`repros/call-*`).

## Layering

The crate is a **general WebRTC engine** with the Signal protocol layered on top behind
a feature flag — not a Signal-only library.

**General core** (always compiled, protocol-agnostic):

| Module | Responsibility |
|---|---|
| `media` | Opus + RTP + SRTP: PCM ⇄ encrypted RTP datagrams |
| `transport` | ICE connectivity + DTLS-SRTP key derivation |
| `session` | [`PeerSession`] — the API that composes the above |
| `signaling` | SDP offer/answer (ICE creds, DTLS fingerprint, Opus rtpmap) |

**Signal backend** (`features = ["signal"]`, opt-in):

| Module | Responsibility |
|---|---|
| `signal/` | ringrtc V4 protocol — `opaque` protobuf signaling, the `accepted` control message, `call_id`; **`SignalCall`** wraps `PeerSession` |
| `turn` | Signal's TURN relay client |
| (in `transport`) | X25519-DH SRTP keying (ringrtc V4), vs the core's DTLS-SRTP |

A WebRTC-native consumer uses `PeerSession` directly (no feature); the Signal app uses
`SignalCall` (`features = ["signal"]`). The lib pulls `prost`/`x25519-dalek`/`hkdf`/
`rtc-turn` only with the feature on.

## Status

- Both directions device-verified clean on a live Signal 1:1 call (Pixel 2 XL,
  `--no-art`): AEC/NS-cleaned mic, no overflow/underflow pops, inbound Opus decoding
  ~93% of packets.
- Each layer is verified individually and composed end-to-end (`repros/call-*`;
  capstone reaches "CALL ESTABLISHED").

## Future / not yet

These are deliberately **out of scope for now** (Opus-only, Signal-first is fine):

- **Multi-codec.** The audio codec is **hardcoded to Opus** (`media.rs` uses concrete
  `OpusEncoder`/`OpusDecoder`). RTP/SRTP framing is codec-agnostic, but adding e.g.
  G.711/AAC means a `trait Codec { encode/decode }` (or enum) in `media.rs`, dispatched
  on the SDP-negotiated payload type — the per-session PT plumbing already exists
  (`Signaling::audio_pt`).
- **More signaling/transport backends.** The general core was kept protocol-agnostic so
  a SIP or Jingle backend could reuse `media` + `transport` without a rewrite. Not built.
- **Jitter buffer / PLC.** Inbound is currently decode-on-arrival (no dejitter, no
  packet-loss concealment). On a LAN this is fine (observed `rtp gaps=0`); for lossy
  off-LAN networks a NetEQ-lite (reorder buffer keyed by RTP seq + Opus PLC via
  `decode(None)` + in-band FEC) would harden quality. Not built.
- **Wider call shapes.** 1:1 only; no group calls, no video.

## Dependencies of note

- `opus-rs` — a **vendored pure-Rust Opus** fork (`external/opus-rs`, path dep), no
  libopus/C. Decodes SILK/Hybrid/CELT incl. Code 3 multi-frame packets per RFC 6716.
- `rtc-*` (ICE/DTLS/SRTP/TURN), `prost` (Signal protobufs, feature-gated).
