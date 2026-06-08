//! `audioclient` — a Rust client for Android's native AudioFlinger
//! (`libaudioclient`), without the JVM and without AAudioService.
//!
//! It talks to AudioFlinger directly over binder
//! (`IAudioFlingerService.createTrack`/`createRecord` → `IAudioTrack`/`IAudioRecord`)
//! and drives the shared `audio_track_cblk_t` ring buffer in pure Rust. This is the
//! same wire path native `android::AudioTrack`/`AudioRecord` use — one level below the
//! Java `android.media.AudioTrack` (which needs ART) and beside AAudio's `AAudioStream`
//! (which goes through the AAudioService control plane that is unreliable under `--no-art`).
//!
//! Plan of record + rationale: wart `tasks/98-wart-audio-audioflinger-backend.md`.
//!
//! ## Reuse constraints (this is a platform-ABI crate, not a stable-API one)
//! - **Android-native only** — links rsbinder; off-android every call is a no-op.
//! - **Version-pinned ABI** — the AAudio AIDL transaction layout and the private
//!   `audio_track_cblk_t` struct are tied to the Android version. Valid across the API
//!   range the vendored AIDLs/struct match (target: API 33–35 / Android 13–15).
//! - **Privileged context** — AudioFlinger permission checks require a system uid /
//!   sepolicy domain the caller must already have.
//!
//! ## Status: SCAFFOLD
//! The public API + binder/codegen wiring are in place; the AudioFlinger client and the
//! `audio_track_cblk_t` proxy are not yet implemented (see the `TODO(task 98)` markers and
//! the migration sequence in the task doc). On Android the calls currently warn + no-op.

/// Output stream intent. The consumer maps its own notion of stream class
/// (media / voice-call / notification / alarm / game) to the Android audio
/// attributes here; this crate stays policy-free.
#[derive(Clone, Copy, Debug)]
pub struct OutputConfig {
    pub sample_rate: u32,
    pub channels: u32,
    /// `AUDIO_USAGE_*` (e.g. MEDIA=1, VOICE_COMMUNICATION=2).
    pub usage: i32,
    /// `AUDIO_CONTENT_TYPE_*` (e.g. MUSIC=2, SPEECH=1).
    pub content_type: i32,
}

/// Capture stream intent.
#[derive(Clone, Copy, Debug)]
pub struct InputConfig {
    pub sample_rate: u32,
    pub channels: u32,
    /// `AUDIO_SOURCE_*` (e.g. MIC=1, VOICE_COMMUNICATION=7).
    pub source: i32,
}

/// Opaque stream handle. `0` = invalid / failed.
pub type Handle = u32;

// ── Public API (Android = real backend; elsewhere = no-op) ───────────────────

/// Open an output (playback) stream. Returns a [`Handle`] (`0` on failure).
pub fn open_output(cfg: OutputConfig) -> Handle { imp::open_output(cfg) }
/// Push interleaved f32 PCM into the output ring; returns frames accepted.
pub fn write(track: Handle, pcm: &[f32]) -> usize { imp::write(track, pcm) }
/// Open an input (capture) stream. Returns a [`Handle`] (`0` on failure).
pub fn open_input(cfg: InputConfig) -> Handle { imp::open_input(cfg) }
/// Pull up to `max_frames` of interleaved f32 PCM from the capture ring.
pub fn read(capture: Handle, max_frames: u32) -> Vec<f32> { imp::read(capture, max_frames) }
/// Start the stream (begin pulling/pushing on the AudioFlinger MixerThread).
pub fn start(track: Handle) -> bool { imp::start(track) }
/// Stop the stream.
pub fn stop(track: Handle) -> bool { imp::stop(track) }
/// Close + release the stream.
pub fn close(track: Handle) { imp::close(track) }
/// Per-track gain `0.0..=1.0` (`AudioTrack::setVolume`-equivalent) — bypasses the
/// policy stream-volume index.
pub fn set_volume(track: Handle, level: f32) { imp::set_volume(track, level) }
/// Re-route this track to a specific output device port at runtime
/// (`AudioTrack::setOutputDevice`-equivalent). `0` = unset (policy default).
pub fn set_output_device(track: Handle, port_id: i32) -> bool { imp::set_output_device(track, port_id) }
/// `(framePosition, nanoTime)` from `IAudioTrack.getTimestamp`, for the clock model.
pub fn get_timestamp(track: Handle) -> Option<(i64, i64)> { imp::get_timestamp(track) }

// ── Android backend ──────────────────────────────────────────────────────────
#[cfg(target_os = "android")]
mod imp {
    use super::*;

    /// Generated AIDL bindings for the `IAudioFlingerService` closure
    /// (`IAudioTrack`/`IAudioRecord`/`CreateTrack*`/`SharedFileRegion`).
    #[allow(warnings, clippy::all, dead_code)]
    pub(crate) mod aidl {
        include!(concat!(env!("OUT_DIR"), "/audioflinger_bindings.rs"));
    }

    // TODO(task 98): the AudioFlinger client + audio_track_cblk_t ClientProxy.
    //   1. control plane (rsbinder): get_interface::<IAudioFlingerService>("media.audio_flinger")
    //      → createTrack(CreateTrackRequest{attributes,…}) → CreateTrackResponse{ IAudioTrack, frameCount, … }
    //      → IAudioTrack.getCblk() (SharedFileRegion) → mmap.
    //   2. data plane: port `audio_track_cblk_t` + `AudioTrackClientProxy` obtain/releaseBuffer
    //      from vendor/aosp-frameworks-av/{include/private/media/AudioTrackShared.h,
    //      media/libaudioclient/AudioTrackShared.cpp} — non-blocking writer, skip the futex.
    //   3. IAudioTrack.start()/stop()/getTimestamp(); IAudioRecord mirror for capture.
    // Until then: warn + no-op so the crate compiles and dependents wire against the API.

    pub fn open_output(cfg: OutputConfig) -> Handle {
        log::warn!("audioclient::open_output {cfg:?} — TODO(task 98): AudioFlinger client not yet implemented");
        0
    }
    pub fn write(_track: Handle, _pcm: &[f32]) -> usize { 0 }
    pub fn open_input(cfg: InputConfig) -> Handle {
        log::warn!("audioclient::open_input {cfg:?} — TODO(task 98): AudioFlinger client not yet implemented");
        0
    }
    pub fn read(_capture: Handle, _max_frames: u32) -> Vec<f32> { Vec::new() }
    pub fn start(_track: Handle) -> bool { false }
    pub fn stop(_track: Handle) -> bool { false }
    pub fn close(_track: Handle) {}
    pub fn set_volume(_track: Handle, _level: f32) {}
    pub fn set_output_device(_track: Handle, _port_id: i32) -> bool { false }
    pub fn get_timestamp(_track: Handle) -> Option<(i64, i64)> { None }
}

// ── Off-Android: no-op stubs so dependents compile cross-platform ────────────
#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;
    pub fn open_output(_cfg: OutputConfig) -> Handle { 0 }
    pub fn write(_track: Handle, _pcm: &[f32]) -> usize { 0 }
    pub fn open_input(_cfg: InputConfig) -> Handle { 0 }
    pub fn read(_capture: Handle, _max_frames: u32) -> Vec<f32> { Vec::new() }
    pub fn start(_track: Handle) -> bool { false }
    pub fn stop(_track: Handle) -> bool { false }
    pub fn close(_track: Handle) {}
    pub fn set_volume(_track: Handle, _level: f32) {}
    pub fn set_output_device(_track: Handle, _port_id: i32) -> bool { false }
    pub fn get_timestamp(_track: Handle) -> Option<(i64, i64)> { None }
}
