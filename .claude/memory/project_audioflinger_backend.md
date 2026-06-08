---
name: project_audioflinger_backend
description: Task 98 audioclient-rs — native AudioFlinger-direct audio backend (no AAudioService/JVM); end-to-end audible under --no-art; the 3 bugs solved
metadata: 
  node_type: memory
  type: project
  originSessionId: 7ef4e6b8-b380-4a69-9e9d-1f758a1d9c33
---

**Task 98: `audioclient-rs`** — pure-Rust backend that drives Android **AudioFlinger
directly** (`IAudioFlingerService.createTrack` → `IAudioTrack` → `audio_track_cblk_t`
ring), no AAudioService, no JVM. Decoupled crate (codeberg.org/harryzz/audioclient-rs),
consumed by wart-host. **END-TO-END AUDIBLE under `--no-art`, device-verified** (Pixel 2
XL) via `wart-host --probe-audioclient`. General player/recorder substrate (call audio
in task 97 was just the trigger). See `tasks/98-*.md`.

THREE bugs between "compiles" and "audible":
1. **createTrack silent BAD_VALUE = `AUDIO_SESSION_ALLOCATE` hardcoded `-1`, real value `0`**
   (`system/media/audio`). Server: `else if audio_unique_id_get_use(sessionId)!=USE_SESSION
   → BAD_VALUE goto Exit` — silent, no ALOGE → invisible, identical ART vs --no-art. A
   `[[feedback_no_hardcoding]]` violation. Found by C++ ref byte-diff (only sessionId
   differed `ffffffff` vs `0`).
2. **cblk ring never drained (server underrun→track removed)** — `cblk.rs` end-anchored
   field offsets assuming `u`=32B + cblk_size=region−frameCount·frameSize. Real ABI
   (offsetof on device headers): `sizeof=232 mServer=0 mFutex=8 mBufferSizeInFrames=168
   mFlags=176 mFront=184 mRear=188`; `u`=48B (AudioTrackSharedStatic). Fix = FIXED ABI
   offsets + buffer at `base+232` (`buffers=cblk+1`), NOT region subtraction.
3. **track appops-MUTED** — `Tracks.cpp checkPlayAudioForUsage`: `hasAppOps =
   mPackageName.size() && checkAudioOp(OP_PLAY_AUDIO)==MODE_ALLOWED`, and
   `mPackageName = attributionSource.packageName` (CLIENT-supplied, NOT getPackagesForUid
   → the wart shim CANNOT inject it). Empty package → muted. Fix = valid attributionSource
   (uid=geteuid, packageName); root uid takes `isServiceUid && empty → not muting`, app
   uids pass via non-empty package + shim AppOps MODE_ALLOWED.

**rsbinder recursive Box-array patch** (2 empty `SerializeArray`/`DeserializeArray for
Box<T>` impls, on `5e999e04a`) is needed ONLY for bug #3 (structured recursive
`AttributionSourceState.next[]` → `Vec<Box<Self>>`), NOT for createTrack. Currently a
`[patch]` to `/home/harry/src/rsbinder-patched`; finalize as in-tree vendor or codeberg
fork. Diagnostics kept: `--probe-audioclient[-matrix]`; C++ ref `external/wart-audioclient-ref`
on a-03 (real AudioTrack + offsetof dump + request hexdump). Supersedes the AAudioService
path for output. Follow-ups: wire into host output path, createRecord, volume/timestamp,
pacing (out_write underruns), guest-derived package/uid.
