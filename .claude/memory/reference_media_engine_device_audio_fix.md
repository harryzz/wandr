---
name: reference_media_engine_device_audio_fix
description: "Why wandr-media-engine streaming audio was silent on device (not desktop) and the 3-part async-native fix — block_on deadlock, gate margin, write-then-start HAL standby."
metadata: 
  node_type: memory
  type: reference
  originSessionId: c6bfd2e3-58ed-44e9-8de8-85655ad45867
  modified: 2026-08-04T07:18:44.666Z
---

# media-engine: no-audio-on-device fix (commit `95c1e588`, task 120)

Symptom: navidrome/jellyfin/dash/flac.test opened the audio track on device but played
NO audio (device standby); worked fine on desktop. Root causes, all device-only (the
desktop wasmtime executor + fast network hid them). Verified on flac.test (auto-plays a
SoundHelix MP3 → `Demux::Audio`, same path navidrome uses); user confirmed audible.

**Diagnosing:** guest `println!`→stdout→/dev/null on device. Guest **stderr**→logcat, so
`engine::log` now uses `eprintln!`. A per-second DIAG log of pipeline state
(pending_pcm/audio_q/buffered_frames/prefetch-ahead/flags) was the decisive tool — add a
throttled one again if regressing. Device is `--no-art`, so `wm size`/`density` fail;
read density from the host log `report-panel WxH dpi=NNN` (Pixel 2 XL = 1440×2880 dpi 560).

## 1. `block_on`-over-async DEADLOCK (the core silence + spin)
`httprange::HttpRangeReader::read` fell back to `block_on(fetch_range)` on a cache miss,
called synchronously from inside the async `bg_tick`. On the device's single-threaded
wasip3 executor `block_on` RE-ENTERS the executor: the wasi:tls fetch it waits on can only
progress if the host polls the bg_tick future, but the host is blocked waiting for bg_tick
to return → deadlock. bg-tick spins (proc state "R"), prefetch (a spawned task) can't run,
TLS fetches STOP. Desktop tolerated the nesting.
**Fix — async-native:** during playback (`ReaderShared.streaming`, flipped by
`PrefetchHandle::mark_streaming` after the open-time probe) a miss returns `WouldBlock`,
never block_on. The demux GATES: `fill_queues` only pulls a packet when
`PrefetchHandle::ready_ahead()` reports ≥ `FILL_GATE_MARGIN` bytes cached contiguously
ahead (or at_eof); else break, let async `drive_prefetch` advance, retry next tick.
block_on survives ONLY for the one-shot open probe + local-file mode (no executor re-entry).

## 2. GATE MARGIN too large → ~3 s startup stall → HAL standby
First attempt used `FILL_GATE_MARGIN = 2 MiB` = the fetch WINDOW size. After the open
probe fetched one 2 MiB window, `ahead` was ~2041 KiB < 2048 KiB → gate blocked decode for
~3 s (until prefetch crawled one window past the margin). The empty device ring underran
into HAL standby in that gap. **Fix:** margin = **256 KiB** (just over one packet's read;
the initial window is ≫ that → decode starts immediately). Margin must exceed one
produce-iteration's read (packet + demuxer block-read-ahead), NOT the window.

## 3. START-on-empty-ring → deep-buffer HAL standby (never recovers)
`ensure_audio_device` called `pb.start()` at OPEN, before any PCM. The Qualcomm
deep-buffer HAL started on an empty ring, underran, and went to `disable_audio_route`
standby that new writes did NOT wake (once the ring later filled, `buffered_frames` pinned
at max = not draining = silent). Local-file players (audio.player) never see this — they
decode+write instantly. **Fix — WRITE-THEN-START handshake** (host code documents it):
open WITHOUT start; call `start()` only AFTER the ring's first write (`StreamPlayer.audio_started`).
Healthy signature: `buffered_frames` OSCILLATES (device draining) and 0 `disable_audio_route`.

## Also
- Demux read error → `demux_done` ONLY when genuinely at EOF (`ready_ahead().at_eof`), not
  on a transient miss — margin-independent + demuxer-agnostic (protects the video Ox path).
- **NOT fixed:** dash `Fmp4` `SegStream` has its OWN `block_on` (per-segment fetch, separate
  from HttpRangeReader) — same deadlock risk on device; needs the same async-native gate.
- **NOT fixed:** tiny fonts (density) — solved separately by the Slint UI migration, see
  [[project_navidrome_slint_migration]].

Files: `crates/wandr-media-engine/src/httprange.rs` (streaming flag, WouldBlock,
mark_streaming, ready_ahead) + `src/lib.rs` (FILL_GATE_MARGIN, gate in fill_queues,
demux_at_eof, write-then-start, audio_started, eprintln log).
