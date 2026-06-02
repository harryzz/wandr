# Call engine (wart-call)

A secure real-time audio call from a `wasm32-wasip2` guest. WebRTC is the first
backend; the design keeps the reusable parts (media, ICE) protocol-agnostic so
SIP/Jingle can slot in later.

Status (2026-06-02): **complete + device-verified, including real audio + a real
browser.** Every protocol/crypto/codec/transport/signaling layer runs in a wasm
guest on a Pixel 2 XL, individually (`repros/call-*` + `repros/{wasi-udp,opus}-*`)
and composed into a **live call carrying real mic audio to the speaker over real
UDP with real DTLS-SRTP keys** (`repros/call-live` → "live call: mic →
ICE/DTLS-SRTP/UDP → speaker"). It also interoperates with a real browser
(libwebrtc) — connection + audio (`repros/call-browser`). The library form is
`crates/wart-call` (the `PeerSession` two-peer test reproduces it).

## The three planes

| Plane | What | Shared by |
|---|---|---|
| **media** | RTP + SRTP + Opus: PCM ⇄ encrypted RTP datagrams | WebRTC, SIP, Jingle |
| **transport** | ICE connectivity + DTLS-SRTP key exchange | WebRTC, modern SIP, Jingle |
| **signaling** | SDP offer/answer + ICE candidates | WebRTC only (SIP/Jingle differ) |

The media plane and the ICE transport are protocol-agnostic; only signaling and
key-exchange are WebRTC-specific — which is why the crate is `wart-call`, not
`wart-webrtc`.

## crates/wart-call

WIT-agnostic (like `dioxus-canvas`): it deals only in **PCM f32 frames** and
**opaque datagrams**. The consuming guest wires the PCM ends to the host audio
interface and the datagram ends to a UDP socket, and carries the SDP over its own
signaling channel.

```
crates/wart-call/src/
  media.rs       MediaSession  — Opus + RTP + SRTP, send(pcm)→srtp / recv(srtp)→pcm
  transport.rs   Transport     — ICE + DTLS, STUN/DTLS/SRTP demux, SRTP key export
  signaling.rs   Signaling     — SDP to_sdp() / from_sdp()
  session.rs     PeerSession   — the API; offer/answer, poll_transmit/handle_datagram,
                                 handle_timeout, send_audio/recv_audio
```

`PeerSession` is event-loop-driveable: the guest feeds it inbound datagrams +
time, drains outbound datagrams, exchanges SDP, and pumps PCM in/out (see the
doc-comment example).

## external/rtc + the patch

The Rust WebRTC crates are `webrtc-rs/rtc` (the **sans-IO** design — protocol
state machines with no baked-in tokio/sockets, which is what makes them fit a
single-threaded wasm guest). Pinned as a submodule at `external/rtc`; we carry
one small delta — **rtc-ice's mDNS made optional/default-on** so the ICE crate
builds for wasip2 (upstream pulls rtc-mdns → socket2/tokio). Apply it after a
fresh clone:

```
git submodule update --init external/rtc
tools/scripts/patch-rtc.sh
```

The codec is `opus-rs` (pure-Rust Opus, RFC 6716 — no C/wasi-sdk; ~40× real-time
scalar on the Pixel 2 XL). UDP is `wasi:sockets` (no host code needed). Hot-path
SRTP crypto can later be offloaded to host ARMv8 hardware AES for battery — see
`.claude/memory/project_crypto_hw_offload.md` — but in-wasm software AES is
already comfortably real-time (the media pipeline is 38× real-time).

## De-risk record (repros/)

Each is a `wasi:cli/command` warpkg, device-verified via `wart-host --run-once`:

| repro | proves | probe |
|---|---|---|
| `wasi-udp-probe` | wasi-sockets UDP + STUN srflx | `war.probe.udp` |
| `opus-wasip2` | pure-Rust Opus, 40× real-time | `war.probe.opus` |
| `call-media-pipeline` | media plane, 38× real-time | `war.probe.callmedia` |
| `call-dtls-handshake` | DTLS-SRTP keys (agree) | `war.probe.dtls` |
| `call-ice-connect` | ICE connectivity | `war.probe.ice` |
| `call-signaling-sdp` | WebRTC SDP | `war.probe.sdp` |
| `call-capstone` | end-to-end call | `war.probe.call` |
| `call-udp-loopback` | a call over real wasi:sockets UDP (LAN IP) | `war.probe.calludp` |
| `call-audio-wire` | **PCM ends wired to real mic/AAudio** (mic→engine→speaker) | `war.probe.callaudio` |
| `call-live` | **the capstone** — live call: mic→DTLS-SRTP/UDP→speaker | `war.probe.calllive` |
| `call-interop` | **interop with an independent WebRTC stack** (native) | — |
| `webrtc-rs-wasip2` | the rtc-ice mDNS-optional patch + spike notes | — |

**Interop is proven against real WebRTC — including a real browser.**
- `repros/call-interop` (headless): wart-call connects to the webrtc-rs **async**
  `webrtc` crate — a separate codebase (its own webrtc-ice/webrtc-dtls). Both
  reach `Connected` (ICE + DTLS-SRTP over real UDP).
- `repros/call-browser` (a real browser): **Chrome/Firefox = Google libwebrtc**
  offers a recvonly audio call; wart-call answers, connects, and **streams an Opus
  tone the browser plays** — confirmed `CONNECTED — wart-call ↔ browser ✓` with
  audio. So wart-call's SDP/ICE/DTLS-SRTP/Opus interoperate with the actual
  reference WebRTC implementation, media included.

The only fix the browser demanded was mirroring the offer's media **direction**
in the answer (a `recvonly` offer needs a `sendonly` answer — RFC 3264). Two
environment notes for `call-browser`: disable the browser's mDNS host-candidate
obfuscation (wart-call can't resolve `.local`), and run the harness where the
browser can reach it over UDP (not WSL).

## From here to a shippable call

The `PeerSession` API is the engine; what remains is integration:

1. **Real UDP** — DONE. `PeerSession::new(role, local_addr)` takes the bound
   socket address; `poll_transmit()` yields `(dest, bytes)` to `send_to`,
   `handle_datagram(src, bytes)` takes what `recv_from` got; candidates carry the
   real addresses via SDP. Device-verified: `repros/call-udp-loopback`
   (`war.probe.calludp`) runs a full call between two `wasi:sockets` UDP sockets
   in a guest on the Pixel 2 XL — over the device's real LAN IP via
   `wart_call::local_lan_ip()` (device-verified: the guest discovers the Pixel's
   WiFi IP, e.g. `192.168.1.173`, the address a peer on the same network reaches).
2. **Real audio** — DONE. `repros/call-audio-wire` (`war.probe.callaudio`) wires
   `MediaSession`'s PCM ends to the host `audio` WIT — mic `open-capture`/
   `read-pcm-f32` in, AAudio `create-track`/`write-pcm-f32` out — and round-trips
   the live mic through Opus+SRTP back to the speaker on a Pixel 2 XL —
   **audibly verified** (a reference tone + the captured voice both play out the
   speaker). Three device gotchas handled: the MMAP **output is stereo-only**
   (play a stereo track, interleave the mono pipeline output L=R; capture stays
   mono); there's **no simultaneous input+output MMAP** (so it records-then-plays
   — not a limit for a real call, where each device only captures *or* plays); and
   the output is a **DMA ring that needs write-then-start** (prime it with PCM
   before `start()`, else the HAL never begins pulling → silence). Composing this
   with a networked `PeerSession` (step 1) is a complete live call.
3. **Signaling channel** — exchange the SDP (`Signaling::to_sdp`/`from_sdp`) +
   trickled candidates over a signaling server. (The DTLS-cert fingerprint is
   real: `transport.rs` computes SHA-256 over the cert DER, carries it in the
   SDP, and verifies the peer's handshake cert against it — mutual-auth MITM
   prevention, the WebRTC trust model. `mismatched_fingerprint_rejected` proves
   a swapped cert is refused.)
4. **Coordination** — `wart-arbiter-audio` already provides the comms session
   (focus / routing / mode / doze-exemption); a call app calls `audio-call-start`
   when a `PeerSession` connects.

## Caveat — Signal

This is a **generic/custom** WebRTC call. A real **Signal** call uses **ringrtc**
(Signal's libwebrtc wrapper) + Signal's calling service — a separate
protocol-interop problem, out of scope for wart-call's first backend.
