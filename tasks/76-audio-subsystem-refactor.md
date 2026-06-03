# Task 76 — Audio subsystem ground-up refactor (capability-driven)

**Status:** CORE DONE + device-verified (2026-06-03), some follow-ups open.
Builds on task 75 ([[project_call_audio_output]]), which got outbound call audio
working but exposed a **hard-coded, fragile, guess-driven audio layer**. This
task replaces it with a **capability-driven** audio subsystem: query the device
for what it actually supports, build a clean model of routing / devices /
channels / volume, and stop hard-coding magic values.

### Progress (commits on `main`)

| Piece | Step | Status | Commit |
|---|---|---|---|
| Capability probe (`--probe-audio-caps`) + matrix | 1–3 | ✅ device-verified | `367121ca` |
| Routing core — `DeviceModel` + `Route`→`StreamPlan` (ports by type) | 4 | ✅ verified | `57cf4506` |
| Host applies arbiter route via per-stream `deviceIds` | 4/6 | ✅ verified (earpiece/speaker) | `323c61fe` |
| WIT `stream-class` intent (guest expresses, host maps) | 7 | ✅ verified | `a965bf0e` |
| Volume get/set/max/min via `IAudioPolicyService` | 5 | ✅ verified (0..25) | `bbd65362` |
| Volume keys — arbiter decides, host applies | 5 | ✅ verified (real keys, on call) | `7535008c`,`87ec38fe` |
| Output MUTE — global (policy) | 5 | ✅ verified | `86e3b8f6` |
| Output MUTE — per-app (host PCM gate) | 5 | ✅ verified | `35d68fee` |

**Architecture of record:** arbiter decides, host applies, guest expresses
intent ([[project_audio_routing_arbiter]]). Routing/volume/mute policy lives in
`wart-arbiter-audio`; the host owns the AAudio/policy binding + capability model.
Mute is two orthogonal gates — `audible = !global_mute && !app_mute`.

**Remaining:** mic-disable / input mute (gated on outbound mic, P1/TX — task #11);
speakerphone microphony/AEC (P2 — task #10); replace startup `dumpsys` port-enum
with binder `listAudioPorts` (task #6); live in-call speakerphone re-pin (task #7);
scaffolding cleanup (design goal #5 — task-75 diag / `COMMS_MODE` / `RX_ONLY` in
Signal); a final full-call re-verify (step 8). Point-G API-of-record settled
(binder-for-routing + dumpsys-for-bulk-caps).

## Why (motivation)

Task 75 shipped working call audio, but every win came from trial-and-error
against a hard-coded `audio_impl.rs`, leaving landmines:

- **Magic constants:** `USAGE_MEDIA` hard-coded (voice-comm → `-889`); earpiece
  pinned to `deviceIds=[2]` (the *audio-policy port id* read once from
  `dumpsys`, NOT necessarily the AAudio device id — different namespace);
  `WART_EARPIECE` env hack; stereo-vs-mono chosen by guesswork.
- **No model of device capability:** we never enumerate output/input devices,
  their ids, supported channel masks / formats / sample rates. We discover
  `-889` (UNAVAILABLE) at runtime by crashing into it.
- **Routing is guesswork:** speaker vs earpiece vs headset, `setForceUse`,
  `setPhoneState`, per-stream `deviceIds` — we tried each blind. The
  media-on-earpiece path even gets policy-attenuated to `volume=0.01` and we
  have no volume control.
- **State space unmapped:** which (usage × phone-mode × device × sharing-mode ×
  format × direction) combinations actually open + play/route is unknown; we
  only know a handful of points (below).

The refactor goal: a small **audio capability/routing core** that *asks the
platform* what's available and routes deliberately — so calls, media, ringtone,
mic, and future video all sit on one coherent, device-independent layer (per the
[[feedback_no_hardcoding]] rule).

## Investigation (do this first — it defines the refactor)

### A. Enumerate what the binder/service layer exposes

For each audio service we can reach over rsbinder, list its methods and what
capability/state each yields. Write a read-only probe (`wart-host
--probe-audio-caps`) that dumps everything to logcat. Services:

- **`media.aaudio`** (`IAAudioService`) — `openStream`/`getStreamDescription`
  (already used). Check: does `StreamParameters` out (`params_out`) report the
  granted device id / channel / format / buffer caps? Use
  `AAudioStream_getDeviceId`-equivalent (the granted `deviceIds` in the
  description) to learn the REAL device id namespace vs the policy port id.
- **`media.audio_policy`** (`IAudioPolicyService`) — the capability goldmine:
  - `listAudioPorts` → every input/output **device + mix port** with ids, types
    (`AUDIO_DEVICE_OUT_EARPIECE`/`_SPEAKER`/`_TELEPHONY_TX`/…), supported
    formats, channel masks, sample rates, gains. THIS is the device-capability
    source of truth (we saw it in `dumpsys media.audio_policy`: "Port ID: 2
    Earpiece, 3 Speaker, 19 Built-In Mic, …").
  - `getDevicesForAttributes(attributes)` → which device a given
    usage/content-type would route to RIGHT NOW (the policy's own answer — stop
    guessing).
  - `setForceUse(usage_category, forced_config)` — the routing lever. Note the
    union shape: `CommunicationDeviceCategory` (NONE/SPEAKER/BT_SCO/BT_BLE/
    WIRED_ACCESSORY) vs `MediaDeviceCategory` (NONE/SPEAKER/HEADPHONES/BT_A2DP/
    docks/WIRED_ACCESSORY/NO_BT_A2DP). MEDIA has no EARPIECE → earpiece on the
    media path needs per-stream `deviceIds`, not force-use.
  - `setPhoneState` (NORMAL/IN_COMMUNICATION/…) — already used via the arbiter.
  - `listAudioPatches` / `getAudioPort` — current active routing (which device a
    stream is patched to; `dumpsys media.audio_flinger` showed our stream patched
    to `AUDIO_DEVICE_OUT_SPEAKER`).
- **`audio`** (`IAudioService`, the framework one) — `setStreamVolume` /
  `getStreamVolume` / `getStreamMaxVolume` (STREAM_MUSIC, STREAM_VOICE_CALL),
  ringer mode, `setMode`. This is where **volume control** (P8) lives. Check
  reachability + SELinux from our domain (read-only probe first).
- **`android.hardware.vibrator`** (`IVibrator`) — already used by the ringer;
  enumerate capabilities (amplitude control, effects) for richer haptics.
- Map each: method → capability/state it reads or sets → does it work from our
  (root) domain or hit SELinux/EX_SECURITY (probe, like the audio-policy probe).

### B. Device enumeration — build the device model

From `listAudioPorts`, produce the actual device table for THIS device:
- **Outputs:** earpiece, speaker, speaker-safe, telephony-tx, wired headset/
  headphone, BT A2DP/SCO/BLE — each with its **port id**, **AAudio device id**
  (verify they match or map), supported **channel masks** (the MMAP "stereo-only"
  claim — confirm), **formats** (PCM_FLOAT/PCM_16), **sample rates**.
- **Inputs:** built-in mic (bottom), back mic, telephony-rx, remote-submix —
  ids, channel masks (mono?), presets.
- Resolve the **AAudio deviceId ↔ AudioDeviceInfo.getId ↔ policy port id**
  question definitively (web refs say AAudio's `setDeviceId` takes the
  `AudioManager.getDevices()` id; our `deviceIds=[2]` used the *policy port id* —
  confirm whether it actually routed to the earpiece or coincidentally worked).

### C. Routing — map every mechanism to its effect

A truth table of routing levers and what each does on THIS device:
- Per-stream `StreamParameters.deviceIds` (pin output/input to a device).
- `setForceUse(MEDIA, …)` and `setForceUse(COMMUNICATION, …)`.
- `setPhoneState(IN_COMMUNICATION)` (+ its ducking side effects on MEDIA).
- Headset auto-routing (wired/BT) for MEDIA — does a connected headset capture
  the stream automatically? Does the stream migrate mid-call?
- For each target (earpiece / loudspeaker / wired / BT-A2DP / BT-SCO): the
  *correct* combination, validated, not guessed.

### D. Channel config (mono/stereo) — where + why

- Document where mono vs stereo is used and the constraint behind each: capture
  = mono (mic); the Pixel 2 XL MMAP output "stereo-only" (verify against
  `listAudioPorts` channel masks); call playback currently stereo (mono Opus
  duped L/R) on USAGE_MEDIA. Decide the right per-path channel config from the
  device's reported masks, not assumption.

### E. Volume

- `setStreamVolume`/`getStreamMaxVolume` for the relevant stream type; wire
  hardware **VOLUME_UP/DOWN keys** → the active stream. Root-cause the
  media-on-earpiece `volume=0.010188` (1%) attenuation (stream-type volume index
  vs a policy earpiece-media curve) and fix it properly.

### G. Choose a STABLE, long-lived audio API foundation (do the research up front)

Before building the routing core, decide **which audio API the whole subsystem
sits on** so it survives Android version bumps and doesn't rot. We currently
hand-roll **raw binder** to `media.aaudio` / `media.audio_policy` (vendored
AIDLs) — fast to reach but the **most ABI-fragile** path (internal/per-version
AIDL, parcelable layout drift — we've already hit this elsewhere). Research +
write up the trade-offs, then pick:

- **Candidates to evaluate** (with sources): the **NDK C APIs** — AAudio
  (recommended for new low-latency audio) and `AudioRecord`/`AudioTrack` (older,
  very stable, NDK `<media/NdkAudio*>`); **Oboe** (Google's recommended C++
  wrapper over AAudio+OpenSL ES — absorbs device/version quirks, is the
  *officially recommended* way and explicitly long-lived); **OpenSL ES**
  (deprecated — rule out); raw **binder AIDLs** (current approach — fastest
  capability access but least stable contract).
- **The key question for OUR framework:** we run host-side Rust, no Java/JNI by
  policy ([[feedback_no_art_layer_dependencies]]), and reach HALs/services over
  rsbinder. Which option gives **stable routing + device enumeration +
  low-latency duplex** that we can call from Rust **without** depending on
  version-specific binder layouts? Options to weigh:
  1. **NDK libaaudio/libOboe via `dlopen`/cc-rs** — link the platform's NDK
     audio .so (stable C ABI, version-managed by the OS) instead of hand-rolled
     binder. Most durable for the media hot path; loses some capability
     introspection (which we'd still get from a thin `IAudioPolicyService` read).
  2. **Keep raw binder but pin to @VintfStability / @stable AIDLs only** — note
     which audio AIDLs are actually stability-tagged vs internal.
  3. **A wart audio HAL abstraction** — define our own stable internal API
     (the WIT + a host trait) and let the *backend* (binder today, NDK later)
     swap underneath without touching guests. (This is the long-lived shape
     regardless — the WIT contract outlives any one backend.)
- **Deliverable of this point:** a short decision note ("audio API of record")
  recorded in the task + a memory, with the rationale, so the refactor builds on
  a deliberately-chosen, durable foundation rather than the current expedient
  raw-binder path. Mirrors how `wasi:tls` (task 66) chose a host-side stable
  surface. Cross-check the crypto/codec API-durability thinking in
  [[project_crypto_hw_offload]].

### F. State matrix — "all possible states, what works / what doesn't"

Build a matrix and fill every cell with WORKS / FAILS(code) / DUCKED / UNTESTED,
from probes + device tests. Axes:
`direction (out/in) × usage × phone-mode (NORMAL/IN_COMMUNICATION) × target
device × sharing-mode (SHARED/EXCLUSIVE) × format (F32/I16) × channels`.
Seed it with what task 75 already established (below), then fill the gaps.

## Known state (from task 75 — seed the matrix; verify, don't trust blindly)

| Config | Result |
|---|---|
| OUT, `USAGE_MEDIA`, NORMAL, speaker, SHARED, F32, stereo | ✅ opens + plays (MMAP -19 → legacy Shared fallback) |
| OUT, `USAGE_MEDIA`, IN_COMMUNICATION, speaker | ⚠️ opens but **ducked to ~1%** + readCounter parked → silent |
| OUT, `USAGE_VOICE_COMMUNICATION`, any mode | ❌ `-889` UNAVAILABLE — MMAP `-19` (no device), **no legacy fallback** (no AAudio mixer profile on the voice/telephony output) |
| OUT, `USAGE_MEDIA`, `deviceIds=[2]` (earpiece port) | ✅ routed to earpiece, BUT policy-attenuated to `volume=0.01` (low) |
| IN (mic), SHARED, F32, mono, VOICE_RECOGNITION preset | ⚠️ opens, but capture thread spun ("processDataNow wait for valid timestamps") + audio didn't reach the peer — needs investigation |
| in+out MMAP simultaneously | ❌ historically `-889` (DMA-endpoint contention; `[[project_audio_mic_capture]]`) — but SHARED+SHARED coexisted in one task-75 test (verify) |
| `aaudio.mmap_policy=1` (force legacy) | ❌ AAudioService stops registering `media.aaudio` entirely on this device — DEAD END |

Decode/codec facts (not routing, but adjacent): peer sends **60 ms Opus**; decode
at the packet's exact TOC sample count (opus-rs 0.1.22 panics on mismatched
frame_size). See [[project_call_audio_output]].

## Probe results — session 1 (steps 1–3, device-verified 2026-06-03)

Implemented `wart-host --probe-audio-caps` (dump + typed model) and
`--probe-audio-matrix` (state matrix), both read-only. Run on the Pixel 2 XL.
Code: `runtime/wart-host/src/audio_caps.rs` (+ `audio_impl::probe_open`/
`probe_coexist`, `audio_policy_impl::probe_devices_for_attributes`, slot-25
`getDevicesForAttributes` in the policy AIDL stub).

**Device table (parsed from `dumpsys media.audio_policy`).** Outputs: Earpiece
**port 2**, Speaker **3**, Telephony-Tx **12**, Speaker-Safe **4** (all
`[dynamic]` profiles — no fixed format/rate/mask). Inputs: Built-In Mic **19**,
Telephony-Rx **24**, Back Mic **20**, Remote-Submix **27** (mic profiles:
PCM_8_24, 8k–48k, masks 0xc/0x10/0x30/0x80000007).

**Namespace resolved (the `deviceIds=[2]` mystery).** A default `USAGE_MEDIA`
open was granted `deviceIds=[3]` = the **Speaker port id**. So **AAudio
`deviceIds` == audio-policy port id** (same namespace) — task-75's `deviceIds=[2]`
genuinely pinned the Earpiece port. The routing core can enumerate ports from
the policy and pin AAudio streams by the same id. (A third namespace exists too:
the common `AudioDeviceType` enum below — distinct from both.)

**`getDevicesForAttributes` over binder WORKS and decodes cleanly** (slot 25;
`AudioDevice[]` incl. the `AudioDeviceAddress` union — rsbinder-aidl **0.8.0**
has union support; the memory's "0.7.0" was stale). Returned, per usage:
`MEDIA → OUT_SPEAKER(140)`, `VOICE_COMMUNICATION → OUT_SPEAKER_EARPIECE(141)`,
`NOTIFICATION/ALARM → OUT_SPEAKER_SAFE(142)`. Accurate + meaningful → **the
refactor can drive routing decisions from binder, not by parsing `dumpsys` at
runtime** (key point-G/API-of-record evidence). Note the policy's *preferred*
call-audio device is the **earpiece** (141).

**State matrix (filled; ✅ = openStream handle>0):**

| Config | Result |
|---|---|
| OUT MEDIA NORMAL default SHARED F32 **mono** | ❌ **-889** — mono output not offered |
| OUT MEDIA NORMAL default SHARED F32 **stereo** | ✅ granted speaker(3), 48k, hwSpf=2 |
| OUT MEDIA NORMAL default SHARED **I16** stereo | ❌ **-883** (INVALID_FORMAT) — F32 required |
| OUT MEDIA NORMAL **EARPIECE(port 2)** SHARED F32 stereo | ✅ (port-id pin works) |
| OUT MEDIA NORMAL default **EXCLUSIVE/MMAP** F32 stereo | ✅ (MMAP opens when uncontended) |
| OUT `VOICE_COMMUNICATION` NORMAL SHARED F32 mono | ❌ -889 (matches task 75) |
| OUT `VOICE_COMMUNICATION` **IN_COMMUNICATION** SHARED F32 mono | ❌ -889 — **mode-independent** |
| OUT MEDIA **IN_COMMUNICATION** SHARED F32 stereo | ✅ opens (task-75 ducking is a runtime *volume* effect, not an open failure) |
| IN VOICE_RECOGNITION NORMAL SHARED F32 mono | ✅ |
| IN default NORMAL SHARED **I16** mono | ❌ -883 — F32 required |
| **in+out SHARED+SHARED simultaneous** | ✅✅ out+in both open (resolves the task-75 ambiguity — SHARED pairs coexist; only MMAP pairs contend) |

New facts vs task 75: **output must be F32 stereo** (mono → -889, I16 → -883);
the `-889` on `VOICE_COMMUNICATION` is **mode-independent**; SHARED in+out
**coexist**. Cross-checks (MEDIA/NORMAL/stereo ✅, VOICE_COMM ❌-889) reproduced
the task-75 table → harness validated. Device left clean (phone state restored to
NORMAL, force-use 0, no stuck streams).

**Volume (read from `dumpsys audio` — the robust source).** Full per-stream
index/min/max table captured: `STREAM_MUSIC` max **25** (earpiece idx 8 /
speaker 22), `STREAM_VOICE_CALL` max **15** (earpiece 5). The ~230-method
`IAudioService` positional stub for volume *writes* (P8) is **deferred** to its
own session — a WebFetch of the r36 AIDL returned contradictory transaction
indices, so a positional stub is too fragile to land blind (a wrong slot could
hit a setter); validate indices against read-back when wiring writes.

**Deferred / not attempted this phase:** `listAudioPorts` over binder (returns
the framework `AudioPortFw` parcelable — used `dumpsys` instead); `IAudioService`
volume stub (above); routing core / volume writes / mic-TX / AEC (steps 4+).

## Refactor design goals (the deliverable)

1. **A capability/device model** built at startup from `listAudioPorts` +
   `getDevicesForAttributes` — not hard-coded ids. One source of truth for
   "what outputs/inputs exist, their ids/masks/formats."
2. **A routing API** that takes intent (media / call-speaker / call-earpiece /
   call-headset / ringtone) and picks the right device + force-use + stream
   params from the model — replacing the `USAGE_MEDIA`/`deviceIds=[2]`/
   `WART_EARPIECE`/`COMMS_MODE`/`NO_TURN` hacks.
3. **Clean WIT** for the guest: express intent (usage/route preference), not
   plumbing. Mono/stereo/format chosen host-side from device capability.
4. **Volume** as a first-class capability (get/set/keys).
5. **Remove the task-75 scaffolding:** experiment flags, the diag log line,
   tick/rtp_diag counters, the hard-coded earpiece port, the env hacks.
6. Keep it **resolution/device-independent** — runs on hardware whose port ids,
   masks, and supported routes differ from the Pixel 2 XL.

## Suggested steps

1. **Probe** (`--probe-audio-caps`, read-only): dump `listAudioPorts`,
   `getDevicesForAttributes` for each usage, current patches, stream volumes,
   `IAudioService`/`IVibrator` reachability + SELinux. Land the raw capability
   picture for this device first.
2. **Device model** in `audio_impl` (or a new `audio_caps.rs`): parse the probe
   data into a typed table; resolve the AAudio-deviceId ↔ port-id namespaces.
3. **Fill the state matrix** with targeted on-device tests (extend the probe).
4. **Routing core**: intent → params/force-use, validated per matrix.
5. **Volume + keys**.
6. **Migrate** call/media/ringtone/mic onto the new core; delete the hacks.
7. **WIT cleanup** + guest migration (Signal call path).
8. Re-verify the full task-75 call flow on the new layer; device-verify.

## Internet references

- AAudio device routing (`setDeviceId` = `AudioManager.getDevices()` id;
  `getDeviceId` to verify the granted device; default = primary output):
  <https://developer.android.com/ndk/guides/audio/aaudio/aaudio> ·
  Audio NDK ref <https://developer.android.com/ndk/reference/group/audio>
- `AudioDeviceInfo` (device types: `TYPE_BUILTIN_EARPIECE`/`_SPEAKER`/
  `_WIRED_HEADSET`/`_BLUETOOTH_A2DP`/`_BLUETOOTH_SCO`/`_BUILTIN_MIC`, `getId()`):
  <https://developer.android.com/reference/android/media/AudioDeviceInfo>
- `IAudioPolicyService.aidl` (listAudioPorts / getDevicesForAttributes /
  setForceUse / setPhoneState — the canonical method list to vendor against):
  <https://android.googlesource.com/platform/frameworks/av/+/refs/heads/main/media/libaudioclient/aidl/android/media/IAudioPolicyService.aidl>
- Configure audio policies (AOSP — how outputs/devices/strategies are defined,
  what the policy can route): <https://source.android.com/docs/core/audio/implement-policy>
- `AudioManager` (setStreamVolume / getStreamMaxVolume / setMode / ringer —
  volume + mode model mirrored by the binder service):
  <https://developer.android.com/reference/android/media/AudioManager>
- Oboe (Google's AAudio wrapper) — device selection + BT-audio routing notes,
  the practical reference for "AAudio device id" semantics + pitfalls:
  <https://github.com/google/oboe/wiki/TechNote_BluetoothAudio> ·
  device-id interference gotcha <https://github.com/google/oboe/issues/1472>

API-durability research (point G — pick the long-lived foundation):
- Oboe — Google's **recommended** audio library; "use Oboe … built on top of
  AAudio … falls back to OpenSL ES" (the official long-lived choice):
  <https://github.com/google/oboe> · <https://developer.android.com/games/sdk/oboe>
- "Update your audio code" / AAudio vs the deprecated OpenSL ES (which API is
  current vs end-of-life): <https://developer.android.com/ndk/guides/audio>
- NDK stable C audio ABIs (`AAudio`, `AMediaCodec`, `AudioRecord`/`AudioTrack`
  NDK) — the version-managed-by-OS surface vs hand-rolled binder:
  <https://developer.android.com/ndk/reference/group/audio>
- AAOSP audio HAL / AIDL stability (`@VintfStability` — which audio binder
  contracts are actually stable vs internal):
  <https://source.android.com/docs/core/audio>

Test assets:
- **Simple PCM test samples at various bit depths / sample rates / channels** —
  handy for validating the playback/capture path + state matrix (feed a known
  tone, confirm format/channels/rate round-trip without depending on a live
  call): <https://mauvecloud.net/sounds/>

## Cross-refs

`[[project_call_audio_output]]` (task-75 findings + the exact WORKS/FAILS),
`[[reference_audio_policy_calls]]` (setPhoneState/setForceUse for calls),
`[[project_audio_mic_capture]]` (capture quirks + in/out MMAP -889),
`[[project_arbiter_audio]]` (AudioService arbiter module — focus/ring/comms),
`[[project_crypto_hw_offload]]` (adjacent; SIMD/codec notes), `[[feedback_aaudio_gotchas]]`,
`[[feedback_no_hardcoding]]`.
