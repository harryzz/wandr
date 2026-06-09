---
name: project_audio_capability_model
description: "Task 76 audio capability probe (steps 1-3) — device port table, AAudio-id==port-id namespace resolution, getDevicesForAttributes-over-binder works, filled state matrix, volume from dumpsys"
metadata: 
  node_type: memory
  type: project
  originSessionId: 60a5ba7d-3852-4a04-bc9b-dc30175ddbfb
---

Task 76 probe phase (steps 1–3) DONE + device-verified on Pixel 2 XL 2026-06-03.
Read-only investigation that lands the device's real audio picture before the
capability-driven refactor. Builds on [[project_call_audio_output]] (task 75).

**Code (uncommitted):** `runtime/wandr-host/src/audio_caps.rs` — `--probe-audio-caps`
(dumpsys parse → typed `AudioDeviceCaps` model + binder reachability) and
`--probe-audio-matrix` (state matrix). Helpers: `audio_impl::probe_open` /
`probe_coexist`, `audio_policy_impl::probe_devices_for_attributes`, and slot-25
`getDevicesForAttributes` filled in `vendor/aidl-stubs/.../IAudioPolicyService.aidl`.
Run on device: `su -c "LD_LIBRARY_PATH=/data/local/tmp <bin> --probe-audio-caps"`.

**Key findings (the deliverable):**
- **AAudio `deviceIds` == audio-policy port id** (same namespace). A default
  `USAGE_MEDIA` open is granted `deviceIds=[3]`=Speaker port. So task-75's
  `deviceIds=[2]` really pinned the Earpiece. Port table: OUT earpiece=2 speaker=3
  telephony-tx=12 speaker-safe=4; IN mic=19 telephony-rx=24 back-mic=20 submix=27.
- **`getDevicesForAttributes` over binder WORKS + decodes cleanly** (incl. the
  `AudioDeviceAddress` union — rsbinder-aidl is **0.8.0**, has union support; the
  "0.7.0" in older memories is stale per [[feedback_check_latest_versions]]).
  Per-usage answer: MEDIA→OUT_SPEAKER(140), VOICE_COMMUNICATION→OUT_SPEAKER_EARPIECE(141),
  NOTIFICATION/ALARM→OUT_SPEAKER_SAFE(142). ⇒ refactor can drive routing from
  binder, not dumpsys (API-of-record / point-G evidence). Policy *prefers*
  earpiece for call audio.
- **Output must be F32 stereo**: mono→-889, I16→-883. `VOICE_COMMUNICATION`
  output →-889 is **mode-independent** (fails in NORMAL and IN_COMMUNICATION).
  **SHARED in+out coexist** (only MMAP pairs contend). IN_COMMUNICATION duck is a
  runtime *volume* effect, not an open failure.
- **Volume read from `dumpsys audio`** (robust): STREAM_MUSIC max 25
  (earpiece 8/speaker 22), STREAM_VOICE_CALL max 15. The ~230-method
  `IAudioService` positional stub for volume *writes* (P8) is **deferred** — a
  WebFetch of the r36 AIDL gave contradictory transaction indices; too fragile to
  land blind (wrong slot could hit a setter). Validate indices vs read-back when
  wiring writes.

**Deferred:** `listAudioPorts` over binder (framework `AudioPortFw` parcelable —
used dumpsys); routing core / volume writes / mic-TX / AEC = task 76 steps 4+.
Point-G "audio API of record" decision still open but the probe favours
binder-for-routing + dumpsys-for-bulk-caps. Full matrix + table in
`tasks/76-audio-subsystem-refactor.md` "Probe results — session 1".
