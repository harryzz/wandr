---
name: reference_audio_policy_calls
description: "AudioPolicyService for call audio (Signal VoIP + future Phone app) — what IAudioPolicyService gives us, the small host surface needed, and what's NOT audio. Exploration, not yet built."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**Exploration (2026-06-02) of `media.audio_policy` (`IAudioPolicyService`) for
call audio — Signal VoIP calls soon, a Phone Call app later.** Builds on
[[project_audio_mic_capture]] + task-21 output. NOT built yet.

**We use `media.aaudio` (IAAudioService), which BROKERS AudioPolicyService +
AudioFlinger.** For default speaker-out / mic-in we need NOTHING from
AudioPolicy — AAudioService calls `getOutputForAttr`/`getInputForAttr` +
`startInput`/`stopInput` internally (drives the mic privacy indicator too).
`audio: [android.media.IAudioService]` in `service list` is the Java
system_server AudioService — AVOID (ART layer); we talk to the native services.

**TWO call types — very different cost:**
- **VoIP (Signal/WebRTC) = `AudioMode.IN_COMMUNICATION` (3).** The APP pumps PCM
  both ways (our capture + playback). Fully in reach. The hard part is the media
  engine (WebRTC/ringrtc: codec/jitter/RTP/ICE/DTLS) which is GUEST-side app
  logic, not host.
- **Cellular call = `AudioMode.IN_CALL` (2).** Audio is modem-routed (the app
  doesn't pump PCM); the audio part is tiny (setPhoneState + the HAL's voice
  route). The real work is TELEPHONY (RIL/IRadio, ITelephony: dial/answer/SIM/
  network) — a separate, much bigger subsystem than audio.

**The small call-audio control surface on IAudioPolicyService (all PRIMITIVE
args — trivial rsbinder bind, NO recursive parcelables):**
- `setPhoneState(AudioMode state, int uid)` — IN_COMMUNICATION on call start,
  NORMAL on end. Switches the platform into comms routing/AEC tuning.
- `setForceUse(AudioPolicyForceUse usage, AudioPolicyForcedConfig config)` —
  the speaker/earpiece/BT toggle. usage `COMMUNICATION`(0); config `NONE`(0)=
  default earpiece, `SPEAKER`(1)=speakerphone, `BT_SCO`(3)=BT headset.
  (libaudioclient int enums, NOT the newer system union.)
- `getForceUse(usage)` / `getPhoneState()` — read-back (safe to probe).
- (later) `setStreamVolumeIndex` / per-attributes volume = in-call volume;
  `registerClient(IAudioPolicyServiceClient)` = routing-change callbacks
  (watch the rsbinder @nullable-callback gotcha — [[feedback_rsbinder_nullable_callback]]).

**ALREADY in reach via AAudio fields we have (just set them for calls):**
- capture `inputPreset = VOICE_COMMUNICATION` (7, not VOICE_RECOGNITION 6) →
  platform AEC + NS + AGC pre-processing (echo cancel for calls).
- playback `usage = AAUDIO_USAGE_VOICE_COMMUNICATION` (2) + contentType SPEECH →
  earpiece routing + correct ducking.
So a "call mode" on create_capture/create_track = a few enum changes.

**Sidestepped:** `getInputForAttr`/`getOutputForAttr` carry the recursive
`AttributionSourceState` that broke rsbinder before
([[feedback_rsbinder_aidl_recursive]]) — we DON'T call them; AAudioService does.

**De-risk probe — BUILT + read-access verified (commit `d7964c33`).** Binds a
POSITIONAL STUB of IAudioPolicyService (`vendor/aidl-stubs/android/media/
IAudioPolicyService.aidl`): 106 methods kept in order, only 4 real
(setPhoneState 4, setForceUse 5, getForceUse 6, getPhoneState 55), rest
`void slot_N()` — dodges ~100 transitive parcelables incl. the recursive
AttributionSourceState. Enums copied into the stub dir. Wired as a `.source()`
in build.rs (stubs dir already an include_dir). Probes `--probe-audio-policy`
(read) + `--probe-audio-policy-route <speaker|earpiece>` (setForceUse round-trip
that restores). **Index gotcha: a naive awk method-count invented a phantom
method (`id`) → getPhoneState off-by-one (56 vs real 55) → `NotEnoughData`;
re-derive with a comment-stripped `;`-split, 106 methods.** Transaction codes
are positional, so the count MUST be exact.
- **Device (read-only, as root):** media.audio_policy reachable; `getForceUse(
  COMMUNICATION)=NONE(earpiece)` + `getPhoneState` both return cleanly — READ
  ACCESS OK, no perm/SELinux denial.
- **WRITE access VERIFIED (user-authorized run):** `--probe-audio-policy-route
  speaker` → prev=NONE → `setForceUse(COMMUNICATION, SPEAKER)` OK (no perm/
  SELinux denial) → read-back=SPEAKER(1) → restored to NONE. Proves write access
  AND that the read is genuine (read-back=1, not the status header). So root
  wart can DRIVE call routing. `setPhoneState` (index 4) not separately run (most
  intrusive — puts device IN_COMMUNICATION) but same service/perm class as the
  verified setForceUse → expected to work; confirm when building the call path.

**Bottom line: media.audio_policy is fully reachable (R+W) from root wart.** The
audio side of calls is de-risked. SIM note: this device has NO SIM → a cellular
Phone app (IN_CALL + modem/RIL) isn't testable here anyway; Signal/VoIP
(IN_COMMUNICATION) is SIM-independent and the real target. Next concrete build:
capture w/ inputPreset=VOICE_COMMUNICATION (echo-cancel) + playback usage=
VOICE_COMMUNICATION, setPhoneState(IN_COMMUNICATION) on call start, the
setForceUse speaker toggle, and the WebRTC/ringrtc media engine guest-side.

**Audio-focus** (pause-others-when-call-starts) is NOT AudioPolicyService — it
was system_server's AudioService.java (gone post-ART) → it's the wart-arbiter's
job (the audio-focus-arbiter slice; mic capture now gives it a real driver).
