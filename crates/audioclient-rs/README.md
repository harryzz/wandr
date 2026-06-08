# audioclient-rs

Rust client for Android's **native AudioFlinger** (`libaudioclient`) — `AudioTrack` /
`AudioRecord` over binder, **no AAudioService, no JVM**. Library name: `audioclient`.

It calls `IAudioFlingerService.createTrack`/`createRecord` → `IAudioTrack`/`IAudioRecord`
and drives the shared `audio_track_cblk_t` ring directly — the same wire path native
`android::AudioTrack` uses, one level below Java `android.media.AudioTrack` (needs ART) and
beside AAudio's `AAudioStream` (whose service control plane is unreliable under `--no-art`).

> **Status: SCAFFOLD.** Public API + binder/codegen wiring are in place; the AudioFlinger
> client and the `audio_track_cblk_t` proxy are `TODO(task 98)`. On Android the calls warn +
> no-op for now. Design/plan: `tasks/98-wart-audio-audioflinger-backend.md` in the wart repo.

## Reuse constraints (platform-ABI crate, not a stable-API one)
- **Android-native only** (links rsbinder); off-android every call is a no-op.
- **Version-pinned ABI** — the AAudio AIDL layout + the private `audio_track_cblk_t` struct
  are Android-version-specific. Target: API 33–35 (Android 13–15).
- **Privileged context** — AudioFlinger permission checks need a system uid / sepolicy domain.

## AIDL vendoring
The codegen (`build.rs`) needs the libaudioclient AIDL closure. Two modes:

- **Wart-internal (default):** reads the device-matched AOSP **git submodule** vendored at
  `../../runtime/wart-host/vendor` — zero duplication. Nothing to do.
- **Self-contained / publishable:** run `./vendor-aidl.sh` to copy the minimal AIDL set into
  `./aidl/` (build.rs then prefers it). The AOSP version **must match the target device's
  audioserver**; the script copies from the pinned submodule by default (or pass a source root).
  Commit `./aidl/` to ship the crate standalone.
