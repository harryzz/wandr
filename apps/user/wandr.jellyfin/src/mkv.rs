//! Streaming EBML / Matroska (MKV / WebM) demux over HTTP Range.
//!
//! Pure-Rust MKV demuxers exist (symphonia-format-mkv, matroska-demuxer) but all
//! read via synchronous `Read + Seek` over a whole seekable file — the same model
//! that ruled out the mp4 crate's `read_sample` for our async-Range source. So,
//! as with MP4, we hand-roll — but Matroska is far friendlier to streaming:
//! WebM was designed for HTTP, so it is a forward, in-time-order element stream
//! (EBML header → Info/Tracks up front → Clusters in order). We parse the header
//! once, then walk Clusters forward as Range windows arrive, emitting one
//! `MkvBlock` per frame. The DECODE half is 100% shared with the MP4 path:
//! avcC/hvcC → Annex-B via streaming::to_annexb, AAC ASC → Symphonia, block
//! timecodes → PTS.
//!
//! Element IDs and CodecID strings are taken from the Matroska spec, cross-checked
//! against symphonia-format-mkv's element_ids.rs (read-source-first).

use crate::streaming::RollingBuffer;

// ---- element IDs -----------------------------------------------------------
const ID_EBML: u64 = 0x1A45DFA3;
const ID_SEGMENT: u64 = 0x18538067;
const ID_INFO: u64 = 0x1549A966;
const ID_TIMESTAMP_SCALE: u64 = 0x2AD7B1;
const ID_DURATION: u64 = 0x4489;
const ID_TRACKS: u64 = 0x1654AE6B;
const ID_TRACK_ENTRY: u64 = 0xAE;
const ID_TRACK_NUMBER: u64 = 0xD7;
const ID_TRACK_TYPE: u64 = 0x83;
const ID_CODEC_ID: u64 = 0x86;
const ID_CODEC_PRIVATE: u64 = 0x63A2;
const ID_VIDEO: u64 = 0xE0;
const ID_PIXEL_WIDTH: u64 = 0xB0;
const ID_PIXEL_HEIGHT: u64 = 0xBA;
const ID_AUDIO: u64 = 0xE1;
const ID_SAMPLING_FREQ: u64 = 0xB5;
const ID_CHANNELS: u64 = 0x9F;
const ID_CLUSTER: u64 = 0x1F43B675;
const ID_CLUSTER_TS: u64 = 0xE7;
const ID_SIMPLE_BLOCK: u64 = 0xA3;
const ID_BLOCK_GROUP: u64 = 0xA0;
const ID_BLOCK: u64 = 0xA1;
const ID_REFERENCE_BLOCK: u64 = 0xFB;

const START: [u8; 4] = [0, 0, 0, 1];

/// Read an EBML variable-length integer at `buf[pos..]`. `keep_marker` = keep the
/// length-marker bit (element IDs) vs strip it (sizes / track numbers). Returns
/// (value, byte_length) or None if not enough bytes / malformed.
fn read_vint(buf: &[u8], pos: usize, keep_marker: bool) -> Option<(u64, usize)> {
    let first = *buf.get(pos)?;
    if first == 0 {
        return None; // 5+ leading-zero-byte VINTs unsupported (never used here)
    }
    let len = first.leading_zeros() as usize + 1; // 1..=8
    if pos + len > buf.len() {
        return None;
    }
    let mut val = if keep_marker {
        first as u64
    } else {
        (first & (0xFFu8 >> len)) as u64
    };
    for k in 1..len {
        val = (val << 8) | buf[pos + k] as u64;
    }
    Some((val, len))
}

/// Is this size field the "unknown size" sentinel (all data bits set)? Master
/// elements (Segment, Cluster) may use it in streamed files.
fn is_unknown_size(val: u64, len: usize) -> bool {
    val == (1u64 << (7 * len)) - 1
}

/// An element header: id, byte length of (id+size) header, and payload size
/// (`None` = unknown/streamed).
struct ElemHeader {
    id: u64,
    header_len: usize,
    size: Option<u64>,
}

/// Read an element header at `buf[pos..]`. None if fewer than the full header is
/// buffered.
fn read_header(buf: &[u8], pos: usize) -> Option<ElemHeader> {
    let (id, id_len) = read_vint(buf, pos, true)?;
    let (sz, sz_len) = read_vint(buf, pos + id_len, false)?;
    let size = if is_unknown_size(sz, sz_len) { None } else { Some(sz) };
    Some(ElemHeader { id, header_len: id_len + sz_len, size })
}

// ---- header parse (Info + Tracks) ------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct MkvTrack {
    pub number: u64,
    pub codec_id: String,
    pub codec_private: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Clone, Debug)]
pub struct MkvHeader {
    /// Nanoseconds per timestamp tick (default 1_000_000 = 1 ms).
    pub timestamp_scale_ns: u64,
    pub duration_us: i64,
    pub video: Option<MkvTrack>,
    pub audio: Option<MkvTrack>,
    /// Absolute file offset of the first Cluster (where block demux begins).
    pub first_cluster_off: u64,
}

/// Walk immediate children of a master element occupying `buf[start..end]`,
/// calling `f(id, payload_start, payload_end)`. Stops at the buffer end.
fn walk_children(buf: &[u8], start: usize, end: usize, mut f: impl FnMut(u64, usize, usize)) {
    let mut i = start;
    while i < end {
        let Some(h) = read_header(buf, i) else { break };
        let payload = i + h.header_len;
        let psize = match h.size {
            Some(s) => s as usize,
            None => break, // unknown-size child inside a bounded master: stop
        };
        let pend = (payload + psize).min(end);
        f(h.id, payload, pend);
        i = payload + psize;
    }
}

fn read_uint(buf: &[u8], start: usize, end: usize) -> u64 {
    let mut v = 0u64;
    for &b in &buf[start..end.min(buf.len())] {
        v = (v << 8) | b as u64;
    }
    v
}

fn read_float(buf: &[u8], start: usize, end: usize) -> f64 {
    match end - start {
        4 => f32::from_be_bytes(buf[start..start + 4].try_into().unwrap()) as f64,
        8 => f64::from_be_bytes(buf[start..start + 8].try_into().unwrap()),
        _ => 0.0,
    }
}

fn parse_track_entry(buf: &[u8], start: usize, end: usize) -> MkvTrack {
    let mut t = MkvTrack::default();
    walk_children(buf, start, end, |id, s, e| match id {
        ID_TRACK_NUMBER => t.number = read_uint(buf, s, e),
        ID_CODEC_ID => t.codec_id = String::from_utf8_lossy(&buf[s..e]).trim_end_matches('\0').to_string(),
        ID_CODEC_PRIVATE => t.codec_private = buf[s..e].to_vec(),
        ID_VIDEO => walk_children(buf, s, e, |vid, vs, ve| match vid {
            ID_PIXEL_WIDTH => t.width = read_uint(buf, vs, ve) as u32,
            ID_PIXEL_HEIGHT => t.height = read_uint(buf, vs, ve) as u32,
            _ => {}
        }),
        ID_AUDIO => walk_children(buf, s, e, |aid, as_, ae| match aid {
            ID_SAMPLING_FREQ => t.sample_rate = read_float(buf, as_, ae) as u32,
            ID_CHANNELS => t.channels = read_uint(buf, as_, ae) as u32,
            _ => {}
        }),
        _ => {}
    });
    t
}

/// Track type is read separately (the closure above can't both borrow t and hold
/// the type), so re-scan for it.
fn track_type(buf: &[u8], start: usize, end: usize) -> u64 {
    let mut ty = 0;
    walk_children(buf, start, end, |id, s, e| {
        if id == ID_TRACK_TYPE {
            ty = read_uint(buf, s, e);
        }
    });
    ty
}

/// Parse the EBML header region (must contain a complete Info + Tracks and reach
/// the first Cluster). Returns None if the buffer is too short or not Matroska.
pub fn parse_header(buf: &[u8]) -> Option<MkvHeader> {
    // Top level: EBML header, then Segment.
    let ebml = read_header(buf, 0)?;
    if ebml.id != ID_EBML {
        return None;
    }
    let seg_start = (ebml.header_len + ebml.size? as usize).min(buf.len());
    let seg = read_header(buf, seg_start)?;
    if seg.id != ID_SEGMENT {
        return None;
    }
    // Segment payload begins here; Cluster/Cue byte-positions are relative to it.
    let seg_data = seg_start + seg.header_len;

    let mut hdr = MkvHeader {
        timestamp_scale_ns: 1_000_000,
        duration_us: 0,
        video: None,
        audio: None,
        first_cluster_off: 0,
    };
    let mut duration_ticks = 0.0f64;

    // Walk Segment children until the first Cluster.
    let mut i = seg_data;
    while i < buf.len() {
        let Some(h) = read_header(buf, i) else { break };
        if h.id == ID_CLUSTER {
            hdr.first_cluster_off = i as u64;
            break;
        }
        let payload = i + h.header_len;
        let Some(psize) = h.size else { break };
        let pend = (payload + psize as usize).min(buf.len());
        match h.id {
            ID_INFO => walk_children(buf, payload, pend, |id, s, e| match id {
                ID_TIMESTAMP_SCALE => hdr.timestamp_scale_ns = read_uint(buf, s, e),
                ID_DURATION => duration_ticks = read_float(buf, s, e),
                _ => {}
            }),
            ID_TRACKS => walk_children(buf, payload, pend, |id, s, e| {
                if id == ID_TRACK_ENTRY {
                    let mut t = parse_track_entry(buf, s, e);
                    match track_type(buf, s, e) {
                        1 => { // video
                            if hdr.video.is_none() { hdr.video = Some(std::mem::take(&mut t)); }
                        }
                        2 => { // audio
                            if hdr.audio.is_none() { hdr.audio = Some(std::mem::take(&mut t)); }
                        }
                        _ => {}
                    }
                }
            }),
            _ => {}
        }
        i = payload + psize as usize;
    }
    hdr.duration_us = (duration_ticks * hdr.timestamp_scale_ns as f64 / 1000.0) as i64;
    if hdr.first_cluster_off == 0 || hdr.video.is_none() {
        return None;
    }
    Some(hdr)
}

// ---- codec config from CodecPrivate ----------------------------------------

/// avcC (H.264 CodecPrivate) → start-coded SPS/PPS prefix + NAL length size.
pub fn parse_avcc(cp: &[u8]) -> Option<(Vec<u8>, usize)> {
    if cp.len() < 7 {
        return None;
    }
    let nal_len = (cp[4] & 0x03) as usize + 1;
    let mut out = Vec::new();
    let mut i = 6usize;
    let num_sps = cp[5] & 0x1f;
    for _ in 0..num_sps {
        let l = u16::from_be_bytes([*cp.get(i)?, *cp.get(i + 1)?]) as usize;
        i += 2;
        out.extend_from_slice(&START);
        out.extend_from_slice(cp.get(i..i + l)?);
        i += l;
    }
    let num_pps = *cp.get(i)?;
    i += 1;
    for _ in 0..num_pps {
        let l = u16::from_be_bytes([*cp.get(i)?, *cp.get(i + 1)?]) as usize;
        i += 2;
        out.extend_from_slice(&START);
        out.extend_from_slice(cp.get(i..i + l)?);
        i += l;
    }
    Some((out, nal_len))
}

/// hvcC (H.265 CodecPrivate) → start-coded VPS/SPS/PPS prefix + NAL length size.
pub fn parse_hvcc(cp: &[u8]) -> Option<(Vec<u8>, usize)> {
    let nal_len = (*cp.get(21)? & 0x03) as usize + 1;
    let num_arrays = *cp.get(22)? as usize;
    let mut out = Vec::new();
    let mut i = 23usize;
    for _ in 0..num_arrays {
        let num = u16::from_be_bytes([*cp.get(i + 1)?, *cp.get(i + 2)?]) as usize;
        i += 3;
        for _ in 0..num {
            let l = u16::from_be_bytes([*cp.get(i)?, *cp.get(i + 1)?]) as usize;
            i += 2;
            out.extend_from_slice(&START);
            out.extend_from_slice(cp.get(i..i + l)?);
            i += l;
        }
    }
    Some((out, nal_len))
}

// ---- streaming block demux -------------------------------------------------

/// One demuxed frame: which track, its PTS, keyframe flag, and the raw frame
/// bytes (length-prefixed NALs for H.264/5 — the caller Annex-B-frames them).
pub struct MkvBlock {
    pub track: u64,
    pub pts_us: i64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// Result of trying to pull the next block.
pub enum Next {
    Block(MkvBlock),
    /// Not enough bytes buffered at `pos` — fetch forward and retry.
    NeedMore,
    /// Reached the end of the stream.
    Done,
}

/// Forward block demuxer: parses Clusters/Blocks from the rolling buffer in file
/// order, tracking the current cluster timestamp. `pos` is the absolute file
/// offset of the next element to parse.
pub struct MkvDemux {
    pub pos: u64,
    pub cluster_ts: u64,
    pub timestamp_scale_ns: u64,
    pub video_track: u64,
    pub audio_track: u64,
    pub total_len: u64,
}

impl MkvDemux {
    fn pts_us(&self, ticks: i64) -> i64 {
        ticks * self.timestamp_scale_ns as i64 / 1000
    }

    /// Pull the next block on the video or audio track, skipping everything else.
    /// Advances `pos`. Blocks with lacing produce their FIRST frame only (movie
    /// video/AAC in MKV is unlaced in practice; laced multi-frame is a TODO).
    pub fn next(&mut self, buf: &RollingBuffer) -> Next {
        loop {
            if self.pos >= self.total_len {
                return Next::Done;
            }
            // Need the element header buffered.
            let hslice = match buf.slice(self.pos, 12.min((self.total_len - self.pos) as u32)) {
                Some(s) => s,
                None => return Next::NeedMore,
            };
            let Some(h) = read_header(hslice, 0) else { return Next::NeedMore };
            let payload = self.pos + h.header_len as u64;

            match h.id {
                // Master containers we descend INTO (advance past header only).
                ID_SEGMENT | ID_CLUSTER | ID_BLOCK_GROUP => {
                    self.pos = payload;
                    continue;
                }
                ID_CLUSTER_TS => {
                    let sz = h.size.unwrap_or(0) as u32;
                    let Some(s) = buf.slice(payload, sz) else { return Next::NeedMore };
                    self.cluster_ts = read_uint(s, 0, sz as usize);
                    self.pos = payload + sz as u64;
                    continue;
                }
                ID_SIMPLE_BLOCK | ID_BLOCK => {
                    let Some(sz) = h.size else { return Next::Done };
                    let Some(body) = buf.slice(payload, sz as u32).map(|b| b.to_vec()) else {
                        return Next::NeedMore;
                    };
                    self.pos = payload + sz;
                    if let Some(blk) = self.parse_block(&body, h.id == ID_SIMPLE_BLOCK) {
                        return Next::Block(blk);
                    }
                    continue; // block on an ignored track
                }
                _ => {
                    // Skip any other element (SeekHead, Cues, Void, Tags, …).
                    match h.size {
                        Some(sz) => { self.pos = payload + sz; continue; }
                        None => return Next::Done,
                    }
                }
            }
        }
    }

    /// Parse a (Simple)Block body: track VINT, int16 rel timecode, flags, frame.
    fn parse_block(&self, body: &[u8], simple: bool) -> Option<MkvBlock> {
        let (track, tn) = read_vint(body, 0, false)?;
        if track != self.video_track && track != self.audio_track {
            return None;
        }
        let rel = i16::from_be_bytes([*body.get(tn)?, *body.get(tn + 1)?]) as i64;
        let flags = *body.get(tn + 2)?;
        let lacing = (flags >> 1) & 0b11;
        let data_start = tn + 3;
        // Unlaced (or first frame of a laced block — good enough for playback of
        // unlaced movie tracks; laced audio is a known TODO).
        let frame = if lacing == 0 {
            body.get(data_start..)?.to_vec()
        } else {
            // Skip the lacing header (num-frames-1 byte) and take the remainder as
            // a single frame; imperfect but avoids a hard failure on laced input.
            body.get(data_start + 1..)?.to_vec()
        };
        // SimpleBlock keyframe = flags bit7; BlockGroup Block keyframes are marked
        // by the ABSENCE of a ReferenceBlock — we can't see that here, so treat
        // BlockGroup blocks conservatively as non-keyframe (the first cluster
        // still opens on a SimpleBlock keyframe in practice).
        let keyframe = simple && (flags & 0x80) != 0;
        Some(MkvBlock {
            track,
            pts_us: self.pts_us(self.cluster_ts as i64 + rel),
            keyframe,
            data: frame,
        })
    }
}

// Reference-block id kept for documentation of the keyframe rule above.
const _: u64 = ID_REFERENCE_BLOCK;
