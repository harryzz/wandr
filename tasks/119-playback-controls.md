# Task 119 — Part B: playback controls for wandr.jellyfin

> **Status: SCOPED, not started.** Adds player controls to the working
> DirectPlay client (Part A): pause/stop, FF/REW, mute, volume ±, subtitles,
> audio-track switch, + progress/resume. Guest-side (wandr.jellyfin); no WIT
> change expected (the host already exposes decode/reset/present + wasi:audio
> pause). Builds on the existing `Phase::Playing`, `key_handler`/`pointer_handler`,
> `draw_*`, `fetch_range`, `StreamPlayer`, and `dec.reset()`.

## Goal

Turn the play-only client into a usable player: transport (pause/stop/seek),
audio (mute/volume/track), and subtitles — with an on-screen control bar and
keyboard shortcuts, and Jellyfin resume/progress so it behaves like a real client.

## Shared foundation (build first — everything depends on it)

1. **Player command channel.** Input handlers run in `on_key`/`on_pointer`; the
   playback engine runs in `pump_stream` (on-frame) + `bg_tick`. Add a small
   command set applied to `StreamPlayer` (flags/fields, not a queue is fine since
   both run on the single store thread): `paused`, `muted`, `volume`, `seek_to:
   Option<i64>`, `pending_audio_track`, `subs_on`, `subs_track`. Input sets them;
   the engine consumes them.

2. **On-screen control bar** (new render branch for `Phase::Playing`, drawn over
   the video via `composite`/`draw_*`): play/pause, a **scrub bar** (position +
   buffered-ahead + total time), skip ±, volume slider + mute, a subtitles
   toggle, an audio-track menu, and back/stop. **Auto-hide** after ~3 s idle;
   reveal on pointer move / key. Pointer **hit-testing** for buttons + scrub drag.
   This is the biggest single chunk and the container for every control.

3. **The SEEK primitive** (`seek_to(target_us)`) — the hard foundation for
   FF/REW/scrub/resume. Steps:
   - **Byte offset for time**, per container: MP4 = keyframe sample at/before
     target via `stss`+`stts`+`stco/stsz` (video) + the audio sample at that time;
     MKV = `Cues` (SeekHead→Cues) → cluster byte offset (verify matroska-demuxer
     0.6 exposes `seek`; if not, reposition the `HttpRangeReader` to the cluster
     offset and re-scan). Transport is static + HTTP Range, so a seek = a **new
     fetch window** at the computed offset (drop the current buffer).
   - `dec.reset()` (flush reference frames) + feed a **keyframe** first.
   - Reposition demux cursors (`next_v`/`next_a` for MP4; the MKV reader).
   - **Re-anchor the A/V clock**: clear `video_q`/`audio_q`/`pending_pcm`, flush
     the audio ring, reset `first_pts_us`/`origin_ns` and the audio-master anchor
     (`audio_pts_known`/`audio_start_ns`/`audio_first_pts_us`) so `media_now`
     restarts at the seek target. **This clock re-init is the riskiest part.**

## The controls (tiered by dependency)

### Tier 1 — cheap, guest-side only (once the bar + channel exist)
- **Pause / resume** — `paused` flag: engine stops decode/present, and **pauses
  the wasi:audio track** (host `audio_desktop::pause`). Freeze `media_now`. On
  resume: unpause + **re-anchor** the audio clock (the device position stalled
  while wall time advanced → shift `audio_start_ns` by the pause duration, or
  re-derive). Gotcha: the re-anchor, or audio drifts after resume.
- **Mute / volume ±** — `muted: bool` + `volume: f32` (0..1, ~0.1 steps). Scale
  `pending_pcm` by `muted ? 0 : volume` **at the ring-write** (not in decode) so
  changes apply immediately to already-buffered PCM. Trivial f32 gain.
- **Stop** — `stop_requested` already exists; ensure full teardown (decoder,
  audio, demux) + report **Stopped** to Jellyfin (below) + back to `Phase::Browse`.

### Tier 2 — needs the seek primitive
- **FF / REW** — `seek_to(clock_us ± step)` (e.g. +30 s FF, −10 s REW; tune).
- **Scrub bar drag** — pointer drag on the bar → `seek_to(fraction * duration)`.
- **Resume + progress report to Jellyfin** — POST `/Sessions/Playing/Progress`
  periodically (position + paused/muted/volume/track state) and
  `/Sessions/Playing/Stopped` on stop; read `UserData.PlaybackPositionTicks` to
  offer **resume from last position** (uses `seek_to` at open). Makes watched
  status + cross-device resume work. Also `/Sessions/Playing` on start.

### Tier 3 — new pipelines
- **Subtitles on/off (+ track/language)** — two options, pick one:
  - **(a) Jellyfin VTT endpoint** *(simpler, recommended first)*:
    `/Videos/{id}/{mediaSourceId}/Subtitles/{streamIndex}/Stream.vtt` → parse
    WebVTT (cue timing + text) → timed text overlay via `draw_text`, bottom-center,
    against `media_now`. No in-container subtitle demux.
  - **(b) In-container demux** *(DirectPlay-pure)*: pull the MKV subtitle track
    (subrip/ASS) from matroska-demuxer, parse SRT/ASS, render. More work (ASS
    styling); defer.
  - Toggle = show/hide; a menu picks the language (from Jellyfin `MediaStreams`
    Type=Subtitle: Language/Title/IsForced/IsDefault).
- **Change audio track (languages)** — enumerate audio tracks + language from
  Jellyfin `MediaStreams` (Type=Audio: Language/Title/Codec/Channels) and the
  container. Switch options:
  - **(a) Re-open at position** *(reuses seek, simpler)*: re-open the stream at
    `clock_us` selecting the new audio track. Heavier (re-fetch) but no bespoke path.
  - **(b) In-place decoder swap**: tear down `AudioDec` + build the new track's
    (possibly different codec — AC-3 vs AAC vs Opus), re-point demux to the new
    track, flush ring, re-anchor. Faster, more code.

## "Anything else" — worth adding
- **Keyboard shortcuts** (map): Space=play/pause, ←/→=seek ∓10 s (J/L too),
  ↑/↓=volume, M=mute, S=subs, C/audio-key=cycle audio track, Esc/Back=stop, Home=
  restart. Wire in `on_key`.
- **Buffering indicator** — show a spinner when the fetch/prefetch is behind
  (RollingBuffer/prefetch state) so a stall reads as buffering, not a freeze.
- **Chapters** — if the container/Jellyfin exposes them, chapter skip + markers on
  the scrub bar.
- **Playback speed** (0.75/1.25/1.5/2×) — scales the clock; needs audio
  time-stretch to keep pitch (or accept pitch shift). *Defer — real time-stretch
  is its own task.*
- **Aspect fit/fill toggle**, **next-episode/autoplay** (TV, out of movie scope).

## Sequencing
1. Foundation: command channel + control bar + auto-hide + key bindings.
2. Tier 1 (pause/mute/volume/stop) — quick wins on the foundation.
3. Seek primitive → FF/REW + scrub + resume/progress-report.
4. Tier 3: subtitles (VTT-from-Jellyfin first), then audio-track switch.

## Decisions (LOCKED 2026-07-28)
- **Control bar**: **full transport** — play/pause + scrub bar + volume slider +
  mute, auto-hide ~3 s. Subtitle/audio-track menus added in Tier 3.
- **Jellyfin progress/resume**: **in v1** (`/Sessions/Playing/{,Progress,Stopped}`
  + resume from `UserData.PlaybackPositionTicks`, via the seek primitive).
- **Subtitles**: **Jellyfin VTT endpoint** (a) — fetch `Stream.vtt`, parse WebVTT,
  overlay via `draw_text`. No in-container subtitle demux.
- **Audio-track switch**: **re-open at position** (a) — reuse the seek primitive;
  handles a codec change for free.
- **Playback speed**: **deferred** (real time-stretch is its own task).

## Build order (chunks)
- **A — Foundation + Tier 1** (no seek needed), split for testable increments:
  - **A1 ✅ DONE** — command channel (`paused`/`muted`/`volume` on `Engine`; pump
    applies them); key bindings (Space/k pause, ↑/↓ volume, m mute, Esc/q stop);
    pause = wasi:audio `pause()` + clock re-anchor (shift `audio_start_ns`/`origin_ns`
    by paused duration → `media_now` continuous) + early-return pump; mute/volume =
    gain on the slice at the ring-write; minimal on-screen indicator (PAUSED pill +
    `vol%`/`muted` in the status line). Keyboard-testable.
  - **A2 ✅ DONE** — clickable transport bar in `render_playing`: auto-hiding
    (~3 s; `controls_until_ns`/`controls_bump`, paused pins it visible) title bar +
    bottom panel with play/pause button (rect-drawn play/pause glyphs), scrub
    *display* (played + buffered-ahead + knob), stop, mute button, volume slider;
    always-on thin progress sliver + A/V-sync readout. `on_pointer` reveals on any
    pointer + hit-tests playpause/stop/mute/volume-click via the shared `control_bar`
    layout. Scrub DRAG/seek = Chunk B.
- **B — Seek + Tier 2.** KEY FINDING: both demuxers read RANDOM-ACCESS through
  `HttpRangeReader` (`demux_cursor`≡`u64::MAX`; the RollingBuffer forward-fetch is
  inert), so seek needs NO manual byte-offset/fetch-window — just reposition +
  reset, and reads re-fetch on demand.
  - **B1 ✅ DONE** — `do_seek(p, target_us)` in bg-tick (MKV seek does I/O):
    MP4 = read `trak.mdia.minf.stbl.{stts,stss}` (public `trak`) → keyframe sample
    id ≤ target, set `next_v`/`next_a`; MKV = `MatroskaFile::seek(ticks)` (Cues).
    **MKV uses matroska-demuxer 0.8 + a VENDORED one-line patch** (stock 0.8's Cues
    seek mis-resolves `CueRelativePosition` → `InvalidEbmlDataSize`; patch in
    `vendor/matroska-demuxer` via `[patch.crates-io]`). **oxideav-mkv was evaluated
    then PARKED** — its `open()` scans to EOF for Cues → fetch storm over HTTP; keep
    matroska (header-only open). See `[[reference_jellyfin_container_demux_and_mkv_seek]]`
    + task 120 (wandr-media / oxideav long-game).
    Then clear queues + `pending_pcm`, `dec.reset()`, `pb.flush()` (audio ring), and
    clear the clock anchors (`first_pts_us`/`origin_ns`/audio-master) so `media_now`
    re-anchors at the landed keyframe. Command = `Engine.seek_request`; input:
    ←/→ (j/l) = ∓10 s, Home = restart, **scrub-bar click** = seek to fraction.
    (`mp4` crate's `SttsEntry` is `pub(crate)` → pass `(count,delta)` tuples.)
  - **B2 ✅ DONE** — scrub-bar continuous DRAG: `Engine.scrubbing`/`scrub_frac`;
    on_pointer down-on-track starts, move previews (playhead + target time follow
    the cursor, bar pinned visible), up commits `seek_request`. Click = down+up.
  - **B3 ✅ DONE** — Jellyfin progress/resume. `Item.resume_ticks` (browse
    `Fields=…,UserData` → `PlaybackPositionTicks`); `on_stream_started()` seeks to
    the resume point (>5 s, <90 %) + POSTs `/Sessions/Playing`; bg-tick posts
    `/Progress` every ~10 s of media time (position + paused); stop posts `/Stopped`
    with the final position. Reports = detached `reqwest::task::spawn`; ticks = µs·10.
- **C — Tier 3**:
  - **C1 ✅ DONE — subtitles (Jellyfin VTT overlay).** `Playback.subtitles` (Type=Subtitle
    MediaStreams); `s` cycles off→track→…→off (`Engine.sub_sel`/`sub_dirty`); bg-tick fetches
    `/Videos/{id}/{msid}/Subtitles/{idx}/Stream.vtt`, `parse_vtt` → timed cues in a `SUBTITLES`
    thread_local; render draws the active cue bottom-center (approx-centered, no text-measure
    API). Handles `[HH:]MM:SS.mmm`, multi-line, strips inline tags.
  - **C2 — next — audio-track switch** (re-open at position; enumerate audio MediaStreams).

## Notes / risks
- The **clock re-anchor on seek and on pause/resume** is the subtlest part — the
  audio-master `media_now` (`audio_first_pts + walltime − buffered`) must be
  re-initialised cleanly or A/V drifts. Test seek + pause/resume for sync.
- No WIT change expected — reset/present/decode + `wasi:audio` pause/write already
  exist. If a control turns out to need a new host verb, that stops and asks
  (WIT-approval rule).
- Subtitle-track and audio-track **metadata (language/title)** comes from the
  Jellyfin `MediaSources.MediaStreams` already fetched during PlaybackInfo.
