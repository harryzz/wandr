# Task 75 — NEXT SESSION: Signal call audio output (the last mile)

> Handoff for a fresh session. Read `.task-state` too. Everything is committed +
> pushed (HEAD `daaa90b2`). Start with the **repro experiment** in §3 — it splits
> the problem in one shot.

## 1. Where we are

Signal 1:1 calls now work end-to-end **through signaling and crypto** with a real
Signal peer (tested wart ⇄ a real Signal Desktop on account `5b649304`):

- ✅ **Signaling both directions** (offer/answer/ICE/hangup) — `receive_video_codecs`
  + `max_bitrate` fix (`cdeebb16`), ICE-before-Answer fix (`d282c87c`).
- ✅ **Reconnect supervisor + wake-from-sleep watchdog** (`27faf087`).
- ✅ **SRTP decrypt + Opus decode interop** — the hard part. ringrtc feeds the
  **32-byte raw** identity key to the KDF (strips the `0x05` DJB prefix); we sent
  33-byte `serialize()`. Fixed in `e0eed1fb` (`raw_identity_key()` in
  `crates/wart-call/src/transport.rs`). On device: `srtp ok` climbs, `peak=1.000`
  (real decoded voice).
- ❌ **Audio OUTPUT to the speaker** — still silent. THIS is the remaining work.

## 2. The exact remaining bug

The playback ring is fed with perfect audio (`peak=1.000`) but the **AAudio
service `EndpointShared` mixer never pulls our started+registered stream** — its
`readCounter` stays frozen, so after ~2 writes fill the 1536-frame buffer
(`wr_ok=2`), every `write_pcm_f32` is rejected (`wr_zero` climbs) → silence.

Fixed along the way (real bugs, committed):
- **Up-message-queue drain** (`daaa90b2`): the service was hitting
  `writeUpMessageQueue(): Queue full. Did client stop? Suspending stream` and
  closing the stream ~0.4 s after open, because we never drained the endpoint's
  `upMessageQueueParcelable` (server→client events). Now drained in
  `write_pcm_f32`/`read_pcm_f32` (`runtime/wart-host/src/audio_impl.rs`). That
  warning is GONE, but the stream still isn't mixed.
- **Underrun resync** (`181e6d2e`) in `write_pcm_f32`.

Ruled OUT (don't re-chase):
- in+out MMAP coexistence — **receive-only** (mic never opened) is still silent.
- the phone Signal app's audio focus — `org.thoughtcrime.securesms` DISABLED,
  still silent (it WAS grabbing `VOICE_COMMUNICATION` focus per call; now gone).
- comms audio mode — `focus::call_start()` now wired on **outbound connect**
  (engine.rs, sets IN_COMMUNICATION + setForceUse SPEAKER), still silent.
- client registration — the forked child DOES `registerClient ok` (its own pid).
- the up-queue-full suspend — drained (above).

Open leads from the teardown trace (`AAudioServiceEndpoint*`):
- `AAudioServiceEndpointMMAP: onVolumeChanged() volume = 0.010188` — **~1 %
  volume**. Check the stream/MEDIA volume and whether it's effectively muted.
- `AAudioServiceEndpointPlay: callbackLoop() write() … DISCONNECTED` only fired at
  call-END — need the **start-of-call** trace to see if the mix loop ever pulled.

## 3. FIRST STEP — run the repro on-device and compare (decisive)

`repros/call-live` (real mic → real speaker, full crypto) was **device-verified
today** via `wart-host --run-once` — a **direct/standalone** run, NOT the
zygote-forked path the Signal app uses. So the prime hypothesis is
**standalone `--run-once` works, zygote-forked child does not.** Confirm it:

```bash
cd repros/call-live
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/call-live.wasm components/probe.wasm
# pack + install however the README shows; then:
wart-host --run-once war.probe.calllive      # speak during "on the call"; LISTEN
```

- **If call-live PLAYS audio now** → the device/audio stack is fine; the bug is
  **app-vs-repro**. Diff them:
  - process model: `--run-once` (direct) vs **zygote-forked child** under the live
    Hybrid stack ← most likely culprit (audio session / mixer in a forked child).
  - write pattern: call-live **pre-fills a huge buffer + tight write loop**; the
    app streams ~1 frame per call tick (~10–50 ms, gated by `poll_events`).
  - concurrency: call-live plays in a dedicated phase; the app plays under the
    wasm executor alongside the call loop + other stack components.
  - To test the fork hypothesis: get the same playback running in a zygote-forked
    child (e.g. a tiny probe launched via `wart-arbiter launch`, not `--run-once`).
- **If call-live is ALSO silent now** → the device state drifted tonight
  (audioserver bounces, `aaudio.mmap_policy` churn — now back to AUTO=2). It's
  environmental: reboot the device or fully reset audio, re-verify call-live,
  then retry the Signal app.

Capture the **start-of-call** service trace during a Signal call to see the mix
loop decision:
```bash
adb shell 'su -c "logcat -c"'   # clear, then place the call, then:
adb shell 'su -c "logcat -d"' | grep -iE 'AAudioServiceEndpoint|EndpointShared|EndpointPlay|startStream|onVolumeChanged|callbackLoop|MMAP'
```

## 4. Secondary issues (after audio out works)

- **Mic capture** stalls in a call (`audio tx` ~0–3) — the in+out MMAP
  coexistence. `repros/call-live` sidesteps it (record → close mic → play,
  sequential); a real call needs true full-duplex. Likely same root as the output
  issue; revisit once output plays.
- **Inbound ICE never nominates** — wart as the *answerer* (controlled agent)
  reaches `Connecting` but not `Connected`; outbound (controlling) connects fine.
  rtc-ice controlled-side nomination and/or the relay path (deferred Phase B).
- **Relay media (Phase B)** — when media arrives via the TURN relay it isn't
  surfaced to the decoder; only the direct/host fraction decodes.

## 5. Cleanup before finishing (currently in tree for debugging)

- `apps/user/war.signal/engine/src/call.rs`: **`RX_ONLY = true`** in `pump_audio`
  (receive-only experiment) — flip to `false` for full duplex once output plays.
- Remove the engine diagnostic logs once done: `RX OFFER/ANSWER opaque[..]` hex
  dump, the per-second `media …` line (udp/audio/peak/wr/srtp counters), and the
  `call state -> …` line (all in `engine.rs`).
- Re-enable the phone Signal app when finished testing:
  `adb shell su -c "pm enable org.thoughtcrime.securesms"`.
- Device left on `aaudio.mmap_policy=2` (AUTO); a fresh `run-hybrid-stack.sh`
  gives a clean stack.

## 6. Key files

- `runtime/wart-host/src/audio_impl.rs` — AAudio binder-direct path
  (`open_pcm_stream`, `write_pcm_f32`, `drain_up_messages`, `start`).
- `apps/user/war.signal/engine/src/call.rs` — `pump_audio`, `MediaStats`, the
  call engine adapter.
- `apps/user/war.signal/engine/src/engine.rs` — call run-loop, media/state logging.
- `crates/wart-call/src/{transport.rs,session.rs,signal/}` — SRTP keying, media.
- `repros/call-live/` — the working standalone mic→speaker reference.
