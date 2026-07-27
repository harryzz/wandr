---
name: project_task119_jellyfin_av_sync
description: Task 119 wandr.jellyfin A/V desync — RESOLVED. Root cause = ignored MP4 audio edit-list pre-roll + a latent host underrun-cursor bug it exposed.
metadata: 
  node_type: memory
  type: project
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-07-27T06:33:47.938Z
---

# Task 119 wandr.jellyfin — A/V desync ROOT CAUSE + FIX (RESOLVED)

**Symptom:** real Jellyfin H264/AAC titles played AUDIO ~8–11 s AHEAD of VIDEO,
constant, on BOTH WSLg desktop and native Windows, over HTTP and (proven) local file.

**Root cause (proven with `ffprobe` on the actual file):** the MP4's AUDIO track
carries an **empty edit** (`elst media_time = -1`) — a deliberate ~11 s silent
pre-roll: `ffprobe` shows video `start_time=0`, audio `start_time=11.066`. The
guest read RAW sample times and **ignored the edit list**, so it played the audio
~11 s early. Invisible to every internal clock because the guest never learned the
offset existed. The `mp4` crate DOES expose the edit list (`track.trak.edts.elst`);
we just weren't applying it. (This is why 3 read-only audits — host video, host
audio, guest pacing — all proved the *timing* code bounds skew to <1.3 s: the 8 s
was DATA the client failed to reconcile, the guest agent's Finding 3.)

**Fix 1 — GUEST (`apps/user/wandr.jellyfin/src/lib.rs`):**
- `edit_offset_us(track, movie_ts, media_ts)` sums empty-edit `segment_duration`
  (movie timescale = `+delay`) and subtracts `media_time` trims (media timescale).
- Applied to each track's PTS in `fill_queues` (video `−83 ms` reorder trim, audio
  `+11066 ms` pre-roll) → both tracks on ONE movie timeline.
- Unified clock in `pump_stream`: video free-runs from movie-0; a `!audio_pts_known`
  gate holds audio decode until the clock reaches its shifted first PTS; audio-master
  takes over continuously at that instant. NO "hold video until audio" gate.

**Fix 2 — HOST (`runtime/wandr-host/src/audio_desktop.rs`, the cpal callback):**
the long initial silence exposed the audio agent's **Finding 1**: on an empty ring
the resample `cursor` kept advancing (`cursor += ratio`) while `consumed` stayed 0,
so after the pre-roll it pointed far past the buffer and READ SILENCE OVER real
audio (pos advanced, nothing heard). Fix: on `avail_frames==0` output silence +
`cursor=0.0` and return; on partial underrun (`want>avail`) reset `cursor.fract()`.
This is a genuine host bug worth keeping regardless of the edit-list case.

Also fixed earlier & kept: audio resampler rate is driven by the **MP4 media
timescale**, not the ASC freq index (HE-AAC/SBR or mux disagreements caused a
*growing* fast-audio drift — "By the Sea").

**Verified:** local The Drama on desktop — first ~11 s correctly silent (matches
web player), then audio in sync; user-confirmed. Buffer healthy (bufd 500 ms).

**State / follow-ups:**
- Debug scaffolding removed (diag: line, FIRST-frame logs, `av_delay_ms.txt` knob,
  delay-queue). KEPT: on-screen A/V readout (`VIDEO clk | AUDIO pos | Δ`) for
  format testing, and the **local-file test mode** (see below).
- **Local-file test mode**: `[[mounts]] host="~/Videos" guest="/media"` in
  package.toml; `/state/jellyfin/local_mp4.txt` names a guest path → app auto-plays
  it via the SAME pipeline (disk transport). `HttpRangeReader::new_local` reads
  windows off disk through the same cache. Mount host must resolve at the preopen
  ROOT (symlink `~/Videos`→dir, not a symlink INSIDE — WASI won't follow it out).
- **WINDOWS still needs a host rebuild** (`build-host-windows.bat`) to get Fix 2 —
  I can't cross-build it from WSL. Guest (Fix 1) is deployed there. Pre-roll files
  (The Drama) stay silent-after-preroll on Windows until the host is rebuilt;
  normal files (no edit-list) work with just the guest fix.
- TODO: verify "By the Sea" (different edit / HE-AAC) + the streamed Jellyfin path.
See [[reference_gstreamer_desktop_backend_spike]], [[reference_wslg_wayland_resize_crash]].
