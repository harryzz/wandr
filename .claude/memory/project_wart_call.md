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

**SIGNAL 1:1 CALL — BIDIRECTIONAL AUDIO DEVICE-VERIFIED with a REAL Signal peer
(2026-06-03, commit 28926d1f).** Both directions now work end-to-end on the Pixel
2 XL against a real ringrtc client. THE outbound-audio fix: **Opus payload type =
102** (`OPUS_PAYLOAD_TYPE` in lib.rs), NOT 111. ringrtc/Signal uses PT 102 for
Opus; libwebrtc (which ringrtc wraps) accepts our *unsignaled* audio SSRC but only
when the PT matches a registered receive codec — PT 111 (Chrome's default) wasn't
in ringrtc's receive map so it silently dropped our entire outbound stream (we
heard them, they heard nothing). Captured from a live call: peer sends
`peer_pt=102 ssrc=0x7d2`; we send pt=102 ssrc=0xA|0xB (unsignaled SSRC is fine).
Diag tooling: `MediaSession::rtp_peer_ids()` → (pt, ssrc), surfaced in the Signal
engine's `/state/calldbg.log` media-stats line (NOT logcat). **Inbound RX-quality
filter DONE+device-verified (commit 658b929d):** the peer multiplexes streams on
one port (0x7d2/PT102 audio + 0xd/PT101 telephone-event); `recv()` now skips any
non-audio-PT packet (empty vec = benign skip, not error) so the decoder + seq/ts
diag see one coherent stream → rtp gaps 1.4M→0, ts_step steady 2880, decode-err
~15%→~7.6% (residual was NOT loss — it was the Code 3 parser bug, fixed
2026-06-09 commit 8133ef91; see below). **The Opus PT is
now PER-SESSION (not a global const):** WebRTC parses the negotiated SDP rtpmap
(`Signaling::audio_pt`, from_sdp/to_sdp), `set_remote_signaling` adopts the peer's
PT; Signal fixes 102 in `params_into`. Correctly scoped across both backends.
Mic capture: output-first open
ordering (see Signal call.rs) — on taimen, open the USAGE_MEDIA output BEFORE the
capture or capture-first leaves the output unable to open (-889); then mmap-record
HAL captures real full-scale mic (the getInputForAttr EX_ILLEGAL_STATE + MMAP
"wait for valid timestamps" spin are transient/benign).

**OFF-LAN CALLS WORK — TURN Phase B done+device-verified (WiFi↔cellular, audio
both ways; commit ae83f719).** Phase A only allocated the relay + advertised the
candidate; media never traversed it, so calls were LAN-only (direct host↔host).
Phase B (transport.rs): inbound — drain `rtc_turn` `DataIndicationOrChannelData
(peer,data)` and re-demux each payload as if direct from `peer` (STUN→ICE so the
relay-relay connectivity check selects the pair, SRTP→media); outbound — track
`typ relay` remote candidates (`relay_remotes`) and send any datagram addressed
to one THROUGH our relay via `Relay::send_to` (the peer's TURN server only accepts
our permitted relayed address, not raw host/srflx). Media addressing centralized
in `Transport` (`queue_media`/`poll_transmit`); `add_remote_candidate(addr,
is_relay)`. **Blocking opus-rs crash fixed along the way (commit c8576a8f):** a
60ms wideband SILK frame from a low-bitrate off-LAN peer panicked opus-rs 0.1.22's
hardcoded `w_pcm_i16=640` buffer (needs 960*ch) → SIGILL → dead call guest;
vendored `external/opus-rs` fork sizes it `5760*channels` like the siblings, and
wart-call's opus-rs is now a path dep on it. **The "residual inbound loss" was
NOT loss** — FIXED 2026-06-09 (commit 8133ef91): it was an opus-rs **Code 3
(multi-frame) parser bug**. A libopus peer packs 3×20ms Hybrid frames into one
60ms RTP packet (toc=0x7b); the parser (a) branched on the PADDING bit (0x40) to
pick CBR-vs-VBR instead of the VBR bit (0x80) — RFC 6716 §3.2.5 makes them
independent — and (b) used a bogus VBR frame-length encoding (`(b&0x7f)<<8|b1`
instead of `<252→1B / ≥252→first+second*4`). So >half the peer's packets failed
(`decode_ok` froze at 488, `decode_err` climbed; `rtp gaps=0` proved no real
loss). Found by instrumenting the decode-error path (opus error string + failing
TOC → calldbg `dec_err='…' toc=…`), NOT by assuming loss — a jitter-buffer/PLC
build would have fixed nothing (rule #1 win, [[feedback_read_source_first]]).
Device-verified: decode_ok now climbs in lockstep (~93% vs 42%). The dec_err
instrumentation is kept as permanent tooling.

**OUTBOUND pops fixed 2026-06-09 (commit e63b8bc2):** the guest read ONE FRAME
(20ms) of mic per engine tick, but the engine ticks ~35/s while the mic produces
50×20ms frames/s → it fell ~26% behind real-time, the host capture-drain buffer
overflowed + dropped the surplus, and the far side's jitter buffer starved
(interruptions/pops). FIX = the guest drains the WHOLE capture each tick (loop
read_pcm_f32 until a short read), so the full mic stream is sent regardless of
tick rate. Device-verified: audio tx 37/s→~50/s (real-time), far side confirms no
pops. (Capture RING overflow was already 0 via the host capture-drain pump in
audio_impl.rs; the opus ENCODER is single-frame Code 0 and was NOT the cause —
checked.) RULE: a real-time producer/consumer must drain to empty per tick, never
a fixed chunk, when the tick rate is below the media frame rate.

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
