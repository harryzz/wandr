---
name: project_audio_mic_capture
description: Mic input (audio capture) — open-capture/read-pcm-f32 WIT + host create_capture mirroring task-21 output. DONE + device-verified.
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**Mic capture (audio input) — DONE + device-verified on the Pixel 2 XL
(2026-06-02), commits `df37e397` (de-risk probe) + `1609dbbc` (capture path).**
Builds the input half of the AAudio stack on top of task-21's output half
(see [[project_wasm_runtime]]). Symmetric to playback.

**Permission de-risk first (`--probe-audio-capture`):** a root/su wandr caller
CAN `openStream(AAUDIO_DIRECTION_INPUT)` — the empty-stub `AttributionSource`
suffices (service fills pid/uid from the binder caller); no `RECORD_AUDIO` /
AttributionSource-recursion block. The AVC denial in the log is the audio HAL
reading a sysprop (`hal_audio_default` domain), not us.

**Shape:**
- WIT `interface audio` (skiko-gfx.wit): `open-capture(cfg)->track-handle` +
  `read-pcm-f32(capture, max-frames)->list<f32>`. Capture SHARES the
  track-handle space — `start`/`pause`/`pending-frames`/`close` work on a
  capture handle unchanged. Synced to wandr.ime.keyboard + wandr-app mirrors +
  external/skiko working tree (WIT-sync rule).
- host `audio_impl.rs`: factored `create_track` + `create_capture` into a shared
  `open_pcm_stream(params, channels, capture)`. `create_capture` =
  `AAUDIO_DIRECTION_INPUT` + `inputPreset=VOICE_RECOGNITION`. `read_pcm_f32` is
  the consumer mirror of `write_pcm_f32`: load the service writeCounter
  (Acquire), copy out interleaved f32, advance OUR readCounter (Release).
  `TrackState.read_ctr_ptr` is now `*mut` (capture writes it; output only reads).

**Device quirks (taimen / Pixel 2 XL Adreno):**
- AAudio `Endpoint.aidl` says the record ring "could share same queue" — and
  here the `upDataQueueParcelable` comes back EMPTY (bpf=0,cap=0); the capture
  PCM lands in `downDataQueueParcelable`. So the capture path takes whichever
  data queue the service actually populated (prefer up, fall back to down).
- **Can't hold input + output MMAP endpoints at once** — opening an output
  stream while a capture stream is open returns `-889` (AAUDIO_UNAVAILABLE).
  So in-process mic→speaker loopback playback doesn't run; NOT a capture
  blocker (real use = mic→guest needs only the one input endpoint). The
  `--probe-audio-loopback` probe degrades to capture-only + reports peak/rms.

**Device proof:** capture opens (cap_frames=1536, bpf=4), ring advances at 48kHz
(383520 frames/8s), peak/rms track the mic — quiet room peak=0.02/rms=0.0009,
user speaking peak=1.0/rms=0.0063. Codecs stay guest-side (raw PCM-f32 at the
WIT boundary). Follow-ups: stereo/i16 capture, audio-focus arbiter (now has a
driver), a voice-notes/recorder consumer guest.
