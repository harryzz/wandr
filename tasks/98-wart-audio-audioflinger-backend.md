# Task 98 — native AudioFlinger backend (`audioclient-rs`)

A general, reusable audio backend that talks to Android's **AudioFlinger directly**
over binder (`IAudioFlingerService.createTrack` → `IAudioTrack` → the shared
`audio_track_cblk_t` ring), **without AAudioService and without the JVM**. The
call-audio path was only the trigger (task 97); this is the player/recorder
substrate (Spotify-like + VoIP). Lives in its own decoupled crate
`audioclient-rs` (codeberg.org/harryzz/audioclient-rs), consumed by wart-host.

## Status — END-TO-END AUDIBLE under `--no-art` (device-verified)

The full output path works on the Pixel 2 XL: `createTrack` → mmap the cblk ring →
write PCM → the AudioFlinger MixerThread drains it to the HAL, **unmuted**. Proven
with `wart-host --probe-audioclient` (a 440 Hz tone) and the
`--probe-audioclient-matrix` diagnostic.

## Architecture
- **Control plane** (`src/track.rs`): `IAudioFlingerService.createTrack` with a
  hand-built `CreateTrackRequest`; `getCblk` → `mmap` → `ClientProxy`.
- **Data plane** (`src/cblk.rs`): pure-Rust `audio_track_cblk_t` client ring proxy
  (`obtainBuffer`/`releaseBuffer`/`write`/`read`), ported from
  `AudioTrackShared.{h,cpp}`.
- **Off-Android**: every call is a no-op stub so dependents build cross-platform.
- rsbinder = wart-host's pinned `5e999e04a` **+ a 2-line recursive Box-array
  patch** (see below).

## The three bugs that stood between "compiles" and "audible"

### 1. `createTrack` → silent `BAD_VALUE` — **`AUDIO_SESSION_ALLOCATE` was wrong**
`track.rs` hardcoded `AUDIO_SESSION_ALLOCATE = -1`; the real constant is **`0`**
(`system/media/audio`). AudioFlinger does
`if (sessionId == ALLOCATE) newId(); else if (audio_unique_id_get_use(sessionId)
!= USE_SESSION) { lStatus = BAD_VALUE; goto Exit; }` — a **silent** reject (no
ALOGE) for `-1`. This is why audioserver logged nothing (even at VERBOSE), why it
was identical under ART and `--no-art`, and why no request-field variant fixed it.
**Found by** building a C++ reference (`external/wart-audioclient-ref` on a-03) that
calls `createTrack` with our exact values via the device's own libaudioclient — it
**succeeded sending `sessionId=0`** — then byte-diffing its serialized request vs
ours (only the last 4 bytes differed: `ffffffff` vs `00000000`). A
no-hardcoding-rule violation (`[[feedback_no_hardcoding]]`).

### 2. cblk ring never drained (server `BUFFER TIMEOUT … underrun` → track removed)
`cblk.rs` derived the control-block size as `region − frameCount·frameSize` and
**end-anchored** the field offsets, assuming `u` (the union) was 32 bytes. The real
`audio_track_cblk_t` (ABI via `offsetof` on the device headers) is:
`sizeof=232, mServer=0, mFutex=8, mBufferSizeInFrames=168, mFlags=176,
u.mStreaming.mFront=184, mRear=188` — `u` is **48 bytes** (`AudioTrackSharedStatic`
is bigger than `mAlign[8]`). So `mRear` was written to the wrong offset → the
server saw `rear=0` → underrun. **Fix**: fixed ABI offsets + the buffer at
`base + 232` (`buffers = cblk + 1`), not `base + (region − frameCount·frameSize)`.
After the fix the tone writes ~120k–220k frames (`zero-ticks≈1`) and reaches
`audio_hw_primary out_write`.

### 3. Track appops-muted (`OpPlayAudioMonitor`) — needs a non-empty `packageName`
`Tracks.cpp checkPlayAudioForUsage`:
`hasAppOps = mPackageName.size() && checkAudioOp(OP_PLAY_AUDIO,…)==MODE_ALLOWED`,
and `mPackageName = attributionSource.packageName` (client-supplied — **not** from
`getPackagesForUid`, so the wart shim can't inject it). Our empty/stub
`AttributionSourceState` → empty package → muted. **Fix**: send a valid
`attributionSource` (uid = `geteuid()`, a `packageName`). For the probe (root,
uid 0) AudioFlinger takes the `isServiceUid && empty-packages → not muting` path;
for real app uids the non-empty package + the shim's permissive AppOps
(`MODE_ALLOWED`) pass the `OP_PLAY_AUDIO` check.

This is the **only** reason the rsbinder recursive Box-array patch is needed —
`AttributionSourceState.next[]` is a self-referential parcelable, which rsbinder
generates as `Vec<Box<Self>>` but lacks `SerializeArray`/`DeserializeArray for
Box<T>`. The 2 empty blanket impls (mirroring the `Strong<T>` impls) fix it. **Not**
needed for `createTrack` itself (that was the sessionId bug — a forward-decl stub
`AttributionSourceState` is accepted there).

## Capture (`open_input` → `createRecord` → `IAudioRecord`) — DONE, device-verified
The symmetric input half. Differs from output: the request is an `AudioConfigBase`
(no offloadInfo); the **response carries `cblk` + `buffers` as separate
`SharedFileRegion`s** (no `getCblk` round-trip — mmap both, or `buffers = cblk +
CBLK_SIZE` when the server omits it); start is `IAudioRecord.start(syncEvent=0,
triggerSession=0)`. The mic input profile is **16-bit PCM** — F32 is rejected by
`getInputForAttr` with `-38 (INVALID_OPERATION)` + a *suggested* config — so capture
requests `INT_16_BIT` and `read()` converts i16→f32 (output stays f32). `Track` is now
an `Endpoint` enum (`IAudioTrack | IAudioRecord`) with per-region mmaps. Verified:
129k samples/3s @ 48k mono, live ambient peak. (`ACDB-LOADER -19` = benign HAL
calibration noise.)

## Tooling (kept)
- `wart-host --probe-audioclient [secs] [hz] [vol]` — plays a tone via the backend.
- `wart-host --probe-audioclient-capture [secs]` — opens the mic via `createRecord`,
  reads PCM, reports frame count + peak (proves live capture).
- `wart-host --probe-audioclient-matrix` — request-variant matrix + serialized-request
  hexdump (the diagnostic that isolated the sessionId byte).
- `external/wart-audioclient-ref` (a-03 AOSP tree) — C++ reference using the device's
  libaudioclient: real `AudioTrack`, `createTrack(our values)`, `offsetof` dump, and
  byte-level hexdump for diffing. Build: `m wart-audioclient-ref` (dies in kati — OK)
  then `ninja -f out/combined-aosp_arm64.ninja <intermediate>`; source-only edits use
  ninja-direct (no `m`).

## Remaining / follow-ups
- rsbinder recursive patch is finalized as the `external/rsbinder` submodule
  (`harryzz/rsbinder`, branch `wart-recursive` = `5e999e04a` + the 2-line patch);
  `wart-host` `[patch]`es the hiking90 git source to it. ✅
- Capture (`createRecord`/`AudioRecord`) ✅.
- Transport + clock: `pause`/`flush` (`IAudioTrack`) + `get_timestamp` ✅ — note
  `IAudioTrack.getTimestamp` answers only for offload/direct tracks (normal mixed
  tracks → `INVALID_OPERATION -38`), so position comes from the cblk `mServer` +
  CLOCK_MONOTONIC (device-verified: advances at the sample rate).
- `out_write underrun` warnings = client pacing at start/end; couple the write pump to
  a steady cadence (the task-75 `min_frame_delay` lesson).
- Remaining per-track: `set_volume` (cblk `mVolumeLR` or `applyVolumeShaper`),
  `set_output_device`; track-invalidation restore (`CBLK_INVALID`/`DISABLED`).
- Wire the backend into the real host audio output path (replace the AAudioService path).
- Package/uid should come from the calling guest's identity, not the hardcoded
  `"android"`/`geteuid()` default in `attribution_source()`.
