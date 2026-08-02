---
name: reference_mp4_014_fragmented_read_sample_broken
description: mp4 crate 0.14 read_sample() is BROKEN for fragmented/CMAF MP4 — parses moof/trun boxes but computes wrong sample offsets; task 119 Part B needs a hand-written fragmented sample reader
metadata: 
  node_type: memory
  type: reference
  originSessionId: c6bfd2e3-58ed-44e9-8de8-85655ad45867
  modified: 2026-08-02T12:27:26.038Z
---

**`mp4` crate 0.14 CANNOT read fragmented/CMAF (DASH/HLS) samples** — verified by
reading its source (not just docs). It *parses* the fragment box tree fine
(`Mp4Reader.moofs: Vec<MoofBox>` with `traf`/`trun`/`tfhd`/`tfdt`,
`is_fragmented()`, codec config from the init `moov` `stsd`), but
`track.rs::sample_offset()` on the `!trafs.is_empty()` path returns a **constant**
`tfhd.base_data_offset.unwrap_or(0)` per fragment: it ignores the sample index,
never adds `trun.data_offset` nor the accumulated prior-sample sizes, and is `0`
when `tfhd` omits `base_data_offset` (the normal CMAF "default-base-is-moof" case).
So `read_sample()` (track.rs:550) seeks every sample to the same wrong byte →
garbage. `sample_time()` for fragments is also crude (`(sample_id-1) *
default_sample_duration`, ignores per-`trun` durations).

**⚠️ The task 119 doc CLAIMED this was "PROVEN" (`repros/fmp4-probe`), but that
probe is EMPTY (`fn main(){}`) and commit `bd892f17` changed only docs — a
doc-only "proof". Classic proven-vs-guessed trap; caught by reading the crate
source.** See `[[feedback_humility_proven_vs_guessed]]`, `[[feedback_read_source_first]]`.

**What to do for `Demux::Fmp4` (task 119 Part B / `wandr-media-engine`):** reuse
`mp4` 0.14 for BOX PARSING only (init-segment `moov` for codec config; the parsed
`trun`/`tfhd`/`tfdt` per fragment give `sample_sizes`/`sample_durations`/`sample_cts`/
`data_offset`/`base_media_decode_time`), then compute the sample layout OURSELVES:
byte range = `moof_start + trun.data_offset + Σ prior sizes`, size = `trun.sample_sizes[i]`
(or `tfhd.default_sample_size`), PTS = `tfdt.base_media_decode_time + Σ durations`
(`+ trun.sample_cts[i]` for composition), scaled by the init `mdhd` timescale.
DASH video+audio are SEPARATE segment streams, so `Demux::Fmp4` holds TWO sources
(video→VFrame H.264/HEVC AnnexB, audio→AFrame AAC) feeding the one StreamPlayer.
Related: [[reference_jellyfin_container_demux_and_mkv_seek]].

**✅ RESOLUTION (2026-08-01): use `oxideav-mp4` 0.0.9 — NOT a hand-parse, NOT
patching `mp4` 0.14.** It is a purpose-built streaming CMAF/fragmented demuxer,
pure-Rust MIT, in the oxideav ecosystem we already use (`oxideav-core`/`-ac3`),
and it COMPILES for wasm32-wasip2. EMPIRICALLY PROVEN on real Tears-of-Steel CMAF
(`repros/fmp4-probe`, native run, both video+audio PASS): correct avcC/ASC codec
config matching the manifest, IDR keyframe first, monotonic PTS, realistic frame
sizes — i.e. the sample-offset math the `mp4` crate got wrong is correct here.
API: `oxideav_mp4::demux::open(Box<dyn oxideav_core::ReadSeek /*Read+Seek+Send*/>,
&NullCodecResolver) -> Box<dyn oxideav_container::Demuxer>`; `Demuxer::streams()
-> [StreamInfo{params: CodecParameters{codec_id, media_type, width/height,
sample_rate/channels, extradata}}]`, `next_packet() -> Packet{stream_index,
pts:Option<i64>, dts, flags.keyframe, data}`, `seek_to(stream, pts)` (tfra/sidx,
fragment-bounded fallback — NOT the `oxideav-mkv` EOF-scan), `duration_micros`.
Packet.data is AVCC length-prefixed (nal_len from avcC lengthSizeMinusOne) — the
engine's `install_player` already does AVCC→AnnexB. CAUTIONS: reader must be
`Send` (`HttpRangeReader` is `Rc`→!Send: wrap it); pin EXACT `=0.0.9` (young, like
`oxideav-ac3`). Scope: DASH-only `Demux::Fmp4`; do NOT replace the working `mp4`
0.14 progressive path yet.

**MULTI-SEGMENT confirmed (2026-08-01):** `oxideav-mp4` demuxes a CONCATENATION
of `init + seg0 + seg1 + seg2` (each media segment = `styp`+`moof`+`mdat`)
continuously — 288 video + 563 audio packets, monotonic PTS spanning 12.0s across
all 3 segment boundaries. So `open_fmp4`'s download-then-play (concat all rep
segments into one Cursor, one demuxer) is sound. WIRED: `wandr-media-engine`
`Demux::Fmp4` + `apps/user/wandr.dash` (dash-mpd parse → resolve segment URLs →
engine). Verified playing on desktop (WSLg), video+audio+seek.

**STREAMING (per-segment) — the design forced by oxideav's open():**
`oxideav_mp4::demux::open()` WALKS THE WHOLE INPUT TO EOF (`while let Some(hdr) =
read_box_header(...)`) to index every fragment, so you CANNOT hand it a lazy
"virtual concat" reader and expect streaming — it would fetch everything up front.
Instead `Demux::Fmp4` holds a `SegStream` per rep that opens a FRESH demuxer per
`init + ONE media segment` (each segment is a keyframe-aligned, self-contained
fragmented file; PTS is ABSOLUTE from its `tfdt` — verified: init+seg1 alone →
first pts = the segment's start, not 0). Fetch is blocking (`wit_bindgen …
block_on(net::fetch_url)`) — legal because fill_queues runs in the async bg-tick
(same as the MP4/MKV Range reader). Seek = jump `SegStream.idx` to the segment
covering the target (segment-granular, lands on a keyframe). Result: ~1 s startup
(vs ~90 s download-then-play), bounded memory (init + 1 segment), fetch-on-demand.
DASH addressing: `apps/user/wandr.dash` handles BOTH `$Time$` (SegmentTimeline) and
`$Number$` (fixed `@duration`+`@startNumber`) modes. Default stream = DASH-IF Big
Buck Bunny (dash.akamaized.net); unified-streaming rate-limits after heavy use.
