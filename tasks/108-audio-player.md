# Task 108 — Feature-rich audio player + audio-stack gaps

Design: `docs/audio-player-design.md` (read it first — the architecture,
W3C-alignment, and the transcode-vs-tunnel decision live there).

**Goal.** A real audio player on wandr, built the WASI-clean way: a pure-Rust
guest over a **layered, capability-negotiated** audio stack — `wasi:audio` PCM
as the mandatory portable floor; HW codec and HW effects as *optional*
capabilities the guest queries and opts into, with trivial guest-Rust
fallback. Fills the current gaps (no decode anywhere; no transport clock; no
media-session) without accreting Android-isms into the contracts.

**Core principle (from the design):** mechanism in the host, policy in the
guest — the SRTP HW-offload pattern (`[[project_wandr_crypto_srtp_offload]]`)
generalized to codecs and DSP. The guest decides HW-or-custom per stream; the
host advertises what HW it has; absence → guest does it itself.

## Milestones

### M1 — guest-decode floor + `position` (the spike)
- `apps/user/wandr.audio.player`: pure-Rust guest, `wasi:canvas` UI +
  `wasi:input-handlers`.
- Symphonia decode of a local FLAC + MP3 → `rubato` resample to 48 k →
  `wasi:audio.playback.write` (the shipped backpressure model).
- **Contract add:** promote `playback.position() -> u64` (frames played) in
  `proposals/wasi-audio/wit/audio.wit` + host impl
  (`runtime/wandr-host/src/wasi_audio_impl.rs`) — the player is the promoting
  consumer. Optionally `drain()`.
- UI: play/pause, a seekbar driven by `position`, track tags (Symphonia
  metadata), album art via `graphics.decode-image` (no new lib).
- **Exit:** plays a local file on the desktop host *and* the device; seekbar
  tracks; visual verify with the user (`[[feedback_visual_verification]]`).
- Records: guest size, decode CPU %, first-audio latency.

**✅ M1 DONE + USER-VERIFIED ON DEVICE (2026-06-14).** Symphonia FLAC decode →
`wasi:audio` audible; `playback.position` promoted (host impl, tracks wall ±40ms);
`apps/user/wandr.audio.player` is a `wasi:canvas` reactor — vinyl art placeholder,
title/format from tags, real waveform overview (guest-side from PCM), seekbar
driven by `position`, play/pause + tap-seek via touch (pointer-handler +
inputflinger). 1.54 MB guest. Run: `wandr-arbiter launch wandr.audio.player`
(needs host-108 for `position`). Notes: desktop winit window unusable in the WSL
sandbox (wayland reset) → verified via device screencap. Album-art decode
(`graphics.decode-image`) deferred (the test FLAC has no embedded art).

**✅ wasi:audio `flush()` + `drain()` ADDED + device-verified gapless seek
(2026-06-14).** The close+reopen seek finding is resolved: `flush` (drop
buffered now) + `drain` (play-out-then-stop) on the playback resource; host
impl = `IAudioTrack.flush`/`stop` (flush PAUSES first — flush mid-play wedges
the track), with `position` kept continuous (host subtracts the dropped
backlog from the `written` counter). Player seek rewired: `flush → re-anchor
the device clock → prime the ring → resume` (anchor_dev/anchor_track model).
Gotcha: `AudioTrack.flush()` is only valid stopped/paused — first attempt
flushed mid-play and killed audio.

### M2 — `wasi:media-session` (the native-feel gap)
- New arbiter-owned package `wasi:media-session@0.0.1` (sibling of
  `wandr:audio-focus`/`alarm`/`notify`), tracking the **W3C Media Session API**
  shape: guest publishes now-playing metadata + playback state + position;
  arbiter renders the lockscreen/notification transport and routes
  headset/BT **media-button** events (play/pause/next/prev/seek) to the
  guest's `media-session-handler` export (probed `.ok()` per instance).
- Wire focus/route/volume through existing `wandr:audio-focus` (no new work).
- **Exit:** lockscreen/notification transport controls the player; a headset
  button toggles play/pause; now-playing shows title/artist/art.

### M3 — guest-side richness (no new WIT)
- Gapless (decode-ahead + encoder delay/padding trim), crossfade (mix two
  decoders), ReplayGain (read RG tags, apply gain pre-write).
- Custom EQ (biquad) + spectrum/waveform viz (`rustfft` → `wasi:canvas`).
- Opus via `external/opus-rs`; playlist/queue (guest state).
- Network streaming via the `wasi:tls` reqwest-shim / `wasi:http`.

### M4 — optional HW offload lanes (ONLY behind a measured need)
- `wasi:audio-codec@0.0.1` (WebCodecs-shaped): `probe` + HW decode/encode,
  **transcode** (PCM back) and **tunnel** (decode → sink) modes; reuse the
  `wasi:video-decoder` error enum. Host backend = MediaCodec/DSP.
- `wasi:audio-effects@0.0.1`: attach Android `AudioEffect`s (EQ/BassBoost/
  Virtualizer/Loudness/Reverb) to the stream, portable params.
- Player switches **transcode/guest while foreground** (visualizer/DSP) and
  **tunnel when backgrounded/screen-off** (battery) — "exposes both, picks
  per-situation."
- **Trigger:** M1's decode-CPU/battery numbers show offload pays for long
  background playback. Contracts may land before the impl so the guest is
  written against the final shape.

## Discipline (binding — from the design §Discipline)
1. PCM floor mandatory + sufficient; HW always optional.
2. Every HW capability has a guest fallback (portability never breaks).
3. Capability query + typed `unsupported-codec`/`no-hw-codec` errors.
4. No non-portable knobs / no Android-isms in the WIT — the contracts must
   suit a PipeWire/CoreAudio/WASAPI host too (host portability is a *note*,
   not an M-target, but the WIT must not foreclose it).

## Known risks / notes
- `[[feedback_no_hardcoding]]`: derive rate/buffer sizes from the device
  config and `buffered-frames`, not magic numbers.
- AAC/MP3 feature flags in Symphonia (licensing stays guest-side by design).
- Don't widen `wasi:audio` beyond the `position` add in M1 — HW lanes are
  *separate* packages (`[[feedback_clean_library_usage]]`).
- Visualization + custom DSP are incompatible with tunnel mode by physics —
  expected, not a bug.

## Verification
- M1: local file audible on desktop **and** device, seekbar tracks
  `position`, tags + art render; user visual/audio confirm.
- M2: lockscreen transport + headset button drive the player; now-playing
  correct.
- No host changes beyond the named `wasi:audio` `position` add in M1 and the
  new optional packages in M2/M4 — anything else is a finding to report.
