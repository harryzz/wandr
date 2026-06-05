# Task 91 — ART-off call audio (two-way voice in a Signal call under `--no-art`)

> Status: 🚧 SCREEN half FIXED (code, device-verified-mechanism, pending deploy);
> AUDIO half reframed (intermittent, not deterministic). Follow-on to task 87

## DEVICE-VERIFIED DIAGNOSIS (2026-06-05) — supersedes the hypotheses below

Two separate bugs in the `--no-art` call; the proximity probe + a live call settled both:

**SCREEN (proximity → screen-off + touch-disable doesn't engage) — root cause +
fix found, mechanism device-verified.** The whole chain works under `--no-art`:
on a connected call the real proximity sensor reports near on cover
(`x=0`, `ccb_proximityHandle: PRX_STATE nearBy` → `report-sensor proximity 0` →
`sensors: proximity near`), the classifier flips, and with a comms session active
`Event::ProximityChanged{near}` blanks the panel (`set_display_power(false)`) +
suppresses touch on every host — confirmed live (screen ON → cover → OFF →
uncover → ON). The ONLY missing link: the call **never signals the comms session**
— `audio-focus-list count=0`, `proximity holders=0` during a connected call —
because the Signal engine calls `focus::call_start()` **nowhere** (only
`call_end`). It was dropped on purpose: the arbiter coupled `call_start` to
`setPhoneState(IN_COMMUNICATION)`, which ducks the `USAGE_MEDIA` call audio to ~1%
on this device (task 75). **FIX (this task, code done):** decouple them —
`wart-arbiter-audio` `cmd_call_start`/`end` no longer send `audio-policy
set-mode comm`/`normal` (keep `CommsActive`, transient focus, badge); the guest
(`engine.rs`) calls `focus::call_start()` at both connect sites (incoming Accept +
outgoing Connected), symmetric to `call_end`. 16 audio tests pass; engine builds.

**AUDIO (no speech in/out) — reframed: intermittent, NOT a deterministic earpiece
block.** A standalone `--probe-audio-duplex` got `-889` on the earpiece-pinned
output (`deviceIds=[2]`, "could not open in EXCLUSIVE mode"), BUT a *live* call's
same `create_track(call-earpiece, deviceIds=[2])` opened fine (`startClient …
result 0`, full duplex — mic+out audible, user-confirmed). So the earpiece output
is **not** a hard `-889`; the failure is **intermittent MMAP flakiness** (the
task-87-class `-889`/`-895` + `pcm_sync_ptr FD in bad state`). Needs an A/B over
*several* calls to characterize, not a single probe. Separate from the screen fix.

> (Original scoping + the now-superseded hypotheses H1–H4 follow.)
> Status: 🔲 SCOPED + GROUNDED (no device test / patch yet). Follow-on to task 87
> (ART-off audio *output* — ringtones/tones now audible) and task 75 (call-audio
> output, solved under ART). Distinct from both: this is **full-duplex call
> audio** (mic out **and** earpiece/speaker in, at the same time) under `--no-art`.
> See `[[project_call_audio_output]]`, `[[project_audio_mic_capture]]`,
> `[[project_arbiter_audio]]`, `[[project_artless_audio]]`.

## Symptom (user-reported)

The wart **Signal** guest makes/receives calls fine under **ART** with working
voice. Under **`--no-art`**: a call can be placed, an incoming call **rings**
(audible), but the call itself has **no speech audio either direction** — the
other side hears nothing from the mic, and we hear nothing from them.

## The key distinction (why ring works but the call doesn't)

A **ringtone is output-only**, and it's the MMAP path task 87 fixed — so it
plays. A **call is full-duplex**: it opens a **playback** stream *and* a **mic
capture** stream **simultaneously**. That's the new thing under `--no-art`, and
the device (Pixel 2 XL / taimen) has a hard constraint here.

## The mechanism (current code — grounded, not from stale memory)

Full-duplex pump in `apps/user/war.signal/engine/src/call.rs::pump_audio`
(~L257–333). **Order is load-bearing** (the in+out MMAP constraint):
1. Open **OUTPUT first** — `audio::create_track(StreamClass::VoiceCall, stereo,
   USAGE_MEDIA)`. On this device `USAGE_VOICE_COMMUNICATION` output is unopenable
   (`-889`), so call audio goes out as `USAGE_MEDIA`, which **falls back to the
   legacy (non-MMAP) SHARED path**.
2. Then open **CAPTURE** — `audio::open_capture(mono)` → an **MMAP input**
   endpoint. It can coexist **only because the output is legacy/non-MMAP**;
   opening capture first (or holding an MMAP *output*) `-889`s the other.
3. Pump: `read_pcm_f32` → `call.send_audio` (mic→peer); `recv_audio` →
   `play_buf` FIFO → `write_pcm_f32` (peer→speaker).

Host side (`runtime/wart-host/src/audio_impl.rs`): always requests
`AAUDIO_SHARING_MODE_SHARED`; the *service* decides MMAP-vs-legacy. There is
already a **coexistence probe** mirroring exactly this scenario —
`wart-host --probe-audio-loopback` (audio_impl.rs ~L410–468): opens output
(USAGE_MEDIA legacy) then capture and reports **"LEGACY/coexists ✓"** vs
**"MMAP-spin/no-data ✗"**. This is the high-signal diagnostic tool.

## Why it likely breaks under `--no-art` (HYPOTHESES — verify, don't assert)

Cheapest A/B first. The call differs from the working ringtone by (a) a
simultaneous capture and (b) a continuous SHARED output.

- **H1 (prime suspect — the task-87 irony): the call OUTPUT now opens
  MMAP-exclusive under `--no-art`, breaking coexistence.** Task 87 *made the MMAP
  output START path work* (permission/scheduling stubs). The call relies on the
  output being **legacy/non-MMAP** so the **capture MMAP** can coexist. If the
  SHARED output now resolves to an exclusive MMAP backing under `--no-art`, it
  holds an MMAP output endpoint → `open_capture` `-889`s (or MMAP-spins, no data)
  → **mic dead (TX)**, and the output may also wedge → **RX dead**. Net: ring
  (short MMAP, no capture) works; call (output+capture) doesn't.
- **H2: mic capture START is permission/attribution-blocked on the INPUT path.**
  Task 87's `permission`/`IPermissionController` stub unblocked `MmapThread::start`
  for *output*; the *input* MMAP `start` may hit the same
  `checkAttributionSourcePackage` path and never reach RUNNING under `--no-art`.
- **H3: the SHARED output's mixer thread** (`AAudioServiceEndpointShared`) hits a
  residual `--no-art` block for a *continuous* stream that a short MMAP tone
  doesn't (the task-87 `Command 7`/shared-mixer wedge territory).
- **H4 (secondary — route, not silence): comms route not applied.** VoiceCall
  class → host routes via the arbiter's call route (earpiece default). If the
  arbiter `audio-call-start`/route path doesn't fire for the `--no-art` Signal
  call, audio could be mis-routed — but "no audio at all" points more at a
  stream-open/START failure (H1–H3) than a route.

## Diagnosis plan (source-first, then A/B — no blind patching)

1. **Run the coexistence probe** `wart-host --probe-audio-loopback` under
   `--no-art` vs ART-up. If `--no-art` reports "MMAP-spin/no-data ✗" while ART is
   "LEGACY/coexists ✓", H1 is confirmed — the output went MMAP.
2. **A/B a real call** (ART vs `--no-art`): capture host audio logs — `create_track`
   result, `open_capture` result (`-889`?), `aud_tx`/`aud_rx`/`wr_ok`/`wr_zero`/
   `mic_peak` (call.rs diag), and audioserver AAudio command/route + any
   `checkAttributionSourcePackage`/`-889`/`pcm_sync_ptr FD bad state` lines.
3. **Confirm the output sharing reality**: does the call's USAGE_MEDIA SHARED
   output open *legacy* or *exclusive MMAP* under `--no-art`? (audio_policy /
   AAudio logs.) This decides H1.

## Likely fixes per hypothesis (don't pre-commit)
- **H1**: steer the call OUTPUT off MMAP under `--no-art` (e.g.
  `performanceMode=NONE` / the legacy path) so the capture MMAP coexists — restore
  the ART behavior the code already assumes.
- **H2**: extend the task-87 attribution/permission unblock to the capture/input
  MMAP `start`.
- **H3**: a task-87-style native stub for the continuous SHARED path.

## Don't redo / dead-ends (from task 75 + 87)
- Call output is correctly `USAGE_MEDIA` (voice-comm = `-889` here) — **don't**
  switch to voice-comm. The full-duplex pump + play_buf FIFO + render-independent
  bg-tick are present in current code (task 75 work landed).
- `aaudio.mmap_policy=1` is a **dead end** (AAudioService then won't register
  `media.aaudio` at all). Keep `mmap_policy=2`.
- Method: **ART-up vs `--no-art` A/B is the high-signal tool** ("works under ART"
  = diff what `system_server` provides). Don't rewrite the host audio path.
  `[[feedback_read_source_first]]`, `[[feedback_visual_verification]]` (audible
  output needs the user to confirm).

## Source pointers
- `apps/user/war.signal/engine/src/call.rs::pump_audio` (the full-duplex pump).
- `runtime/wart-host/src/audio_impl.rs`: `open_pcm_stream` / `create_track` /
  `create_capture`; `--probe-audio-loopback` (~L410), `--probe-audio-capture`.
- `runtime/wart-host/src/audio_routing.rs` (route → deviceIds pin).
- Task 87 (`tasks/87-artless-audio-output.md`) — the MMAP-START fixes (the 4
  binder stubs) that may have moved the call output onto MMAP (H1).
- `crates/wart-call/src/{signal/call.rs,session.rs}` (send_audio/recv_audio).
