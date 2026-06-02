# Task 75 — Signal 1:1 calls: ringrtc protocol on the wart-call engine (SCOPED)

**Status:** SCOPED, not started (filed 2026-06-02). A fresh-session task — this
doc is self-contained. Builds directly on the **completed** wart-call engine
([[project_wart_call]]) and the working text Signal client (task 67,
[[project_signal_wasip2_transport_swap]]).

## Goal

Real **1:1 voice calls in the wart Signal app** — place/receive a call to/from a
real Signal client (Android / iOS / Desktop), audio both ways. Reuse the
device-verified `crates/wart-call` engine as the media/transport backend; reuse
the existing encrypted Signal messaging channel (`war.signal`) as the call
signaling transport. **Audio-only, 1:1-only** for v1 (video + group calls
explicitly deferred — see Scope boundaries).

## The key insight — "ringrtc is Rust" is only half the story

ringrtc (`github.com/signalapp/ringrtc`) is **two layers**:
1. **Rust orchestration** (`src/rust/`) — the call-manager state machine, glare
   resolution, multi-ring, and the signaling **message formats**. This part is
   Rust and is the valuable, reusable *spec*.
2. **A patched fork of C++ libwebrtc** — the actual media engine (PeerConnection,
   ICE, DTLS-SRTP, jitter buffer, NACK/RTX, congestion control, Opus/VP8). The
   Rust calls into it over FFI. **This will not compile to wasm32-wasip2** (a huge
   threaded C++ codebase, no wasi-sdk port) — and it's exactly what `wart-call`
   already replaces in pure Rust.

So this task is **not** "compile ringrtc to wasm." It's: **graft ringrtc's Signal
1:1 call protocol onto `wart-call` as the media/transport backend**, in the wart
Signal guest. We already proved the risky half — `wart-call` connects to real
libwebrtc (the browser, `repros/call-browser`) with audio — so a wart-call ↔
ringrtc-libwebrtc call is plausible; the work is matching Signal's exact call
setup.

## What we already have (large head start)

- **`crates/wart-call`** — full WebRTC engine, device-verified + libwebrtc-interop:
  signaling/ICE/DTLS-SRTP/RTP-SRTP/Opus, real UDP, real mic→speaker
  (`repros/call-live` is the on-device capstone). `PeerSession` API.
- **The Signal call signaling envelope is already vendored.**
  `external/libsignal-service-rs/protobuf/SignalService.proto` defines
  `CallMessage { Offer, Answer, IceUpdate, Hangup, Busy, Opaque }` — each carries
  an `opaque` bytes blob. These ride the **same E2E-encrypted message channel**
  `war.signal` already sends/receives (task 67). So the call *transport* exists;
  we send a `CallMessage` like any other Signal message.
- **Audio I/O** — mic capture + AAudio playback wired to wart-call PCM
  (`repros/call-audio-wire`, audibly verified), incl. the 3 device gotchas
  (stereo-only output, no simultaneous in+out MMAP, write-then-start).
- **`wart-arbiter-audio`** ([[project_arbiter_audio]]) — comms-session coordination
  (audio focus, route to earpiece/speaker, `setPhoneState(IN_COMMUNICATION)`, doze
  keep-alive) already built; a call app calls `audio-call-start` on connect.

## The crux — the `opaque` payload (ringrtc's connection parameters)

Signal's call protocol (v4) stopped putting SDP in the `CallMessage` and instead
carries an **`opaque` bytes blob** = a protobuf of ringrtc's own
*ConnectionParameters* / *SenderParameters* (defined in ringrtc's
`protobuf/signaling.proto`, NOT in SignalService.proto). That blob holds: ICE
ufrag/pwd, the DTLS fingerprint, the chosen audio codec (Opus) + payload type +
RTP header-extension ids, and SRTP/transport params — i.e. everything our
`Signaling`/SDP carries, just in ringrtc's wire format. **Decoding + emitting this
blob faithfully is the central protocol task.** First job of the build is to pull
ringrtc's `signaling.proto` and map it ⇄ `wart_call::Signaling`.

## Two approaches (decide in step 1)

- **(A) Vendor ringrtc's Rust, replace its libwebrtc backend with a wart-call
  shim.** Implement the `PeerConnection`/`PeerConnectionFactory` interface ringrtc
  expects, backed by `wart-call`. Pro: reuses Signal's exact call-manager logic
  (glare, multi-ring, timeouts). Con: ringrtc's Rust is deeply libwebrtc-API-
  shaped; the shim is large and drags C++-oriented assumptions into wasm.
- **(B) Reimplement just the Signal 1:1 call protocol on `wart-call`** (RECOMMENDED
  for v1). Write a small `signal-call` module: the 1:1 state machine
  (offer→answer→connected→hangup, glare/busy) + `opaque` ⇄ `Signaling` codec, on
  top of `PeerSession`, sending/receiving `CallMessage`s through the existing
  Signal engine. Use ringrtc's Rust source + `signaling.proto` as the **reference
  spec**. Pro: clean, wasm-native, minimal, no C++-shaped baggage. Con: must match
  Signal's setup faithfully (protocol-matching risk — mitigated by interop tests).

Recommendation: **B**, treating ringrtc as the spec. Revisit A only if the
opaque/state-machine surface proves larger than reimplementing.

## Phased plan

1. **Decide A vs B + decode the opaque format.** Vendor ringrtc's
   `protobuf/signaling.proto`; write `opaque` ⇄ `wart_call::Signaling`
   (ufrag/pwd/fingerprint/Opus PT). Unit-test round-trip against a captured real
   Signal offer blob if obtainable. Deliverable: a decoder + a written A/B call.
2. **Two wart instances call each other over real `CallMessage`s.** Wire a
   `signal-call` module into the `war.signal` engine: place a call (build Offer
   `opaque`, send `CallMessage.Offer`), receive/answer, trickle ICE via
   `IceUpdate`, connect a `PeerSession`, run audio (`call-audio-wire` recipe),
   `Hangup`. Verify between two linked wart devices/accounts. (Engine work; the
   persistent step-executor caveat from task 67 Phase 2 applies — a call needs a
   long-lived driver across component calls.)
3. **INTEROP with a real Signal client** (the real test, mirrors
   `repros/call-browser`). Call a real Signal Android/iOS/Desktop client and get
   two-way audio. This validates the opaque format + ICE/DTLS/Opus params against
   ringrtc-libwebrtc. Expect to iterate on exact param matching (RTP ext ids,
   payload types, DTLS setup role, the multi-ring/`Hangup` semantics).
4. **war.signal UI + arbiter integration.** Incoming-call screen (ring), in-call
   UI (mute/speaker/end), `wart-arbiter-audio` comms session on connect (focus,
   route, `setPhoneState`, doze keep-alive), notification on incoming call (the
   `war:notify` + background-wake primitives already exist). Earpiece-vs-speaker
   routing via the audio-route appliers already built.

## Scope boundaries (v1)

- **Audio only.** Video needs a wasm VP8/VP9 encoder+decoder (large separate
  effort — no proven pure-Rust real-time VP8 in wasm yet) + the video capture/
  render path. Deferred.
- **1:1 only.** Group calls use Signal's **SFU** (group call server) + a different
  protocol (`group_call.proto`, the frame-crypto / MRP layer). Deferred.
- **No call-link / ad-hoc calls.** Deferred with groups.
- Reuses existing crypto: DTLS-SRTP for media + the E2E Signal message channel for
  signaling (the call is authenticated by exchanging the DTLS fingerprint inside
  the E2E-encrypted `CallMessage`). No new crypto primitive needed for 1:1 audio.

## Key risks / unknowns

- **Opaque format drift.** ringrtc's `signaling.proto` is versioned; match the
  current protocol version Signal clients send. Capturing a real offer blob early
  (step 1) de-risks this.
- **Param fidelity for libwebrtc interop.** RTP header-extension ids, Opus PT,
  DTLS setup actpass/active roles, rtcp-mux, ICE-lite vs full — small mismatches
  → no connect or no audio. We have a strong precedent: `call-browser` shook these
  out against libwebrtc (the direction-mirror fix, candidate selection). Same
  class of work.
- **Persistent call driver in the guest.** A call is a long-lived event loop
  (poll_transmit/handle_datagram/audio pump) that must span the guest's component
  lifecycle — the same "step-executor doesn't span component calls" constraint
  task 67 Phase 2 flagged. Likely a dedicated call-active loop in the engine.
- **NAT traversal.** 1:1 calls may need TURN (symmetric NAT). `rtc-turn` is
  sans-IO over the same UDP; Signal provides TURN servers (delivered in the
  call setup). In scope if a direct connection fails; note `repros/wasi-udp-probe`
  already gets the srflx (STUN) candidate.
- **ringrtc license** — AGPL-3.0. Using it as a *reference spec* (reading the
  protocol) vs vendoring its code has different implications; prefer B
  (reimplement from the protocol) partly for this reason. Confirm before vendoring
  any ringrtc source.

## References

- Engine: `crates/wart-call`, `docs/call-engine.md`, `repros/call-{live,browser,
  audio-wire,udp-loopback}`, [[project_wart_call]].
- Signal client: task 67 `tasks/67-signal-client.md`, `apps/user/war.signal/`
  (engine + ui), `external/libsignal-service-rs/` (fork), `protobuf/
  SignalService.proto` (`CallMessage`), [[project_signal_wasip2_transport_swap]],
  [[project_signal_resume_point]].
- Audio: `repros/call-audio-wire`, [[project_audio_mic_capture]],
  [[feedback_aaudio_gotchas]] (3 device gotchas).
- Arbiter comms session: [[project_arbiter_audio]] (M3 audio-call-start/route),
  [[reference_audio_policy_calls]].
- Upstream spec: ringrtc `src/rust/` (call manager) + `protobuf/signaling.proto`
  (the opaque ConnectionParameters) — read as the spec for B.
