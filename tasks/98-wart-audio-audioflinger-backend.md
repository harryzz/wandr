# Task 98 — `audioclient-rs`: AudioFlinger-direct audio backend (replace hand-rolled AAudio)

> Backend packaged as a standalone reusable crate `crates/audioclient-rs` (lib `audioclient`);
> `wart-host` keeps only the WIT/wasmtime glue. See "Crate packaging & reuse" below.

> Status: 📐 PLAN OF RECORD (2026-06-08). Design decided; implementation not started.
> Supersedes the hand-rolled `media.aaudio` client in `audio_impl.rs` as the audio
> backend. Related: `[[project_call_audioserver_crash]]` (task 97 — why the AAudio
> path fails), `[[project_audio_routing_arbiter]]` (routing stays as-is).

## Why (the journey that led here)

The host's audio backend hand-rolls the **AAudio** client protocol over rsbinder
(`audio_impl.rs::binder_path`: `openStream` → `getStreamDescription` → mmap the FIFO →
`startStream` → write the shared ring). Under `--no-art` this path is unreliable, and
task 97 traced the failures to **AAudioService's control plane**:

- `startStream` returns `-885` (AAUDIO_ERROR_TIMEOUT); `AAudioCommandQueue … Command N
  time out` (START/STOP/CLOSE); the stream goes to `standby_l` after 3 s idle and the
  hand-rolled client never `exitStandby`s; XRUN floods → `Suspending stream what=3`.
- The real `libaaudio` client handles all of this (exitStandby, registerAudioThread,
  `onEventFromServer`, the clock model) — we re-derived the wire protocol incompletely.

Two structural facts make AAudio the wrong base regardless of how complete our client is:

1. **This is a framework, not a call.** The VoIP call is one test consumer. The real
   target is a general audio framework (media/music player "Spotify-like", notifications,
   alarms, games, calls). The universal native base for *all* of these on Android is
   **`AudioTrack`/`AudioRecord` → AudioFlinger** (what ExoPlayer/Media3, MediaPlayer,
   SoundPool, telephony all sit on). AAudio is the **low-latency niche** and lacks the
   media-framework essentials: stream-type volume model, power-efficient `deep_buffer`,
   **compressed offload**, full routing.
2. **AAudioService is the broken layer under `--no-art`.** `AudioTrack` talks **directly
   to AudioFlinger** (`IAudioFlingerService.createTrack` → `IAudioTrack`), so the entire
   AAudioService command-queue/standby machinery — the source of every task-97 bug —
   **does not exist** on this path.

**No off-the-shelf crate fits** (verified): `cpal`/`tinyaudio`/`oboe`/`aaudio` → AAudio
(broken layer; tinyaudio is output-only; cpal too generic for call attributes);
`android-media` → JNI → Java `AudioTrack` → **needs ART (the JVM we deleted)**. There is
no Rust crate wrapping native `libaudioclient` because it's a private C++ system lib with
no stable ABI — so reaching it is inherently an in-tree job (same as `libgui`/`libsf_surface`).

## Decision

**Route R — talk to AudioFlinger directly via rsbinder + the vendored libaudioclient
AIDLs, with a Rust port of the `audio_track_cblk_t` shared-ring proxy.** Pure Rust,
consistent with how the host already does `IAudioPolicyService`, and it **deletes the
AAudioService control plane** that's been failing.

- **Control plane = AIDL (free via rsbinder):** `IAudioFlingerService.createTrack(
  CreateTrackRequest) → CreateTrackResponse{ IAudioTrack, frameCount, portId, … }`, then
  `IAudioTrack.start()/stop()/flush()/pause()/getTimestamp()`. (`IAudioRecord` mirror for
  capture.) `IAudioTrack.start()` is a *direct* call — no command queue, no `-885`.
- **Data plane = NOT in the AIDL:** `IAudioTrack.getCblk()` returns a `SharedFileRegion`
  whose memory is `audio_track_cblk_t` (control block) + the sample buffer. We port the
  `AudioTrackClientProxy` ring protocol (obtain/releaseBuffer, front/rear positions,
  underrun flags) to Rust. Same *shape* as the AAudio FIFO that already worked for
  playback; a non-blocking writer can skip most of the futex (the MixerThread pulls every
  cycle regardless).

**Fallback — Route S** (if the CBLK port's timestamp/clock-model/underrun corners prove
too fiddly): a C++ `libwart_audio.so` shim over native `android::AudioTrack`/`AudioRecord`
(cc-rs, the `libsf_surface`/libgui pattern), which reuses `AudioTrackShared.cpp` for free
at the cost of C++ class ABI (mangling/inline-namespace — a known, solved cost in this
repo). Same external API; only the data-plane owner differs.

### Explicitly NOT used
AAudio / AAudioService / `media.aaudio`; `setPhoneState` (crashes audioserver under
`--no-art`); AAudio `setDevicesRoleForStrategy` route hack as the *primary* lever (the
correct call usage routes via the comms strategy); any JVM/JNI path; cpal / tinyaudio /
android-media / oboe.

## Architecture

```
guest (WIT: audio.create-track / write-pcm / read-pcm; controls; focus)   ← unchanged
        │
wart-host glue (WIT bindings + wasmtime HostState + StreamClass→attributes)
        │  uses ↓
audioclient-rs  (NEW standalone crate — crates/audioclient-rs, lib name `audioclient`)
  ├─ control: rsbinder → IAudioFlingerService.createTrack/createRecord → IAudioTrack/IAudioRecord
  ├─ data:    Rust port of audio_track_cblk_t ClientProxy (write_pcm / read_pcm)
  └─ native API: open_output(attrs) / write / open_input(src) / read / start / stop /
                 close / set_volume / set_output_device / get_timestamp
host routing/volume policy (EXISTING, keep — separate concern)
  └─ rsbinder → IAudioPolicyService (getDevicesForAttributes, setForceUse,
                setDevicesRoleForStrategy, volume) — audio_policy_impl.rs
audioserver: AudioFlinger (createTrack, MixerThread) + AudioPolicyManager   ← survives --no-art
```

- **Stream-class → attributes** (the general-framework dial; extend `StreamClass`):
  - `Media` → USAGE_MEDIA / CONTENT_MUSIC (deep_buffer; offload later) — the music-player case
  - `VoiceCall` → USAGE_VOICE_COMMUNICATION / SPEECH — routes via the comms strategy
    (`setForceUse(COMMUNICATION, SPEAKER|NONE)` — has an earpiece option, unlike FOR_MEDIA)
  - `Notification` / `Alarm` / `Ringtone` / (future) `Game`/low-latency → their usages
- **Routing/volume unchanged:** keep the `IAudioPolicyService` path. `AudioTrack` per-track
  levers (`setVolume` 0..1, `setOutputDevice(port)`) are available as a bonus if needed.
- **WIT/guest unchanged:** `create-track`/`write-pcm-f32`/`read-pcm-f32` + `controls`/`focus`
  keep their signatures; only the host *impl* swaps. No guest recompile, no WIT ABI change.
- **Robust to guest stalls (task-97 bug #1):** the CBLK proxy + AudioFlinger MixerThread
  underrun-fills with silence; no separate host pump needed.

## Crate packaging & reuse (`crates/audioclient-rs`)

The backend ships as a **standalone, reusable crate**, not buried in `wart-host` —
matching the project's HAL-crate pattern (`wart-hal-sensors`/`-display`/`-net`), but
**un-wart-branded** so it's usable from any Android-native Rust project (and publishable).

- **Name:** package `audioclient-rs`, `[lib] name = "audioclient"` → consumers `use audioclient::…`.
  (crates.io: `audioclient`/`audioclient-rs`/`audioclient-sys` all free; `-rs` chosen over a
  bare/`-sys` name — it's not a classic `-sys` linker crate. kebab-case per convention.)
- **Location:** `crates/audioclient-rs` (alongside the reusable `crates/wart-call`), not
  `runtime/` (which holds wart-internal crates).
- **In the crate:** the rsbinder `IAudioFlingerService`/`IAudioTrack`/`IAudioRecord` codegen +
  calls, the `audio_track_cblk_t` `ClientProxy` ring, and the native API above. Optionally a
  sibling `audio-policy` module/crate for the `IAudioPolicyService` routing/volume helpers.
- **Out of the crate (stays in `wart-host`):** the WIT bindings, the wasmtime `HostState`
  wiring, and the wart-specific `StreamClass → audio_attributes` mapping. The crate exposes a
  pure native Rust API — zero WIT/wasmtime — so the host (and the arbiter, and external
  projects) all consume the same thing.
- **`cfg(target_os = "android")`-gated** with a desktop no-op fallback (like the other HAL
  crates) so dependents compile cross-platform.
- **Self-containment:** for wart-internal use it may reference the single vendored AIDL copy
  via a relative path (as `wart-hal-sensors` does → `../wart-host/vendor`); for a *fully
  external/publishable* build it must **vendor its own minimal AIDL set + the
  `audio_track_cblk_t` layout** instead of the relative path.
- **Documented reuse constraints (this is a platform-ABI crate, not a stable-API one):**
  (1) Android-native only; (2) **version-pinned ABI** — the AAudio AIDL transaction layout and
  the private `audio_track_cblk_t` struct are tied to the Android version, so the crate is valid
  across the API range its vendored defs match (state it, e.g. "API 33–35"); (3) needs a
  **privileged context** (system uid / sepolicy) that AudioFlinger accepts.

## AIDL / build changes (`crates/audioclient-rs/build.rs` + `runtime/wart-host/build.rs`)

The `IAudioFlingerService` codegen lives in **the crate's** `build.rs` (rsbinder-aidl
`Builder` — `.source(audioclient_aidl.join("android/media/IAudioFlingerService.aidl"))`,
pulling `IAudioTrack`/`IAudioRecord`/`CreateTrack/RecordRequest/Response`/`SharedFileRegion`
via its include dirs — modeled on `wart-hal-sensors/build.rs`). The host's `build.rs` is
**only** trimmed, never blocked:

- **Crate (`crates/audioclient-rs/build.rs`):** codegen `IAudioFlingerService` + closure
  (vendored-AIDL relative path for wart-internal; vendor-in for a publishable build).
- **Host (`runtime/wart-host/build.rs:668`) — keep:** `IAudioPolicyService` source + the
  `audioclient`/`audio_common`/`av`/`shmem` include dirs (routing stays in the host).
- **Host — drop LATER (cleanup, only after R is device-verified):** the `IAAudioService` +
  `IAAudioClient` sources, the `aaudio_aidl` include dir, and the hand-rolled AAudio client in
  `audio_impl.rs`. Keeping them during bring-up preserves an A/B fallback.

Watch: `CreateTrackResponse`/`CreateTrackRequest` carry nested unions/parcelables — confirm
rsbinder-aidl (pinned git 0.9.0, `[[reference_rsbinder_version]]`) decodes them (it fixed
`AudioPortFw`; verify the createTrack closure similarly).

## CBLK data-plane port spec (the one real piece of work)

Port faithfully from the **vendored reference** (do NOT re-derive):
- Struct: `audio_track_cblk_t` — `vendor/aosp-frameworks-av/include/private/media/AudioTrackShared.h:207`
  (replicate as `#[repr(C)]`; it's a private/version-specific layout — vendor-matched).
- Protocol: `ClientProxy` / `AudioTrackClientProxy` (`obtainBuffer`/`releaseBuffer`, front/rear
  positions masked by `frameCountP2`, `mFutex`, underrun/flags) —
  `vendor/aosp-frameworks-av/media/libaudioclient/AudioTrackShared.cpp`.
- Playback writer (non-blocking): read server position, compute free frames, copy PCM at
  `rear & mask`, advance the client position, publish; carry the remainder (mirror the
  current `write_pcm_f32` semantics). Skip the blocking futex wait; the MixerThread polls.
- Capture (`AudioRecordClientProxy`) is the symmetric reader for `IAudioRecord`.
- `getTimestamp` via `IAudioTrack.getTimestamp` for the clock model (and A/V sync later).

## Critical files

- **NEW crate `crates/audioclient-rs/`** — `Cargo.toml` (package `audioclient-rs`,
  `[lib] name = "audioclient"`, own `[workspace]`, cfg-android deps: rsbinder pin +
  `[build-dependencies] rsbinder-aidl`), `build.rs` (codegen `IAudioFlingerService` closure),
  `src/` (AudioFlinger client + `audio_track_cblk_t` proxy + native API + desktop no-op).
- `runtime/wart-host/Cargo.toml` — add the `audioclient` dependency.
- `runtime/wart-host/src/audio_impl.rs` — replace the hand-rolled `binder_path` (AAudio) with
  **calls into `audioclient`**; keep the module's public API (`create_track`, `write_pcm_f32`,
  `read_pcm_f32`, `start`/`pause`/`close`, `create_capture`) so the WIT/wasmtime glue is intact.
- `runtime/wart-host/src/audio_routing.rs` — `StreamClass`→`audio_attributes_t` mapping passed
  to `audioclient::open_output`; broaden `StreamClass` beyond the call-ish set.
- `runtime/wart-host/src/audio_policy_impl.rs` — unchanged (routing/volume stays in the host).
- `runtime/wart-host/build.rs` — only trim the AAudio sources later (cleanup); routing AIDLs stay.
- Reference (read-only): `vendor/aosp-frameworks-av/media/libaudioclient/{AudioTrack.cpp,
  AudioTrackShared.cpp}`, `include/private/media/AudioTrackShared.h`,
  `media/libaudioclient/aidl/android/media/{IAudioFlingerService,IAudioTrack,CreateTrackRequest,
  CreateTrackResponse}.aidl`.

## Migration sequence (no regressions)

1. ✅ DONE (2026-06-08) — Scaffolded `crates/audioclient-rs` (Cargo + build.rs codegen of
   `IAudioFlingerService`, public API + cfg-android/no-op split, `vendor-aidl.sh`). **Codegen
   verified green** for aarch64-android: `IAudioFlingerService`/`IAudioTrack`/`IAudioRecord` +
   `CreateTrack/RecordRequest/Response` generate AND compile. One stub patch needed —
   `AudioHalCapRule` is recursive (`nestedRules: AudioHalCapRule[]`, rsbinder can't derive); a
   non-recursive build.rs stub (it's an unused HAL-config type) resolves it. Desktop build =
   no-op stubs, clean.
2. Implement the AudioFlinger client + CBLK port in the crate, with a crate-level example/test
   entry; wire a `wart-host --probe-af-tone` that calls `audioclient`, leaving the AAudio path in place.
3. **Verify on device** (`--no-art`): `--probe-af-tone` plays a tone via AudioFlinger,
   no `-885`/command-timeout (there's no AAudioService in the path). Then `--probe-af-loop`
   (capture+playback).
4. Switch `audio_impl.rs` (`create_track`/`write_pcm_f32`/`read_pcm_f32`/`create_capture`) onto
   `audioclient`; device-test a real Signal call (earpiece+speaker, no silence/stall) AND a
   media playback.
5. **Then** remove the AAudio sources/include + the hand-rolled AAudio client (cleanup).

## How to verify (done when)

- A `--no-art` Signal call: peer audio audible and stays audible; earpiece↔speaker toggle
  works; no `-885`/`Suspending stream`/`Command N time out`; UI responsive.
- A media/music stream (USAGE_MEDIA) plays continuously (validates the framework path, not
  just the call).
- No AAudioService (`media.aaudio`) involvement in the call path (`dumpsys media.aaudio`
  shows no endpoints; the track shows under `media.audio_flinger`).

## Risks / fallbacks

- **CBLK struct version match** — `audio_track_cblk_t` is a private layout; must match the
  device's audioserver (vendored `aosp-frameworks-av` is pinned to the device build, like
  the libgui shim). Mismatch → garbage positions → glitch/silence.
- **rsbinder-aidl decode** of the createTrack closure (nested unions) — verify early (step 1).
- **Timestamp/clock-model corners** in the CBLK port — if too fiddly, fall back to **Route S**
  (C++ shim reusing `AudioTrackShared.cpp`). Same WIT/host API, so the fallback is localized
  to the backend module.
