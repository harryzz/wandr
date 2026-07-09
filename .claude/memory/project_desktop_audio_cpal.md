---
name: project_desktop_audio_cpal
description: Desktop (Linux/WSLg/Win/mac) wasi:audio backend = cpal; WSLg routing + BufferSize::Fixed + bg-tick pump gotchas
metadata: 
  node_type: memory
  type: project
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

Desktop backend for `wasi:audio/pcm` (task 108 / desktop A/V roadmap) = **cpal**,
in `runtime/wandr-host/src/audio_desktop.rs` — the cross-platform peer of the
Android AudioFlinger-direct backend. Guest-driven ring (F32 interleaved) bridged
to cpal's callback: `write` fills a 0.5 s ring, the output callback drains it
(silence on underrun), `pending_frames` = ring depth so `position = written −
buffered` (A/V clock) matches device. Wired into the desktop `#[cfg(not(android))]`
arms of `audio_impl.rs`. Device-verified audible on WSLg (audio.player, all 3
albums). See [[project_audio_player]], [[project_audioflinger_backend]].

**4 load-bearing gotchas (all cost real time this session):**

1. **cpal needs the `pulseaudio` feature on WSLg — NOT the default ALSA host, and
   NOT `pipewire`.** `cpal = { version="0.18", features=["pulseaudio"] }`. On Linux
   `default_host()` tries pipewire → pulseaudio → alsa, each only if its feature is
   on. WSLg audio that reaches Windows goes through the PulseAudio server at
   `unix:/mnt/wslg/PulseServer` (RDP-bridged; `/mnt/wslg/PulseAudioRDPSink`) — the
   same libpulse path Linux-Chrome uses. The box's **local PipeWire exposes only a
   "Dummy Output"** (null sink; `wpctl status`), so enabling `pipewire` OR the
   default ALSA host (ALSA `default` → local PipeWire) plays to /dev/null = silent.
   cpal's `pulseaudio` backend is the **pure-Rust `pulseaudio` crate** (no libpulse,
   no system package, no `~/.asoundrc`) and connects straight to `$PULSE_SERVER`.
   Diagnose: `wpctl status` (only Dummy Output = trap); confirm target works with
   `PULSE_SERVER=unix:/mnt/wslg/PulseServer ffmpeg -f lavfi -i sine=... -f pulse x`.

2. **`BufferSize::Default` → dropouts on the WSLg RDP sink; use `BufferSize::Fixed`
   (PulseAudio host only).** Source-verified (cpal-0.18.1 `make_playback_buffer_attr`):
   `Default` passes an empty `pa_buffer_attr` → server picks buffering → huge,
   infrequent callbacks (saw 8916-sample bursts ~2 Hz = starved). `Fixed(n)` pins
   `minimum_request_length` (one period) + `target_length` (two = latency), so the
   server asks for a regular small chunk (~40 ms → 3862 samples ~25 Hz = real-time,
   zero underrun). Gate on `host.id().name().eq_ignore_ascii_case("pulseaudio")` —
   **WASAPI/CoreAudio can reject `Fixed`**, keep them on `Default`. Period derived
   from `sample_rate` (no hardcode): `PULSE_CALLBACK_MS=40`.

3. **The desktop winit loop must pump `wandr:background/background` bg-tick, or
   background-engine guests never start.** audio.player runs its ENTIRE engine
   (library scan, decode, feeding the ring, media-session) in bg-tick, not render.
   The device standalone loop pumps it; the desktop `App` (lib.rs) discarded
   `inst.bg_tick` and only called `render_frame` → empty UI, no audio. Fixed:
   store `bg_tick` on `App`, pump on the guest-authored cadence (clamp 16–1000 ms)
   before render. Benefits ANY bg-service guest on desktop.

4. **Desktop `/music` preopen** = `$HOME/Music` (or `WANDR_DESKTOP_MUSIC=<dir>`),
   mirroring the device's `/data/media/0/Music → /music`. audio.player scans
   `/music/<Album>/*` — files must be in **album subdirectories**, loose files are
   ignored (`scan_library` only descends `is_dir()` entries). Copied test albums
   from phone: stage under `/data/local/tmp` (adb pull can't `su`), one subdir per
   album.

Device output config on WSLg = 44100/I16; guest sends 48000/F32 → PulseAudio
resamples, fine. Roadmap next: nokhwa camera + ffmpeg/libvpx codecs (same
cross-platform pattern).
