---
name: project_audio_player_engine_consolidation
description: "FUTURE plan — move wandr.audio.player onto wandr-media-engine's audio core (dedup Symphonia). Feasible behind a guest PCM-tap hook + an engine track-queue for gapless; crossfade deferred. NO WIT change (all guest-side, before the wasi:audio write)."
metadata:
  node_type: memory
  type: project
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-05T13:57:27.423Z
---

# audio.player → wandr-media-engine consolidation (FUTURE, not built)

**Idea (user, 2026-08-05):** `wandr.audio.player` (task 108) has its OWN
Symphonia+local-file audio; `wandr-media-engine` has a headless audio lane that
navidrome already uses. Move audio.player onto the engine to kill the duplicated
Symphonia wrap and inherit the engine's async device-audio fix
([[reference_media_engine_device_audio_fix]]) + wider codec set.

## ‼️ NOT a WIT change
Everything here is guest-side Rust inside the `wandr-media-engine` crate +
audio.player. Gapless/crossfade/EQ/viz all happen BEFORE the `wasi:audio` write
(decode → PCM → trim/mix/DSP → PCM write); the device only ever sees a PCM
stream. `wasi:audio` is already wired and stays byte-identical. The two engine
additions below are internal Rust APIs, not contract surface. (Only WIT-adjacent
thing in this area = `wasi:media-session` for lockscreen transport — separate,
not needed for the move.)

## What moves cleanly vs. the blocker
Engine's audio lane (`open_audio_sync` → `fill_queues`/`decode_audio`/
`pump_stream`, `AudioDec`, `LinearResampler`, A/V clock, seek) already provides:
- **Local-file source** — NOT a blocker: `httprange.rs` has a `local:
  Option<std::fs::File>` branch (`File::open`), so a path opens the same as a URL.
- **Decode** — superset: `AudioDec` = aac/opus(ropus)/flac/vorbis/ac3/mp3 vs
  audio.player's mp3/aac only.
- **Resample → wasi:audio + clock + seek + async device-audio fix** — all there;
  audio.player would GAIN the device-audio fix.

**The one blocker:** audio.player's three signature features live in its guest
decode loop *because it owns the PCM between decode and device-write*, and the
engine has NO guest hook there (`decode_audio → pump_stream` writes straight to
the device):
- **EQ** (biquad, per-stream filter state)
- **FFT visualizer** (taps the PCM history ring)
- **Gapless / crossfade**

## Framework survey — gapless ≠ crossfade (source-grounded 2026-08-05)
Every established player treats these as TWO problems at TWO levels; audio.player
already mirrors it (`xfade_secs==0` → gapless; `>0` → two-head `Xfade`).

**Gapless = ONE output + a source QUEUE, seam-trimmed. No mixing.** Pre-roll the
next track before EOS so the device ring never underruns; the only real work is
trimming encoder delay/padding from container metadata.
- Symphonia: `enable_gapless:true` reads LAME/Xing (MP3), iTunSMPB (AAC/ALAC),
  Opus pre-skip; auto-adjusts packet timestamps+durations. audio.player ALREADY
  sets this (lib.rs ~line 1008).
- ExoPlayer/Media3 (Android flagship): extractors parse Xing/ID3/udta/MP4
  edit-lists; renderer trims → one continuous AudioTrack stream.
- GStreamer (wandr's desktop backend): `about-to-finish` → app sets next URI;
  urisourcebin pre-buffers; single pipeline flows on.
- rodio: `Sink` = a queue of sources played back-to-back into one output.
→ shape: **queue + pre-roll + metadata seam-trim → one sink.**

**Crossfade = TWO concurrent decoders + a gain-ramp MIXER.**
- rodio `crossfade.rs` (verified): both sources run, `mix()`, **linear** curve.
- MPD `CrossFade.cxx` (verified): overlap `chunks = duration/chunk_duration`;
  MixRamp = smarter *timing* via per-track loudness dB tags, not a new curve.
- GStreamer `audiomixer` + volume ramps (0→1, 1→0). Web Audio: two source+gain,
  **equal-power** (sin/cos) recommended.
- audio.player: second parallel `Loaded` head, **equal-power** (correct — linear
  dips perceived loudness ~3–6 dB mid-blend for uncorrelated tracks; equal-power
  keeps g²+(1−g)²=1).
- ExoPlayer ships gapless but **NO native crossfade** (app-side, two players).
→ shape: **two live decode heads + gain ramp.**

## Design decision for the engine (the takeaway)
- **Gapless BELONGS in the engine — cheap, mainstream.** It's a source-queue over
  the single `StreamPlayer`: `enqueue_next(url)` + pre-roll before EOS + lean on
  Symphonia `enable_gapless` for the trim + feed both into the one wasi:audio ring
  without a flush. Exactly the ExoPlayer/GStreamer/rodio-Sink pattern.
  navidrome/jellyfin get gapless queues for free.
- **Crossfade is the real cost; defer it (industry-normal).** Needs a SECOND
  concurrent decode head + mixer stage — the one thing the single-stream model
  lacks. Android's flagship omits it natively. Later option: an engine
  "queue-with-N-sec-crossfade" mode (two heads + equal-power), OR audio.player
  keeps its existing guest-side `Xfade`.

## Concrete engine API to add (when built)
1. `pcm_tap`: a guest-installable hook (closure/trait) that sees each decoded
   stereo f32 block BEFORE resample→write. Mechanism, not policy — navidrome/
   jellyfin install none. audio.player's EQ + visualizer run here.
2. `enqueue_next(url, dur_us, title)` + pre-roll/seam logic on the single
   `StreamPlayer` for gapless (no flush at the boundary; segment/clock continuity
   like audio.player's `Seg`/`segments` map).
3. (Deferred) two-head crossfade mode.

Then repoint audio.player's source+decode at `open_audio_sync`, dropping its
duplicate Symphonia `FormatReader`/`Decoder`. Library scan / tags / cover cache /
last.fm stay guest-side (app policy, not engine).

## Earlier investigation context
The reverse direction is already recorded: [[project_navidrome_slint_migration]]
notes navidrome copied audio.player's Slint UI but used the engine's NETWORK
audio, deliberately keeping audio.player's local-file+DSP path separate. This
note is the plan to converge the decode/source/device layers while keeping the
DSP as guest policy.

Related: [[project_audio_player]] (task-108 layered design) ·
[[reference_media_engine_device_audio_fix]] ·
[[reference_gstreamer_desktop_backend_spike]] · [[feedback_no_hardcoding]].
