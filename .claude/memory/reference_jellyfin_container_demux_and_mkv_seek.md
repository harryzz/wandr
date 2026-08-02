---
name: reference_jellyfin_container_demux_and_mkv_seek
description: "wandr media demux ALL oxideav (mp4-crate RETIRED 2026-08-02): oxideav-mp4 @git-HEAD for progressive MP4 + CMAF, VENDORED oxideav-mkv fork (open_streaming) for MKV — one Demux::Ox path. NOT symphonia (audio-only). MKV storm = range-reqs/cluster not bytes; MP4 open skips mdat via seek."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-08-02T17:58:50.798Z
---

**UPDATE 2026-08-02 (later) — `mp4` 0.14 crate RETIRED; progressive MP4/MOV now
oxideav-mp4 too.** The engine unified progressive MP4 and MKV into ONE `Demux::Ox`
path (both are `Box<dyn oxideav_core::Demuxer>`): `finish_ox_open` pulls config from
`dmx.streams()`, and `fill_queues`/`do_seek`/`switch_audio` have a single arm. Deleted
the mp4-crate `Mp4Source`, `edit_offset_us`, `stts_sample_at`/`stts_time_of`,
`freq_to_index`, `fetch_moov_for_config`, `aac_freq_hz` (~230 lines). `oxideav-mp4` is
now PINNED to git HEAD `e9e8b579` (NOT crates.io 0.0.9, 2 months stale): the 2026-07-17
commits add FULL §8.6.6 edit-list timeline mapping on demux (empty-edit start delay +
trim + dwell + multi-segment) so packet PTS carries the mapped presentation time — the
engine dropped its bespoke `edit_offset_us` (A/V-sync edit handling is upstream now).
oxideav-mp4 `open()` skips `mdat` via `seek` (`skip_box_body`), so progressive open
over HTTP-Range = ~2 range-reqs / 0.4% even with moov AFTER mdat (proven
`repros/fmp4-probe/src/bin/mp4_probe.rs`). Device-verified: MP4 plays video+audio+seek.
MP4 also gained multi-audio-track switch ('a') for free. The same HEAD bump feeds the
DASH/CMAF Fmp4 path. See [[project_wasi_canvas_migration]] / task 119–120.

**UPDATE 2026-08-02 — MKV demux SWAPPED to a VENDORED wandr FORK of `oxideav-mkv`;
matroska-demuxer GONE.** The shared engine `crates/wandr-media-engine` demuxes MKV
via `crates/wandr-media-engine/vendor/oxideav-mkv/` (upstream git rev `79faa268` + one
added fn) on the oxideav `Demuxer` trait (`streams()`/`next_packet()`/`seek_to()`),
unifying MKV with fragmented-MP4/CMAF. Both apps' `[patch.crates-io] matroska-demuxer`
+ the vendored matroska crate were removed.

**The subtle trap (why the git-pin ALONE wasn't enough):** the July-17 SeekHead→Cues
jump only helps when Cues are SeekHead-*reachable*. For a **known-size MKV with no
reachable Cues** (many Jellyfin remuxes, incl. "Home Alone 2"), upstream `open()`
falls back to `scan_cues_from` — a walk from the first Cluster to segment-end that
SEEKS past each Cluster body reading only its ~12-byte header. It reads almost NO
bytes (so a bytes-read probe shows ~0% and looks fine!) but issues **one seek→read =
one HTTP-Range request PER CLUSTER** → thousands of TLS connects on a real movie = the
"infinity loop at START" the user saw (NOT on seek — matroska-demuxer looped on seek;
oxideav front-loads it to open). PROOF: `repros/fmp4-probe/src/bin/mkv_probe.rs`
counts RANGE REQUESTS (a read after a discontiguous seek), not bytes. On a 4.8 MB
`mkvmerge --no-cues` file: upstream `open` = **63** reqs (one/Cluster), fork
`open_streaming` = **7** (flat); on `big.mkv` (SeekHead-reachable Cues) both = 13 (no
regression). Make the storm-repro file with `mkvmerge --no-cues big.mkv` (an
`ffmpeg -live 1` file does NOT repro — it's unknown-size, a different no-scan path).

**The fork = one added fn `demux::open_streaming`** (+`open_streaming_typed`): identical
to `open` but threads `scan_late_cues=false` into `open_typed_impl`, gating ONLY the
final `scan_cues_from` fallback (front + SeekHead-reachable Cues still parse). The
engine calls `open_streaming`; a genuinely cue-less file then opens header-only and
`seek_to` returns `Error::unsupported` (non-resilient path, NO scan) → engine `do_seek`
treats that as a clean NO-OP (return current clock, no queue/decoder reset). For a
Cued file, `seek_to` returns the ACTUAL landed pts → clock re-anchors there. Promote
the vendored fork to a pushed `harryzz/oxideav-mkv` + upstream PR when convenient.
Motivation/tracking: task 120. The matroska-demuxer description below is HISTORICAL.

---

**(HISTORICAL) wandr.jellyfin container demux = `mp4` crate (MP4) + `matroska-demuxer` 0.8 (MKV,
patched — see below); symphonia is ONLY an audio decoder here. Don't try to demux
with symphonia — its format readers are audio-only.** Verified: `symphonia-format-mkv`
0.5.5 `codec_id_to_type` maps only `A_*` codecs (AAC/MP3/FLAC/Opus/Vorbis/PCM) —
**no video** (`V_MPEG4/ISO/AVC`, `V_MPEGH/ISO/HEVC`, `V_VP9`, `V_AV1` are absent).
Video tracks come back with no codec type + no `extra_data`, and it only fills
params for `track.audio`. Same shape for `symphonia-format-isomp4`. The app needs
video access units for the `wandr:video` decoder, so symphonia can't be the
demuxer. (Symphonia the DECODER is fine for AAC/MP3; it has no Opus decoder — see
[[reference_jellyfin_opus_ropus]].)

**MKV seek (task 119 Chunk B) = matroska-demuxer 0.8 + a VENDORED one-line patch.**
matroska-demuxer's stock `seek()` is broken: 0.6 hard-requires a Cluster in the
SeekHead → `CantFindCluster`; 0.8 fixes that but has a **cue-relative seek bug** —
when Cues carry `CueRelativePosition` (ffmpeg + most muxers write it),
`seek_broad_phase` resolves it against the FIRST cluster (`cluster_start`) instead of
the cue's OWN cluster → reader lands mid-element → `InvalidEbmlDataSize`. Symptom:
seek(0)/Home works, every non-zero seek fails ("only Home works").
- **Fix** (verified in `repros/mkv-seek-probe`, seeks to 5/15/30/60 s land exactly on
  keyframes): in `seek_broad_phase`'s `Some(relative_position)` branch, pass
  `point.track_position.cluster_position` to `get_cluster_offset_and_timestamp`
  instead of `cluster_start`.
- Shipped as a **vendored crate + `[patch.crates-io]`**:
  `apps/user/wandr.jellyfin/vendor/matroska-demuxer/` (patched 0.8.0) →
  `[patch.crates-io] matroska-demuxer = { path = "vendor/matroska-demuxer" }`.
  Promote to a GitHub fork (`harryzz/matroska-demuxer`) for upstreaming later.

**oxideav-mkv EVALUATION history.** Parked 2026-07-28: the published `=0.0.9`
`open()` runs `scan_cues_from`, a **linear scan to segment-EOF when Cues aren't
SeekHead-resolved**, which over HTTP-Range reads the whole movie → a fetch storm
("infinity loop"). ADOPTED 2026-08-02: the July-17 upstream commit (git rev
`79faa268`, still unpublished on crates.io) adds a SeekHead→post-cluster-Cues jump so
`open()` is header-only — proven by `mkv_probe`. NOTE `oxideav-http` (the framework's
own HTTP source) is native **ureq+rustls** (std sockets) → NOT wasip2-usable; we feed
oxideav our own `HttpRangeReader` via a `SendReader` unsafe-Send wrapper (sound in
single-threaded wasip2) because oxideav's `ReadSeek` requires `Send`. The user is
betting on oxideav as a growing pure-Rust ffmpeg replacement. Symphonia demux stays
ruled out (audio-only). Old probes: `repros/oxmkv-seek-probe`, `repros/mkv-seek-probe`.
- Seek architecture: both demuxers are RANDOM-ACCESS over `HttpRangeReader`
  (`demux_cursor`≡`u64::MAX`), so seek = reposition + reset, no byte-window work.
  MP4 seek reads `Mp4Track.trak.mdia.minf.stbl.{stts,stss}` (public `trak`) to find
  the keyframe sample ≤ target (`SttsEntry` is `pub(crate)` → pass (count,delta)
  tuples). See `do_seek` in `apps/user/wandr.jellyfin/src/lib.rs`.
