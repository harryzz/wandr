//! Streaming MP4 demux over HTTP Range.
//!
//! A real player cannot download a 2.7 GB file to demux it, and the `mp4` crate
//! only reads a sample once its `mdat` bytes are in hand (`sample_offset` is
//! private). So we do what every HTTP streaming player does: range-fetch the
//! `moov` index once, parse the SAMPLE TABLE ourselves (per-sample file offset /
//! size / timestamp / sync flag), then range-fetch `mdat` windows on demand and
//! slice each sample out. Demux stays entirely guest-side (wasi-media-source);
//! the host only decodes.
//!
//! This module is the pure, testable half: box-walk `moov` → `TrackTable`s, plus
//! a `RollingBuffer` that holds just the fetched `mdat` window. The fetch/decode
//! loop that drives it lives in `lib.rs`.

/// One coded sample as located in the file (NOT its bytes — those come later
/// from a range fetch).
#[derive(Clone, Debug)]
pub struct SampleRef {
    pub offset: u64,
    pub size: u32,
    /// Presentation time in microseconds (DTS + composition offset).
    pub pts_us: i64,
    /// Decode time in microseconds — the order samples sit in `mdat`.
    pub dts_us: i64,
    pub is_sync: bool,
}

/// A demuxed track: its media timescale and the full ordered sample list.
#[derive(Clone, Debug)]
pub struct TrackTable {
    /// The 4-char handler type: `vide` (video) or `soun` (audio).
    pub handler: [u8; 4],
    pub timescale: u32,
    /// Samples in DECODE order (as stored in `mdat`).
    pub samples: Vec<SampleRef>,
}

impl TrackTable {
    pub fn is_video(&self) -> bool { &self.handler == b"vide" }
    pub fn is_audio(&self) -> bool { &self.handler == b"soun" }
    /// Highest byte offset touched — used to bound the fetch cursor.
    pub fn max_end(&self) -> u64 {
        self.samples.iter().map(|s| s.offset + s.size as u64).max().unwrap_or(0)
    }
}

// ---- box walking -----------------------------------------------------------

fn be32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn be64(b: &[u8], i: usize) -> u64 {
    u64::from_be_bytes([b[i], b[i+1], b[i+2], b[i+3], b[i+4], b[i+5], b[i+6], b[i+7]])
}

/// Iterate the immediate child boxes within `buf[start..end]`, yielding
/// `(type, payload_start, payload_end)`. Handles 32-bit and 64-bit (`size==1`)
/// box sizes; stops on a malformed/zero size rather than looping.
fn children(buf: &[u8], start: usize, end: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut i = start;
    while i + 8 <= end {
        let mut size = be32(buf, i) as usize;
        let typ = [buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]];
        let mut header = 8;
        if size == 1 {
            if i + 16 > end { break; }
            size = be64(buf, i + 8) as usize;
            header = 16;
        } else if size == 0 {
            size = end - i; // to end-of-parent
        }
        if size < header || i + size > end { break; }
        out.push((typ, i + header, i + size));
        i += size;
    }
    out
}

/// Locate a top-level box by type, returning its absolute `[box_start, box_end)`
/// extent (header included). Used to fetch exactly the `moov` index: if this
/// returns `None`, `moov` isn't in the buffer (a non-faststart file with `moov`
/// at the tail — fetch the tail and retry).
pub fn top_level_box(buf: &[u8], typ: &[u8; 4]) -> Option<(u64, u64)> {
    let mut i = 0usize;
    while i + 8 <= buf.len() {
        let mut size = be32(buf, i) as u64;
        let t = [buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]];
        if size == 1 {
            if i + 16 > buf.len() { break; }
            size = be64(buf, i + 8);
        } else if size == 0 {
            return if &t == typ { Some((i as u64, buf.len() as u64)) } else { None };
        }
        if size < 8 { break; }
        if &t == typ {
            return Some((i as u64, i as u64 + size));
        }
        i += size as usize;
    }
    None
}

/// Find the first child box of `typ` within `[start,end)`.
fn find(buf: &[u8], start: usize, end: usize, typ: &[u8; 4]) -> Option<(usize, usize)> {
    children(buf, start, end).into_iter().find(|(t, _, _)| t == typ).map(|(_, s, e)| (s, e))
}

/// Parse every track's sample table from a buffer that contains a complete
/// `moov` box (i.e. the range-fetched head). Returns one `TrackTable` per trak.
pub fn parse_tracks(head: &[u8]) -> Result<Vec<TrackTable>, String> {
    let (mv_s, mv_e) = find(head, 0, head.len(), b"moov").ok_or("no moov in head buffer")?;
    let mut tracks = Vec::new();
    for (t, ts, te) in children(head, mv_s, mv_e) {
        if &t != b"trak" { continue; }
        if let Some(tt) = parse_trak(head, ts, te) {
            tracks.push(tt);
        }
    }
    if tracks.is_empty() {
        return Err("no parsable tracks in moov".into());
    }
    Ok(tracks)
}

fn parse_trak(b: &[u8], s: usize, e: usize) -> Option<TrackTable> {
    let (md_s, md_e) = find(b, s, e, b"mdia")?;
    // handler type (vide/soun) from mdia/hdlr (offset 8 in payload = handler FourCC).
    let (hd_s, _hd_e) = find(b, md_s, md_e, b"hdlr")?;
    let handler = [b[hd_s + 8], b[hd_s + 9], b[hd_s + 10], b[hd_s + 11]];
    // timescale from mdia/mdhd (version-dependent layout).
    let (mh_s, _mh_e) = find(b, md_s, md_e, b"mdhd")?;
    let version = b[mh_s];
    let timescale = if version == 1 { be32(b, mh_s + 20) } else { be32(b, mh_s + 12) };
    if timescale == 0 { return None; }

    let (minf_s, minf_e) = find(b, md_s, md_e, b"minf")?;
    let (stbl_s, stbl_e) = find(b, minf_s, minf_e, b"stbl")?;

    let (stts_s, stts_e) = find(b, stbl_s, stbl_e, b"stts")?;
    let (stsz_s, stsz_e) = find(b, stbl_s, stbl_e, b"stsz")?;
    let (stsc_s, stsc_e) = find(b, stbl_s, stbl_e, b"stsc")?;
    // chunk offsets: stco (32-bit) or co64 (64-bit).
    let (co_s, co_e, co64) = match find(b, stbl_s, stbl_e, b"stco") {
        Some((s, e)) => (s, e, false),
        None => { let (s, e) = find(b, stbl_s, stbl_e, b"co64")?; (s, e, true) }
    };

    let sizes = parse_stsz(b, stsz_s, stsz_e)?;
    let n = sizes.len();
    let chunk_offsets = parse_chunk_offsets(b, co_s, co_e, co64)?;
    let spc = parse_stsc(b, stsc_s, stsc_e, chunk_offsets.len())?; // samples-per-chunk, expanded per chunk
    let dts = parse_stts(b, stts_s, stts_e, n)?; // cumulative decode times (in timescale units)
    let ctts = find(b, stbl_s, stbl_e, b"ctts").and_then(|(s, e)| parse_ctts(b, s, e, n));
    let sync = find(b, stbl_s, stbl_e, b"stss").map(|(s, e)| parse_stss(b, s, e));

    // Walk chunks → per-sample absolute offset.
    let mut samples = Vec::with_capacity(n);
    let mut sample_i = 0usize;
    for (chunk_idx, &chunk_off) in chunk_offsets.iter().enumerate() {
        let count = *spc.get(chunk_idx).unwrap_or(&0) as usize;
        let mut off = chunk_off;
        for _ in 0..count {
            if sample_i >= n { break; }
            let size = sizes[sample_i];
            let dts_ticks = dts[sample_i];
            let cts_off = ctts.as_ref().map(|c| c[sample_i]).unwrap_or(0);
            let is_sync = match &sync {
                Some(set) => set.contains(&((sample_i as u32) + 1)),
                None => true, // no stss = every sample is a sync sample
            };
            let dts_us = ticks_to_us(dts_ticks, timescale);
            let pts_us = ticks_to_us(dts_ticks as i64 + cts_off, timescale);
            samples.push(SampleRef { offset: off, size, pts_us, dts_us, is_sync });
            off += size as u64;
            sample_i += 1;
        }
    }
    Some(TrackTable { handler, timescale, samples })
}

fn ticks_to_us<T: Into<i64>>(ticks: T, timescale: u32) -> i64 {
    (ticks.into() * 1_000_000) / timescale as i64
}

/// stsz: [ver/flags:4][sample_size:4][count:4]([entry:4]*). A non-zero
/// sample_size means all samples share it (no entry array).
fn parse_stsz(b: &[u8], s: usize, e: usize) -> Option<Vec<u32>> {
    if s + 12 > e { return None; }
    let default = be32(b, s + 4);
    let count = be32(b, s + 8) as usize;
    if default != 0 {
        return Some(vec![default; count]);
    }
    let mut v = Vec::with_capacity(count);
    let mut i = s + 12;
    for _ in 0..count {
        if i + 4 > e { break; }
        v.push(be32(b, i));
        i += 4;
    }
    Some(v)
}

fn parse_chunk_offsets(b: &[u8], s: usize, e: usize, co64: bool) -> Option<Vec<u64>> {
    if s + 8 > e { return None; }
    let count = be32(b, s + 4) as usize;
    let mut v = Vec::with_capacity(count);
    let mut i = s + 8;
    let step = if co64 { 8 } else { 4 };
    for _ in 0..count {
        if i + step > e { break; }
        v.push(if co64 { be64(b, i) } else { be32(b, i) as u64 });
        i += step;
    }
    Some(v)
}

/// stsc: [ver/flags:4][count:4]([first_chunk:4][samples_per_chunk:4][desc:4]*).
/// Expand to a per-chunk samples-per-chunk vector of length `num_chunks`.
fn parse_stsc(b: &[u8], s: usize, e: usize, num_chunks: usize) -> Option<Vec<u32>> {
    if s + 8 > e { return None; }
    let count = be32(b, s + 4) as usize;
    // Collect (first_chunk (1-based), samples_per_chunk).
    let mut runs: Vec<(u32, u32)> = Vec::with_capacity(count);
    let mut i = s + 8;
    for _ in 0..count {
        if i + 12 > e { break; }
        runs.push((be32(b, i), be32(b, i + 4)));
        i += 12;
    }
    if runs.is_empty() { return None; }
    let mut per_chunk = vec![0u32; num_chunks];
    for (ri, &(first, spc)) in runs.iter().enumerate() {
        let start = first.saturating_sub(1) as usize;
        let end = runs.get(ri + 1).map(|&(f, _)| f.saturating_sub(1) as usize).unwrap_or(num_chunks);
        for c in start..end.min(num_chunks) {
            per_chunk[c] = spc;
        }
    }
    Some(per_chunk)
}

/// stts: cumulative decode timestamps (in timescale units) per sample.
fn parse_stts(b: &[u8], s: usize, e: usize, n: usize) -> Option<Vec<i64>> {
    if s + 8 > e { return None; }
    let count = be32(b, s + 4) as usize;
    let mut out = Vec::with_capacity(n);
    let mut t: i64 = 0;
    let mut i = s + 8;
    for _ in 0..count {
        if i + 8 > e { break; }
        let cnt = be32(b, i);
        let delta = be32(b, i + 4) as i64;
        i += 8;
        for _ in 0..cnt {
            out.push(t);
            t += delta;
            if out.len() >= n { break; }
        }
    }
    // Pad if stts under-counted (defensive).
    while out.len() < n { out.push(t); t += 0; }
    Some(out)
}

/// ctts: per-sample composition offset (PTS = DTS + offset). Version 1 offsets
/// are signed; version 0 unsigned — both fit i64.
fn parse_ctts(b: &[u8], s: usize, e: usize, n: usize) -> Option<Vec<i64>> {
    if s + 8 > e { return None; }
    let version = b[s];
    let count = be32(b, s + 4) as usize;
    let mut out = Vec::with_capacity(n);
    let mut i = s + 8;
    for _ in 0..count {
        if i + 8 > e { break; }
        let cnt = be32(b, i);
        let raw = be32(b, i + 4);
        let off = if version == 1 { raw as i32 as i64 } else { raw as i64 };
        i += 8;
        for _ in 0..cnt {
            out.push(off);
            if out.len() >= n { break; }
        }
    }
    while out.len() < n { out.push(0); }
    Some(out)
}

/// stss: the set of sync (keyframe) sample numbers (1-based).
fn parse_stss(b: &[u8], s: usize, e: usize) -> std::collections::HashSet<u32> {
    let mut set = std::collections::HashSet::new();
    if s + 8 > e { return set; }
    let count = be32(b, s + 4) as usize;
    let mut i = s + 8;
    for _ in 0..count {
        if i + 4 > e { break; }
        set.insert(be32(b, i));
        i += 4;
    }
    set
}

// ---- codec config + Annex-B framing (ported from wandr.video.player) --------

const START: [u8; 4] = [0, 0, 0, 1];

/// `lengthSizeMinusOne`+1 from avcC (h264, payload off 4) or hvcC (h265, off 21).
/// The `mp4` crate parses neither field; feeding the wrong prefix width is silent
/// garbage, so read it from the raw box.
pub fn nal_length_size(file: &[u8], is264: bool) -> Option<usize> {
    let (tag, off): (&[u8], usize) = if is264 { (b"avcC", 4) } else { (b"hvcC", 21) };
    let p = file.windows(4).position(|w| w == tag)?;
    let payload = file.get(p + 4..)?;
    Some((*payload.get(off)? & 0b11) as usize + 1)
}

/// H.265 VPS/SPS/PPS from the hvcC box (ISO 14496-15 §8.3.3.1), start-coded — the
/// `mp4` crate doesn't expose it. A faithful port of the host's `parse_hvcc`.
pub fn parse_hvcc(file: &[u8]) -> Option<Vec<u8>> {
    let p = file.windows(4).position(|w| w == b"hvcC")?;
    let pl = file.get(p + 4..)?;
    let num_arrays = *pl.get(22)? as usize;
    let mut out = Vec::new();
    let mut i = 23;
    for _ in 0..num_arrays {
        let num = u16::from_be_bytes([*pl.get(i + 1)?, *pl.get(i + 2)?]) as usize;
        i += 3;
        for _ in 0..num {
            let l = u16::from_be_bytes([*pl.get(i)?, *pl.get(i + 1)?]) as usize;
            i += 2;
            out.extend_from_slice(&START);
            out.extend_from_slice(pl.get(i..i + l)?);
            i += l;
        }
    }
    Some(out)
}

/// Turn one MP4 sample's length-prefixed NAL bytes into an Annex-B access unit.
/// At a sync sample the parameter-set prefix rides along (the decoder must be
/// able to start there, and after a `reset` it will have forgotten them).
pub fn to_annexb(sample: &[u8], nal_len: usize, ps_prefix: &[u8], is_sync: bool) -> Vec<u8> {
    let mut au = Vec::with_capacity(sample.len() + ps_prefix.len() + 16);
    if is_sync {
        au.extend_from_slice(ps_prefix);
    }
    let mut i = 0usize;
    while i + nal_len <= sample.len() {
        let mut n = 0usize;
        for b in &sample[i..i + nal_len] {
            n = (n << 8) | *b as usize;
        }
        i += nal_len;
        if n == 0 || i + n > sample.len() {
            break;
        }
        au.extend_from_slice(&START);
        au.extend_from_slice(&sample[i..i + n]);
        i += n;
    }
    au
}

/// Extract the AAC AudioSpecificConfig (ASC) from the `esds` box — the
/// DecoderSpecificInfo (descriptor tag 0x05) inside the ES_Descriptor (0x03) →
/// DecoderConfigDescriptor (0x04). This is the authoritative codec config to
/// hand Symphonia's AAC decoder as `extra_data`; reconstructing it from freq/chan
/// indices is lossy, so read the real bytes.
pub fn parse_esds_asc(head: &[u8]) -> Option<Vec<u8>> {
    let p = head.windows(4).position(|w| w == b"esds")?;
    let mut i = p + 4 + 4; // past 'esds' tag + version/flags(4)

    // ES_Descriptor (0x03): ES_ID(2) + flags(1) + optional dependence/url/ocr.
    if *head.get(i)? != 0x03 { return None; }
    i += 1;
    read_desc_len(head, &mut i)?;
    i += 2; // ES_ID
    let flags = *head.get(i)?;
    i += 1;
    if flags & 0x80 != 0 { i += 2; } // streamDependenceFlag → dependsOn_ES_ID
    if flags & 0x40 != 0 { let u = *head.get(i)? as usize; i += 1 + u; } // URL_flag
    if flags & 0x20 != 0 { i += 2; } // OCRstreamFlag

    // DecoderConfigDescriptor (0x04): 13 fixed bytes then the DecSpecificInfo.
    if *head.get(i)? != 0x04 { return None; }
    i += 1;
    read_desc_len(head, &mut i)?;
    i += 13; // objectTypeIndication(1)+streamType(1)+bufferSizeDB(3)+max(4)+avg(4)

    // DecoderSpecificInfo (0x05): the ASC.
    if *head.get(i)? != 0x05 { return None; }
    i += 1;
    let len = read_desc_len(head, &mut i)?;
    Some(head.get(i..i + len)?.to_vec())
}

/// MPEG-4 descriptor expandable length: 7 bits per byte, high bit = continue.
fn read_desc_len(b: &[u8], i: &mut usize) -> Option<usize> {
    let mut len = 0usize;
    for _ in 0..4 {
        let c = *b.get(*i)?;
        *i += 1;
        len = (len << 7) | (c & 0x7f) as usize;
        if c & 0x80 == 0 { break; }
    }
    Some(len)
}

// ---- rolling mdat window ---------------------------------------------------

/// Holds a contiguous fetched slice of the file `[base, base+len)`, so a sample
/// wholly inside it can be sliced without another fetch. Consumed prefix is
/// dropped to bound memory — samples are read in decode order, so once we pass a
/// sample's bytes we never need them again (until a seek, which refills).
pub struct RollingBuffer {
    pub base: u64,
    pub data: Vec<u8>,
}

impl RollingBuffer {
    pub fn new() -> Self { RollingBuffer { base: 0, data: Vec::new() } }

    pub fn end(&self) -> u64 { self.base + self.data.len() as u64 }

    /// Is `[offset, offset+len)` fully buffered?
    pub fn has(&self, offset: u64, len: u32) -> bool {
        offset >= self.base && offset + len as u64 <= self.end()
    }

    pub fn slice(&self, offset: u64, len: u32) -> Option<&[u8]> {
        if !self.has(offset, len) { return None; }
        let start = (offset - self.base) as usize;
        Some(&self.data[start..start + len as usize])
    }

    /// Append bytes fetched contiguously at the current end.
    pub fn append(&mut self, at: u64, bytes: &[u8]) {
        if self.data.is_empty() {
            self.base = at;
            self.data.extend_from_slice(bytes);
        } else if at == self.end() {
            self.data.extend_from_slice(bytes);
        } else {
            // Non-contiguous (after a seek): reset to the new region.
            self.base = at;
            self.data.clear();
            self.data.extend_from_slice(bytes);
        }
    }

    /// Drop everything before `offset` to reclaim memory.
    pub fn drop_before(&mut self, offset: u64) {
        if offset <= self.base { return; }
        let drop = (offset - self.base).min(self.data.len() as u64) as usize;
        self.data.drain(0..drop);
        self.base += drop as u64;
    }

    /// Reset for a seek to a new byte position.
    pub fn reset_to(&mut self, offset: u64) {
        self.base = offset;
        self.data.clear();
    }
}
