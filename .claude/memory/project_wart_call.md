---
name: project_wart_call
description: "wart-call — the WebRTC call engine crate (pure-Rust, wasm32-wasip2). Device-verified + interops with a real browser (libwebrtc). The big build of 2026-06."
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**wart-call — a WebRTC call engine that runs in a wasm32-wasip2 guest, proven
against a real browser.** `crates/wart-call/` (modules: media/transport/
signaling/session; `PeerSession` API). WIT-agnostic like dioxus-canvas — deals
in PCM f32 frames + opaque `(addr,bytes)` datagrams; the guest wires the PCM ends
to the audio WIT and the datagram ends to `wasi:sockets` UDP. Built on the
pure-Rust webrtc-rs sans-IO `rtc-*` crates ([[reference_audio_policy_calls]],
[[project_crypto_hw_offload]] has the de-risk record). Named wart-call (general),
NOT wart-webrtc — the media core (RTP/SRTP/Opus) + ICE are protocol-agnostic so
SIP/Jingle can reuse them.

**Infrastructure:** `external/rtc` = webrtc-rs/rtc pinned submodule (aae46b7,
pristine upstream) + `tools/scripts/patch-rtc.sh` applies our one delta (rtc-ice
mDNS optional → builds for wasip2; upstream pulls rtc-mdns→socket2/tokio). After
a fresh clone: `git submodule update --init external/rtc && tools/scripts/patch-rtc.sh`.
Codec = `opus-rs` (pure-Rust Opus, ~40x real-time scalar). docs/call-engine.md is
the full design doc.

**Verified, increasingly hard:**
- All layers device-verified individually + the full call end-to-end on the Pixel
  2 XL (`repros/call-*`, `war.probe.{udp,opus,callmedia,dtls,ice,sdp,call,calludp}`).
- Real wasi:sockets UDP + LAN-IP candidate (`local_lan_ip()`) — device-verified.
- DTLS-SRTP fingerprint MITM check (mutual auth, positive+negative tests).
- Cross-impl interop headless vs the webrtc-rs ASYNC `webrtc` crate (separate
  codebase) — `repros/call-interop`.
- **REAL BROWSER (Google libwebrtc): CONNECTED + audio played** (`repros/call-browser`,
  a tiny HTTP harness; user-run). wart-call answers a recvonly offer, connects
  (ICE+DTLS-SRTP), streams an Opus tone Chrome plays. THE definitive proof.

**Gotchas surfaced by real interop (all fixed):** grab ALL SDP candidates not
just the first (browsers offer IPv6/IPv4/host; first was unreachable IPv6); add
all matching-family remote candidates + take the remote addr from the SELECTED
ICE pair; mirror the offer's media DIRECTION in the answer (recvonly offer →
sendonly answer, RFC 3264 — libwebrtc rejects sendrecv). Browser env: disable
mDNS host-candidate obfuscation (.local unresolvable); UDP reachability (not WSL).

**Real audio wiring DONE + AUDIBLY device-verified** (`repros/call-audio-wire`,
`war.probe.callaudio`): a command guest wires MediaSession's PCM ends to the host
`audio` WIT — mic open-capture/read-pcm-f32 in, AAudio create-track/write-pcm-f32
out — and round-trips the live mic through Opus+SRTP back to the speaker on the
Pixel 2 XL. User confirmed: reference tone + their captured VOICE both play out
the speaker. Command+custom-import guest: the wasm32-wasip2 target already emits a
wasi:cli/command COMPONENT with the extra `my:skiko-gfx/audio` import (no
`component new` — just copy the built wasm); the run-once linker adds SkikoUi
which satisfies audio; the audio service is lazy (rsbinder hub OnceLock), works in
any host context. THREE device gotchas (all handled): (1) MMAP **output is
stereo-only** (mono track → -889; play stereo, interleave mono L=R; capture stays
mono); (2) **no simultaneous in+out MMAP** (record-then-play; not a limit for a
real call — each device only captures OR plays); (3) **write-then-start** — the
output is a DMA ring the HAL pulls; `start()` on an EMPTY ring never begins
pulling (writes return 0, ~32 s grind for 4.5 s audio, silence) → PRIME the ring
with PCM first, THEN start ([[feedback_aaudio_gotchas]] flagged this). Mic on this
device sits near the noise floor (peak ~0.008) — a gain question, not wiring.

**CAPSTONE DONE — live call device-verified** (`repros/call-live`,
`war.probe.calllive`): two PeerSessions (A caller, B callee) connect over real
wasi:sockets UDP (ICE + DTLS-SRTP, fingerprint verified); the device MIC feeds
A.send_audio → encrypted RTP crosses the socket → B.recv_audio decrypts+decodes →
plays out the speaker. Real DTLS-derived SRTP keys + real UDP (not loopback keys
like call-audio-wire). 150 frames A→B, B decoded 144000 samples FROM THE WIRE,
real-time playback. Single device = record-then-play (in+out MMAP limit); a real
two-device call splits A/B across two phones via local_lan_ip(). So the full
engine is proven end-to-end on real hardware: signaling·ICE·DTLS-SRTP·RTP/SRTP·
Opus·real-UDP·real-mic/speaker, PLUS browser interop.

**Remaining to a product** (NOT engine work): a signaling channel between two real
devices (exchange SDP + trickle candidates) + app/UX; the wart-arbiter-audio comms
session ([[project_arbiter_audio]]) already coordinates focus/routing/mode/doze.

**SIGNAL 1:1 CALLS scoped as `tasks/75-ringrtc-signal-calls.md`** (do in a fresh
session). Key framing: don't port ringrtc (= Rust orchestration + C++ libwebrtc) —
graft Signal's 1:1 call protocol onto wart-call (which already replaces libwebrtc,
interop-proven). Head start: the `CallMessage`(Offer/Answer/IceUpdate/Hangup/Busy +
`opaque`) envelope is already vendored in SignalService.proto + rides the existing
E2E channel (task 67). Crux = the `opaque` ConnectionParameters blob (ringrtc
`signaling.proto`) ⇄ `wart_call::Signaling`. Recommended approach B (reimplement on
wart-call, ringrtc=spec; AGPL caveat). v1 audio-only + 1:1-only (video/group
deferred). See the task doc for the 4-phase plan + risks. CAVEAT: a real SIGNAL call = ringrtc (a Rust wrapper
over C++ libwebrtc, NOT wasm-viable) + Signal's calling service — separate. SIP/
Jingle would reuse media+transport with a different signaling module.
