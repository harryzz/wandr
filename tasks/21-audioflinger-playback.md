# Task 21 — AudioFlinger playback

> **Status: 🟡 scoped, not started — significantly larger than 19/20.** Wire WASM Compose apps to play sound through Android's `media.audio_flinger` daemon (`android.media.IAudioFlinger` AIDL). Confirmed service present on Pixel 2 XL. **This is a much bigger task than 19/20** — AudioFlinger is a native C++ daemon (not a HAL), its AIDL surface is large (60+ methods, dozens of parcelables), and the actual playback path goes through **shared memory + ParcelFileDescriptor**, not pure binder calls. Worth scoping carefully before committing to the rsbinder route vs. alternatives.

## Goal

Let WASM apps play PCM audio (and ideally compressed-PCM via the framework's mixer). Minimum viable: a guest can call `audio.create-track(format)` to get a handle, then `audio.write(handle, &[f32])` to queue samples that the speaker plays.

Reference: `~/wart/post-art-roadmap.md` §3. AudioFlinger is the dominant native-daemon-not-Java-framework case from §6.5 ("Keep — already native daemons, talk to them via binder") — so it survives the ART replacement and is the right long-term target.

---

## Architecture decision needed before starting

AudioFlinger is in `platform/frameworks/av` (yet another AOSP repo to submodule). Its AIDL is `frameworks/av/media/libaudioclient/aidl/android/media/`. Key interfaces:

- **`IAudioFlinger`** (the daemon entry point): `createTrack(input) → CreateTrackResponse` (returns IAudioTrack handle + a SharedFileRegion for the sample memory + a ParcelFileDescriptor for an event pipe)
- **`IAudioTrack`** (per-track binder): `start()`, `stop()`, `pause()`, `flush()`, etc. The *actual sample writing* is **NOT** a binder call — samples are written to the shared memory ring buffer and signaled via the fd.

Three implementation paths, ranked by effort:

### Path A: NDK AAudio (no binder at all)
- Skip AudioFlinger AIDL entirely. Use the NDK `AAudio` C API via thin Rust FFI.
- AAudio is a stable C API since Android 8 (`<aaudio/AAudio.h>` in NDK r19+). Handles all the AudioFlinger / shared-memory plumbing internally.
- Pros: ~200 lines total. No binder, no AIDL, no submodule. Works on every Android 8+ device including non-rooted. Lower latency than AudioFlinger directly.
- Cons: not the binder pattern. NDK headers + cc-rs FFI shim. Doesn't validate rsbinder for media services.

### Path B: rsbinder to IAudioFlinger.createTrack + shm via rust-ashmem
- True binder path. Call `createTrack`, parse the returned `CreateTrackResponse`, mmap the `SharedFileRegion`, write PCM frames to the ring buffer, signal via the event fd.
- Pros: matches the post-ART end-state architecture (no NDK dep).
- Cons: rsbinder needs `SharedFileRegion` + `ParcelFileDescriptor` parcelable support — `ParcelFileDescriptor` IS provided (`rsbinder::file_descriptor::ParcelFileDescriptor`), `SharedFileRegion` is less certain. Ring-buffer protocol + fd signaling is intricate. Days of work, not hours.

### Path C: Compose's existing media (no native work)
- Defer until apps actually need audio. The current PoC test apps don't play sound.

**Recommendation: defer choice between A and B until first real audio need surfaces in a test app.** Document both paths here; pick when execution starts.

---

## WIT design (path-agnostic — the guest doesn't care which path the host uses)

```wit
/// Minimum audio: PCM playback through one or more tracks. Mirrors the
/// abstraction of AAudio / iOS AVAudioPlayerNode / Web Audio AudioBuffer
/// — guest creates a track, writes interleaved float frames, the host
/// pipes them to the speaker. No 3D positioning, no effects, no input.
interface audio {
    enum format {
        pcm-f32,   // 32-bit float per sample, interleaved (default)
        pcm-i16,   // 16-bit signed int per sample, interleaved
    }
    enum channel-layout {
        mono,
        stereo,
    }
    record track-config {
        sample-rate-hz: u32,         // 8000..192000
        format:         format,
        channels:       channel-layout,
    }
    /// Allocates an audio track. Returns 0 on failure (unsupported format,
    /// no audio device, permission denied).
    create-track:   func(config: track-config) -> u32;
    /// Write `frames` to track's queue. `samples` length must be
    /// (frames * channels). Returns frames actually accepted (may be less
    /// than requested if the ring buffer is full — guest should retry).
    write-pcm-f32:  func(handle: u32, samples: list<f32>) -> u32;
    write-pcm-i16:  func(handle: u32, samples: list<s16>) -> u32;
    /// Start / pause playback.
    start:          func(handle: u32);
    pause:          func(handle: u32);
    /// Drain queued samples and release the track.
    close:          func(handle: u32);
    /// Number of frames currently queued but not yet played (latency hint).
    pending-frames: func(handle: u32) -> u32;
}
```

Deliberately minimal — no audio focus, no spatial audio, no codecs, no MIDI, no streaming-from-URL. Those are separate WIT interfaces if/when needed.

---

## Steps (Path A — AAudio NDK, the recommended starting path)

### 1. Add `aaudio` link

In `wart-host/build.rs` Android-only block, append:

```rust
println!("cargo:rustc-link-lib=aaudio");
```

`libaaudio.so` is in the NDK sysroot at `sysroot/usr/lib/aarch64-linux-android/<api>/libaaudio.so` (present from API 26+).

### 2. FFI shim

Either hand-write a minimal extern block (~80 lines covers builder + stream + read/write + close) or use `bindgen` against `<aaudio/AAudio.h>`. Hand-written is cleaner for this small surface and avoids dragging bindgen into build-deps.

```rust
// wart-host/src/audio_ffi.rs
#[repr(C)] pub struct AAudioStream { _opaque: [u8; 0] }
#[repr(C)] pub struct AAudioStreamBuilder { _opaque: [u8; 0] }

extern "C" {
    pub fn AAudio_createStreamBuilder(builder: *mut *mut AAudioStreamBuilder) -> i32;
    pub fn AAudioStreamBuilder_setSampleRate(builder: *mut AAudioStreamBuilder, rate: i32);
    pub fn AAudioStreamBuilder_setFormat(builder: *mut AAudioStreamBuilder, format: i32);
    pub fn AAudioStreamBuilder_setChannelCount(builder: *mut AAudioStreamBuilder, count: i32);
    pub fn AAudioStreamBuilder_openStream(builder: *mut AAudioStreamBuilder, stream: *mut *mut AAudioStream) -> i32;
    pub fn AAudioStream_write(stream: *mut AAudioStream, buffer: *const std::ffi::c_void, num_frames: i32, timeout_ns: i64) -> i32;
    pub fn AAudioStream_requestStart(stream: *mut AAudioStream) -> i32;
    pub fn AAudioStream_requestPause(stream: *mut AAudioStream) -> i32;
    pub fn AAudioStream_close(stream: *mut AAudioStream) -> i32;
    // ... ~12 more
}
```

### 3. `wart-host/src/audio_impl.rs`

- `HashMap<u32, *mut AAudioStream>` keyed by handle (assigned monotonically), guarded by `Mutex`
- `create_track(config)` → AAudioStreamBuilder + setSampleRate/Format/ChannelCount + openStream → store handle
- `write_pcm_f32(handle, samples)` → look up stream, `AAudioStream_write(stream, ptr, frames, 0)` (non-blocking)
- `close(handle)` → `AAudioStream_close` + remove from map

### 4. WIT + Kotlin bindings + lib.rs wiring

Same hand-edit pattern as previous tasks. `track-config` is 3-field flat record (3 i32s); `write-pcm-f32` takes `list<f32>` (pointer-based marshalling — list types need allocator-based writes, copy pattern from canvas TextBlob handling).

### 5. Verification on device

- Build chain + deploy
- Main.kt smoke: synthesize a 440 Hz sine wave at 48 kHz, write 1 second of samples, hear a beep
  ```kotlin
  val h = Audio.Import.createTrack(Audio.TrackConfig(48000u, Audio.Format.PCM_F32, Audio.ChannelLayout.MONO))
  val samples = FloatArray(48000) { i -> sin(2.0 * PI * 440.0 * i / 48000.0).toFloat() * 0.3f }
  Audio.Import.writePcmF32(h, samples.toList())
  Audio.Import.start(h)
  ```
- Phone speaker should play a clear 440 Hz tone for 1 second.

---

## Steps (Path B — rsbinder to AudioFlinger, deferred alternative)

Outline only — fill in if Path A proves insufficient (e.g., need to access AudioPolicyManager features, BT audio routing, or specific AudioFlinger-only metadata).

1. Add submodule `vendor/aosp-frameworks-av` pinned to `android-11.0.0_r48`, sparse-checkout `media/libaudioclient/aidl/`.
2. rsbinder-aidl `IAudioFlinger.aidl` — large parcelable surface (CreateTrackInput, CreateTrackOutput, AudioConfig, AttributionSourceState, …).
3. Look up service `media.audio_flinger` via rsbinder hub.
4. Call `createTrack(input)`. Parse `CreateTrackOutput` for the IAudioTrack binder + SharedFileRegion + event ParcelFileDescriptor.
5. mmap the SharedFileRegion (POSIX `mmap` via `nix` crate or directly).
6. Implement the audio ring-buffer protocol: write PCM frames at the current write index, update the shared header counter, optionally write to the event fd to wake up AudioFlinger.
7. `IAudioTrack.start()` / `stop()` / `pause()` over binder for transport control.

Estimated: 3-5 days for a basic mono playback. Significantly more for AudioPolicy interaction (audio focus, routing).

---

## Known issues / risks

1. **VIBRATE-style permission scope.** Audio playback typically requires no permission (any app can call `MediaPlayer` / AAudio). Verify on first test that our APK doesn't trip any AVC denial.

2. **AAudio vs OpenSL ES.** AAudio is the modern API (API 26+). OpenSL ES is legacy. Don't bother with OpenSL ES.

3. **Latency expectations.** AAudio in `EXCLUSIVE` mode achieves ~10 ms latency; `SHARED` mode is ~30-50 ms. Pixel 2 XL supports both. Default to SHARED in WIT; expose EXCLUSIVE later if a real low-latency app needs it.

4. **No audio input** in this WIT. Recording is a separate concern (permission gates, focus, etc).

5. **Path A doesn't validate the post-ART binder story.** If AudioFlinger is one of the native daemons we keep, eventually we'll need Path B to prove the rsbinder pattern scales to media services. Path A is the quick-deploy answer; Path B is the long-term-correct answer.

---

## Out of scope

- Recording / `AAudioStreamBuilder_setDirection(INPUT)`.
- Audio focus / ducking (`AudioManager.requestAudioFocus()`).
- Audio routing (USB headset, Bluetooth A2DP routing).
- Codec decoding (MP3, AAC, FLAC) — Compose can do that in pure WASM if needed; or extend WIT later.
- Spatial audio / surround / Atmos.
- MIDI.
- The entire `IAudioPolicyService` surface.
