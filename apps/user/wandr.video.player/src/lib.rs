//! wandr.video.player — task 117 M2 stage 2, ON-SCREEN proof.
//!
//! A UI reactor that plays an H.264 clip through `wandr:video`'s playback lane:
//!
//!   submit-timed(PTS) -> next-decoded -> present(at-ns) -> flush(EOS)
//!
//! WHY A UI APP: this began as a `wasi:cli` command, which proved the CONTROL
//! path but showed nothing — `--run-once` guests are surfaceless, so the host
//! stores decoded frames and never runs `composite_video_surfaces`. A reactor has
//! a render loop, which is what actually composites video to the display. The
//! guest itself never sees a pixel (`decoded-frame` is opaque by design), so
//! decode-to-surface is the only visual proof a guest can produce.
//!
//! PUMPED FROM `on-frame`, not a blocking loop: a reactor must return promptly
//! or the whole UI stalls. Each callback submits a bounded amount of input and
//! presents whatever is due — which is how a real player works anyway.
//!
//! The tick arrives on `wasi:input-handlers/frame-handler` — the ONLY interface
//! the host drives UI guests through (`input.rs::dispatch_frame`). The retired
//! `my:skiko-gfx/renderer` this app first exported was never called at all.
//!
//! Input: whichever of `CLIPS` is present, via a read-only `[[mounts]]` entry in
//! package.toml. Demux is GUEST-SIDE by design (proposals/wasi-media-source),
//! so no sample path and no container knowledge is baked into the host.
//!
//! ‼️ TWO CONTAINERS, ON PURPOSE — this is the finding-#6 experiment, not
//! completeness for its own sake. An Annex-B ELEMENTARY STREAM CARRIES NO
//! TIMESTAMPS: the best a player can do is synthesise one from bitstream
//! position, which is a DTS, and on a B-frame stream that is simply the wrong
//! quantity. An MP4 carries a real per-sample presentation time (DTS + ctts).
//! Feeding the same content both ways is what distinguishes a decoder that
//! carries timestamps through unchanged from one that reorders them, which is
//! exactly where the vaapi and openh264 backends disagree. The player REPORTS
//! which kind it got, so a run is self-describing.

wit_bindgen::generate!({
    world: "wandr:video-player-app/video-player-app",
    path: "wit",
    generate_all,
});

use crate::exports::wandr::ui_shell::frame_pacing::Guest as PacingGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::key_handler::{Guest as KeyGuest, KeyEvent};
use crate::wasi::video_codec::decoder::VideoDecoder;
use crate::wasi::video_codec::types::{Acceleration, Codec, CodecError, DecoderConfig, EncodedChunk};
use crate::wandr::video::present::VideoSurface;
use crate::wandr::video::types::{VideoRect, ZLayer};
use crate::wandr::video_diag::diag as diag_api;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::types as wtypes;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::draw::Canvas;
use std::cell::RefCell;

/// Tried in order; the first that opens wins. MP4 first because it carries real
/// presentation times — swap the order to run the elementary-stream half of the
/// finding-#6 comparison against identical content.
const CLIPS: &[&str] = &["/samples/bbb-h265.mp4", "/samples/bbb-h264.mp4", "/samples/bbb.h264"];
/// Fallback frame rate, used ONLY to synthesise timestamps for an elementary
/// stream, which has none. An MP4 tells us the real ones and this is unused.
const FPS: i64 = 30;
/// H.264 allows up to 16 frames in the DPB (Annex A, MaxDpbMbs), so a conformant
/// decoder may legally consume 17 access units before it emits its first picture.
const MAX_H264_DPB: usize = 16;
/// Decode-ahead cushion, in frames — how far ahead of what we have SHOWN we are
/// willing to feed. Unbounded would decode a whole file into host memory before
/// showing frame one.
///
/// ‼️ IT MUST EXCEED THE DECODER'S REORDER DEPTH OR PLAYBACK DEADLOCKS, and this
/// was set to 8 for exactly that reason — too small. openh264 emits early enough
/// that 8 worked; the VA-API decoder holds more, so the guest fed 8, waited for
/// output that could only come from more input, and stalled forever at
/// `presented 0` while the pump ticked 1000+ times. Nothing errored.
///
/// The deeper problem is a CONTRACT gap, not a constant: a guest cannot ask how
/// deep a decoder's pipeline is, and the host never reports `queue-full` (stage-1
/// finding #2), so there is no back-pressure signal to feed against either. Until
/// one of those exists, the only safe cushion is the codec's SPEC maximum — which
/// is at least derived rather than guessed.
const DECODE_AHEAD: usize = MAX_H264_DPB + 4;
/// Cap the work done per render-frame so a reactor callback never runs long.
/// Submits allowed per `on_frame`. Must let ONE on_frame fill the whole
/// decode-ahead cushion, or playback cannot get a lead: a player that only
/// submits a few AUs per callback stays behind realtime, so every frame's
/// deadline is already past and gets presented immediately rather than SCHEDULED
/// — and with nothing scheduled, the host has no video wake source and the video
/// runs at the guest's idle UI rate instead of the frame rate. `DECODE_AHEAD`
/// (with the host's in-flight cap) is the real bound; this just must not be
/// smaller than it.
const MAX_SUBMITS_PER_FRAME: usize = DECODE_AHEAD;

/// One access unit, ready to submit: Annex-B bytes, its presentation time in
/// microseconds, and whether it can start a decode.
struct Au {
    data: Vec<u8>,
    pts_us: i64,
    keyframe: bool,
}

/// Where an access unit's timestamp came from. Worth carrying rather than
/// assuming: a synthesised timestamp is a DTS wearing a PTS's clothes, and every
/// ordering question downstream depends on knowing which one you have.
#[derive(Clone, Copy, PartialEq)]
enum Timing {
    /// Real presentation times, from the container's sample table.
    Container,
    /// Synthesised from bitstream position at `FPS` — an elementary stream has
    /// nothing better to offer.
    SynthesizedDts,
}

impl Timing {
    fn label(self) -> &'static str {
        match self {
            Timing::Container => "container PTS (real)",
            Timing::SynthesizedDts => "synthesised from bitstream order (a DTS)",
        }
    }
}

/// `{:?}` on the WIT enum renders `Codec::H265`; this is the name a person reads.
fn codec_name(c: Codec) -> &'static str {
    match c {
        Codec::Vp8 => "vp8",
        Codec::Vp9 => "vp9",
        Codec::H264 => "h264",
        Codec::H265 => "h265",
        Codec::Av1 => "av1",
    }
}

fn synth_pts_us(index: i64) -> i64 {
    index * 1_000_000 / FPS
}

/// Demux by CONTENT, not by file extension: an `ftyp` box at offset 4 is the
/// ISO-BMFF signature. A player that trusts the name is a player that crashes on
/// a mislabelled file.
fn demux(bytes: &[u8]) -> Result<(Vec<Au>, Timing, Codec), String> {
    if bytes.len() > 8 && &bytes[4..8] == b"ftyp" {
        demux_mp4(bytes).map(|(aus, codec)| (aus, Timing::Container, codec))
    } else {
        // Raw Annex-B elementary stream — only our H.264 test clip takes this path.
        Ok((
            access_units(bytes)
                .into_iter()
                .enumerate()
                .map(|(i, (data, keyframe))| Au { data, pts_us: synth_pts_us(i as i64), keyframe })
                .collect(),
            Timing::SynthesizedDts,
            Codec::H264,
        ))
    }
}

const START: [u8; 4] = [0, 0, 0, 1];

/// MP4 (ISO-BMFF) → Annex-B access units with REAL presentation times.
///
/// Three things a container gives us that an elementary stream cannot:
///   * sample boundaries — no need to infer AUs by scanning for VCL NALs
///   * `is_sync` — which samples are true random-access points
///   * PTS = DTS + composition offset (`ctts`). That offset is precisely the
///     B-frame reordering delta, so this is the only way to get a presentation
///     time that is actually ascending in display order.
fn demux_mp4(bytes: &[u8]) -> Result<(Vec<Au>, Codec), String> {
    let size = bytes.len() as u64;
    let mut mp4 = mp4::Mp4Reader::read_header(std::io::Cursor::new(bytes), size)
        .map_err(|e| format!("mp4 header: {e}"))?;

    // The keyframe parameter-set prefix, ALREADY start-coded (Annex-B). For H.264
    // it is SPS+PPS from avcC's typed accessors; for H.265 it is VPS+SPS+PPS from
    // the raw hvcC box, which the `mp4` crate does not expose (mirrors the host's
    // parse_hvcc). Both codecs prepend this blob at every sync sample.
    let (tid, timescale, codec, ps_prefix, nal_len, count) = {
        let t = mp4
            .tracks()
            .values()
            .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H264 | mp4::MediaType::H265)))
            .ok_or("no H.264/H.265 track")?;
        let is264 = matches!(t.media_type(), Ok(mp4::MediaType::H264));
        let ps_prefix = if is264 {
            let mut p = Vec::new();
            p.extend_from_slice(&START);
            p.extend_from_slice(&t.sequence_parameter_set().map_err(|e| format!("sps: {e}"))?);
            p.extend_from_slice(&START);
            p.extend_from_slice(&t.picture_parameter_set().map_err(|e| format!("pps: {e}"))?);
            p
        } else {
            parse_hvcc(bytes).ok_or("could not parse hvcC")?
        };
        (
            t.track_id(),
            t.timescale() as i64,
            if is264 { Codec::H264 } else { Codec::H265 },
            ps_prefix,
            // The NAL length prefix is 1..4 bytes, declared per file in avcC/hvcC's
            // lengthSizeMinusOne. Almost always 4 — but "almost always" is how you
            // get a decoder fed garbage on the one file that differs, so read it.
            nal_length_size(bytes, is264).unwrap_or(4),
            t.sample_count(),
        )
    };
    if timescale <= 0 {
        return Err("track timescale is zero".into());
    }

    let mut out = Vec::with_capacity(count as usize);
    for sid in 1..=count {
        let s = mp4
            .read_sample(tid, sid)
            .map_err(|e| format!("sample {sid}: {e}"))?
            .ok_or_else(|| format!("sample {sid} missing"))?;
        // PTS, not DTS: start_time is the decode time, rendering_offset is ctts.
        let pts_us = (s.start_time as i64 + s.rendering_offset as i64) * 1_000_000 / timescale;

        let mut au = Vec::with_capacity(s.bytes.len() + 64);
        // Parameter sets ride with every sync sample: the decoder must be able to
        // start from any of them, and after a `reset` it will have forgotten them.
        if s.is_sync {
            au.extend_from_slice(&ps_prefix);
        }
        let mut i = 0usize;
        while i + nal_len <= s.bytes.len() {
            let mut n = 0usize;
            for b in &s.bytes[i..i + nal_len] {
                n = (n << 8) | *b as usize;
            }
            i += nal_len;
            if n == 0 || i + n > s.bytes.len() {
                break;
            }
            au.extend_from_slice(&START);
            au.extend_from_slice(&s.bytes[i..i + n]);
            i += n;
        }
        out.push(Au { data: au, pts_us, keyframe: s.is_sync });
    }
    Ok((out, codec))
}

/// `lengthSizeMinusOne` (low 2 bits, +1) from avcC (H.264, payload offset 4) or
/// hvcC (H.265, payload offset 21). The `mp4` crate parses neither field, so read
/// it from the raw box; feeding a decoder the wrong prefix width is silent garbage.
fn nal_length_size(file: &[u8], is264: bool) -> Option<usize> {
    let (tag, off): (&[u8], usize) = if is264 { (b"avcC", 4) } else { (b"hvcC", 21) };
    let p = file.windows(4).position(|w| w == tag)?;
    let payload = file.get(p + 4..)?;
    Some((*payload.get(off)? & 0b11) as usize + 1)
}

/// H.265 param sets live in the hvcC box (ISO 14496-15 §8.3.3.1), which the `mp4`
/// crate does not expose. Parse the raw box for the start-coded VPS/SPS/PPS blob —
/// a faithful port of the host's `parse_hvcc` (runtime/wandr-host/src/lib.rs).
/// Layout after the "hvcC" tag: 22 fixed bytes, then numOfArrays at 22, then each
/// array is {type(1), numNalus(2)} followed by numNalus × {len(2), nalu}.
fn parse_hvcc(file: &[u8]) -> Option<Vec<u8>> {
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

/// Split Annex-B into ACCESS UNITS (one coded picture each): a new AU begins at a
/// VCL NAL when the current AU already holds one. Feeding NALs individually would
/// give several of them the same PTS and make presentation timing meaningless.
fn access_units(buf: &[u8]) -> Vec<(Vec<u8>, bool)> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            let sc = if i > 0 && buf[i - 1] == 0 { i - 1 } else { i };
            starts.push((sc, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let (mut aus, mut cur) = (Vec::new(), Vec::<u8>::new());
    let (mut has_vcl, mut idr) = (false, false);
    for (n, &(sc, payload)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map(|&(s, _)| s).unwrap_or(buf.len());
        let t = buf[payload] & 0x1f;
        let is_vcl = t == 1 || t == 5;
        if is_vcl && has_vcl {
            aus.push((std::mem::take(&mut cur), idr));
            has_vcl = false;
            idr = false;
        }
        cur.extend_from_slice(&buf[sc..end]);
        if is_vcl {
            has_vcl = true;
            idr |= t == 5;
        }
    }
    if !cur.is_empty() {
        aus.push((cur, idr));
    }
    aus
}

/// One subtitle cue: an inclusive-start/exclusive-end window in microseconds and
/// the text to show. Media-timeline µs, same clock as the video PTS.
struct Cue {
    start_us: i64,
    end_us: i64,
    text: String,
}

/// Parse SubRip (.srt). Deliberately tiny — SRT is index / `HH:MM:SS,mmm -->
/// HH:MM:SS,mmm` / text lines / blank — and a real player would use a subtitle
/// crate, but the point here is the COMPOSITING, not the format zoo. Malformed
/// blocks are skipped, not fatal: a bad cue should lose one line, not playback.
///
/// FUTURE — richer formats. SRT (and WebVTT, which is the same shape plus inline
/// styling) is enough for plain captions and covers the common case. ASS/SSA —
/// positioned, styled, animated "signs" subtitles — is deliberately OUT OF SCOPE:
/// it is a whole layout engine, and the production answer is libass via FFI
/// (mpv/VLC/Kodi all bind it; the one pure-Rust attempt, subrandr, does WebVTT
/// only and its own README says "do not use from Rust"). Nothing in the pipeline
/// blocks it — captions already render in the guest UI layer over `presented-rect`
/// at display resolution, so an ASS renderer would be a drop-in producer of the
/// same styled runs. It is added when a real ASS need appears, not before.
fn parse_srt(text: &str) -> Vec<Cue> {
    fn ts(s: &str) -> Option<i64> {
        // HH:MM:SS,mmm
        let (hms, ms) = s.trim().split_once(',')?;
        let mut it = hms.split(':');
        let h: i64 = it.next()?.trim().parse().ok()?;
        let m: i64 = it.next()?.parse().ok()?;
        let sec: i64 = it.next()?.parse().ok()?;
        let ms: i64 = ms.parse().ok()?;
        Some(((h * 3600 + m * 60 + sec) * 1000 + ms) * 1000)
    }
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let mut lines = block.lines().filter(|l| !l.trim().is_empty());
        let Some(first) = lines.next() else { continue };
        // The index line is optional in the wild; if the first line is the time
        // range, use it, else the second line is.
        let timing = if first.contains("-->") { first } else { lines.next().unwrap_or("") };
        let Some((a, b)) = timing.split_once("-->") else { continue };
        let (Some(start_us), Some(end_us)) = (ts(a), ts(b)) else { continue };
        let body: Vec<&str> = lines.collect();
        if body.is_empty() {
            continue;
        }
        cues.push(Cue { start_us, end_us, text: body.join("\n") });
    }
    cues.sort_by_key(|c| c.start_us);
    cues
}

struct Player {
    dec: Option<VideoDecoder>,
    /// The embedder-owned compositing surface (task 120 split): the codec decoder
    /// is surface-free; `present(frame, at-ns)` schedules onto THIS.
    surf: Option<VideoSurface>,
    aus: Vec<Au>,
    /// Whether `aus` carry real presentation times or synthesised ones.
    timing: Timing,
    next_au: usize,
    submitted: usize,
    presented: usize,
    /// Host clock (from `render-frame(nanos)`) at the moment playback began —
    /// our zero point for turning a PTS into "is this frame due yet?".
    origin_ns: u64,
    started: bool,
    flushed: bool,
    done: bool,
    last_pts: i64,
    out_of_order: usize,
    /// How many times the host said `queue-full`. Proof the back-pressure
    /// conversation is actually happening rather than the timing merely looking
    /// right for some other reason.
    queue_full_hits: u64,
    /// PTS of the first frame, subtracted so playback starts immediately.
    /// Container PTS need not start at zero — this clip's does not (MP4 `ctts`
    /// offsets are non-negative, so the whole timeline is shifted by the reorder
    /// depth: the first frame is at 66666 us, not 0).
    first_pts_us: Option<i64>,
    /// Subtitle cues (sidecar .srt), sorted by start. Empty if none loaded.
    /// Rendered GUEST-SIDE at display resolution over the video's placed rect —
    /// which is where every mature player puts them (never baked into the frame).
    cues: Vec<Cue>,
    /// Surface geometry, learned from on-resize.
    surface: (u32, u32),
    /// Which acceleration the next decoder open should ask for. Switched live by
    /// the H/S/N keys — the guest-side counterpart of the host's
    /// WANDR_VIDEO_BACKEND knob, and the only way to A/B hardware against
    /// software without a rebuild.
    accel: Acceleration,
    /// Set by a key press; the next pump tears the decoder down and starts over.
    restart: bool,
    /// How many times `on-frame` has driven us. A player that stalls needs to be
    /// able to say WHICH thing stopped — the ticks, the decoder, or the clock —
    /// and without this the three are indistinguishable from the outside.
    pumps: u64,
    /// Codec of the loaded clip, learned from the container in `demux` and used to
    /// open the decoder (H.264/H.265 today; both decode on the desktop host).
    codec: Codec,
    /// Clip pinned by the 4/5 keys (`bbb-h264.mp4` / `bbb-h265.mp4`), so you can
    /// A/B all SW/HW × H.264/H.265 combos live. `None` = the CLIPS default order.
    clip: Option<&'static str>,
}

/// The decode-to-surface rect for a `w`×`h` surface: a 16:9 letterbox centered
/// vertically, full width. One source of truth so `start()` (initial open) and
/// `on_resize` (live reconcile) place the video identically.
fn video_rect(w: u32, h: u32) -> VideoRect {
    let vh = (w * 9 / 16).min(h);
    VideoRect { x: 0, y: (h.saturating_sub(vh)) / 2, width: w, height: vh }
}

impl Player {
    const fn new() -> Self {
        Self {
            dec: None,
            surf: None,
            aus: Vec::new(),
            timing: Timing::SynthesizedDts,
            next_au: 0,
            submitted: 0,
            presented: 0,
            origin_ns: 0,
            started: false,
            flushed: false,
            done: false,
            last_pts: -1,
            out_of_order: 0,
            queue_full_hits: 0,
            first_pts_us: None,
            cues: Vec::new(),
            surface: (1280, 720),
            accel: Acceleration::NoPreference,
            restart: false,
            pumps: 0,
            codec: Codec::H264,
            clip: None,
        }
    }

    fn start(&mut self, nanos: u64) {
        self.started = true;
        // Provisional — re-anchored to the first frame's emergence in `pump` once
        // the decoder's reorder/startup latency has passed (see the note there).
        self.origin_ns = nanos;
        // A 4/5 keypress pins one clip; otherwise the first readable CLIPS entry
        // wins. A missing clip is not an error, a missing mount is.
        let candidates: Vec<&str> = match self.clip {
            Some(c) => vec![c],
            None => CLIPS.to_vec(),
        };
        let Some((path, buf)) = candidates.iter().find_map(|p| std::fs::read(p).ok().map(|b| (*p, b)))
        else {
            println!("player: none of {candidates:?} could be read — is the /samples mount resolving?");
            self.done = true;
            return;
        };
        match demux(&buf) {
            Ok((aus, timing, codec)) => {
                self.aus = aus;
                self.timing = timing;
                self.codec = codec;
            }
            Err(e) => {
                println!("player: demux {path} failed: {e}");
                self.done = true;
                return;
            }
        }
        // Sidecar subtitles: same basename, .srt. Optional — a missing file is not
        // an error, just a video with no captions.
        let srt_path = format!("{}.srt", path.rsplit_once('.').map(|(b, _)| b).unwrap_or(path));
        // The clip is /samples/bbb-h264.mp4 but the sidecar is /samples/bbb.srt,
        // so also try the directory's bbb.srt.
        for candidate in [srt_path.as_str(), "/samples/bbb.srt"] {
            if let Ok(txt) = std::fs::read_to_string(candidate) {
                self.cues = parse_srt(&txt);
                println!("player: subtitles {} — {} cues", candidate, self.cues.len());
                break;
            }
        }

        let span_ms = self.aus.last().map(|a| a.pts_us / 1000).unwrap_or(0);
        println!(
            "player: {path} — {} bytes -> {} access units, timing = {}, last PTS {} ms",
            buf.len(),
            self.aus.len(),
            self.timing.label(),
            span_ms
        );

        // What can this MACHINE actually decode? Probed host-side, so an empty
        // list for a codec means "not decodable here" — not "not built in".
        let available = diag_api::list_decoders();
        if available.is_empty() {
            println!("player: host reports NO enumerable decoders");
        } else {
            println!("player: decoders available here:");
            for d in &available {
                println!(
                    "player:   {:<5} {:<10} {}",
                    codec_name(d.codec),
                    d.name,
                    if d.hardware { "HARDWARE" } else { "software" }
                );
            }
        }
        println!("player: keys — [h] hardware  [s] software  [n] no-preference  [4] bbb-h264  [5] bbb-h265  (restarts playback)");

        // Decode-to-surface: a REAL rect is what makes the host composite video.
        // above-ui = the video covers its rect and the UI lays out around it, so
        // we don't need the transparent-hole dance behind-ui requires.
        let (w, h) = self.surface;
        let rect = video_rect(w, h);
        let vh = rect.height;
        // Task 120: the CODEC (surface-free) and the SURFACE (placement) are now
        // separate resources. `decoder-config` carries only codec + acceleration —
        // no width/height (the stream overrides them) and no rect/layer (those move
        // to `video-surface`). `None` keys = cleartext (no DRM).
        let dec = match VideoDecoder::open(
            DecoderConfig { codec: self.codec, acceleration: self.accel },
            None,
        ) {
            Ok(d) => d,
            Err(e) => {
                println!("player: decoder open FAILED: {e:?}");
                self.done = true;
                return;
            }
        };
        // Report what we GOT, not what we asked for (diag): `prefer-*` is a hint, so
        // a request for hardware can be quietly served by software — the difference
        // is exactly what we are here to measure.
        let got = diag_api::implementation(&dec);
        // behind-ui: the video sits UNDER our UI, so the caption bar we draw below
        // lands on top of the picture. This is what a real player wants. (Needs the
        // app EGL surface to carry alpha + sf_set_opaque(false) so the transparent
        // hole reveals the video — see egl.rs EGL_ALPHA_SIZE.)
        let surf = match VideoSurface::open(rect, ZLayer::BehindUi, 0) {
            Ok(s) => s,
            Err(e) => {
                println!("player: surface open FAILED: {e:?}");
                self.done = true;
                return;
            }
        };
        println!(
            "player: decoder OPEN ✓ decode-to-surface {w}x{vh} — asked {:?}, got {} ({})",
            self.accel,
            got.name,
            if got.hardware { "HARDWARE" } else { "software" }
        );
        self.dec = Some(dec);
        self.surf = Some(surf);
    }

    /// Tear down and replay from the top with the currently selected
    /// acceleration. Dropping the decoder + surface is what releases the host's
    /// compositing surface and any frames it still holds — there is no
    /// reopen-in-place verb, and `reset` would keep the SAME backend, which is
    /// precisely what we want to change.
    fn restart(&mut self, nanos: u64) {
        self.dec = None;
        self.surf = None;
        self.first_pts_us = None;
        self.next_au = 0;
        self.submitted = 0;
        self.presented = 0;
        self.last_pts = -1;
        self.out_of_order = 0;
        self.flushed = false;
        self.done = false;
        self.restart = false;
        println!("player: restarting with {:?}", self.accel);
        // `start` re-reads and re-demuxes; wasteful, but this is a diagnostic
        // path and correctness beats cleverness in one.
        self.start(nanos);
    }

    /// One pump step. Submits a bounded amount, then presents everything due.
    fn pump(&mut self, nanos: u64) {
        if self.restart {
            self.restart(nanos);
        }
        let Some(dec) = self.dec.as_ref() else { return };
        let Some(surf) = self.surf.as_ref() else { return };
        // `nanos` is sampled by the host at the TOP of this render-frame, but the
        // first pump then decodes the whole reorder window synchronously before a
        // frame surfaces (HW decode ~28 ms/frame × ~16 ≈ 0.4 s), so `nanos` is
        // stale by that much when we anchor the clock below. `Instant` here shares
        // the wasi:clocks monotonic timeline with `nanos`, so its elapsed measures
        // exactly that staleness and lets us anchor to the REAL now-at-first-frame.
        let pump_t0 = std::time::Instant::now();
        self.pumps += 1;
        if self.pumps % 120 == 0 && !self.done {
            println!(
                "player: pump {} — clock {} ms, fed {}/{}, decoded-out {}, presented {}, held {}",
                self.pumps,
                (nanos.saturating_sub(self.origin_ns) / 1_000_000) as i64,
                self.next_au,
                self.aus.len(),
                self.submitted,
                self.presented,
0u8,
            );
        }

        // Feed, keeping only a bounded cushion ahead of what we've shown.
        let mut submits = 0;
        while self.next_au < self.aus.len()
            && submits < MAX_SUBMITS_PER_FRAME
            && self.submitted < self.presented + DECODE_AHEAD
        {
            let au = &self.aus[self.next_au];
            let chunk = EncodedChunk {
                data: au.data.clone(),
                timestamp_us: au.pts_us,
                keyframe: au.keyframe,
                decrypt: None,
            };
            match dec.submit(&chunk) {
                Ok(()) => {
                    self.submitted += 1;
                    self.next_au += 1;
                    submits += 1;
                }
                // Back-pressure, NOT loss: retry this same frame next pump. A file
                // player cannot resync at the next keyframe the way RTP can.
                Err(CodecError::QueueFull) => {
                    self.queue_full_hits += 1;
                    break;
                }
                Err(e) => {
                    println!("player: submit failed at AU {}: {e:?}", self.next_au);
                    self.done = true;
                    return;
                }
            }
        }

        // EOS: without flush, the frames a codec holds back for reordering are
        // never released and the tail of the clip silently disappears.
        if self.next_au >= self.aus.len() && !self.flushed {
            let _ = dec.flush();
            self.flushed = true;
        }

        // Hand every decoded frame to the host WITH ITS DEADLINE and let the host
        // show it on time. This is what `present(at-ns)` was always for, and it
        // only became usable once the host had ONE timeline (task 117 step 0):
        // `nanos` here, `wasi:clocks`, and the host's presentation scheduler now
        // read the same CLOCK_MONOTONIC, so an instant we name is one the host
        // understands. Before that they were three unrelated origins and a
        // deadline landed ~55 years out, silently.
        //
        // The guest therefore no longer holds a frame or self-paces. Decode stays
        // bounded because `presented` counts hand-offs and the DECODE_AHEAD gate
        // is measured against it.
        while let Some(frame) = dec.next_decoded() {
            let pts = frame.timestamp_us();
            if pts <= self.last_pts {
                self.out_of_order += 1;
            }
            self.last_pts = pts;
            // Anchor the playback clock to the FIRST frame's actual emergence, not
            // to decode-start. The HW decoder holds a reorder window (VideoToolbox
            // ~16 frames) and decode has latency, so the first frame surfaces
            // hundreds of ms after we begin submitting. If `origin_ns` were pinned
            // at decode-start (as `start()` provisionally does), every deadline
            // would be born already in the past — measured at -456 ms and GROWING,
            // because a host that has no future frame scheduled idles its render
            // loop at the guest UI rate (~5 Hz), which pumps us slower still, so we
            // fall further behind: the vicious cycle that made playback choppy on
            // macOS while Linux (shorter reorder path) stayed smooth. Re-anchoring
            // at first output turns that startup latency into a one-time delay;
            // steady-state frames then land in the FUTURE, so `present(at-ns)`
            // schedules them and the host wakes at the frame rate. This is why
            // mpv/ffplay start the master clock at first output, not at open.
            let first = match self.first_pts_us {
                Some(f) => f,
                None => {
                    self.first_pts_us = Some(pts);
                    self.origin_ns = nanos + pump_t0.elapsed().as_nanos() as u64;
                    pts
                }
            };
            let at_ns = self.origin_ns.saturating_add(((pts - first).max(0) as u64) * 1_000);
            // Present through the embedder surface (consumes the frame). The codec
            // decoder is surface-free; the surface owns compositing + scheduling.
            surf.present(frame, at_ns);
            self.presented += 1;
        }

        if self.flushed && self.next_au >= self.aus.len() && !self.done {
            // Everything fed and drained.
            if self.presented >= self.aus.len() {
                self.done = true;
                // Wall time vs clip duration is the pacing proof: a run that
                // finishes far short of the clip's length did not PLAY it, it
                // raced through it.
                // Clip length comes from the LAST PTS, not from a frame count and
                // an assumed rate — with container timing the file is the
                // authority on how long it runs.
                let elapsed_ms = (nanos.saturating_sub(self.origin_ns) / 1_000_000) as i64;
                let clip_ms = self.aus.iter().map(|a| a.pts_us).max().unwrap_or(0) / 1000;
                println!(
                    "player: DONE — presented {}/{} frames, wall {} ms vs clip {} ms",
                    self.presented,
                    self.aus.len(),
                    elapsed_ms,
                    clip_ms,
                );
                // ‼️ THE FINDING-#6 MEASUREMENT. `out-of-order` counts frames whose
                // PTS did not advance. It is only a DEFECT when the timestamps fed
                // in were real presentation times: with container PTS a correct
                // decoder must hand frames back in ascending order, so anything
                // above zero means the decoder reordered or mispaired them. With
                // synthesised bitstream-order tags a non-zero count is EXPECTED on
                // a B-frame stream and says nothing about the decoder — which is
                // exactly why the two runs are worth comparing.
                println!(
                    "player: back-pressure — host said queue-full {} times \
                     (0 = the host never pushed back; decode ran unbounded)",
                    self.queue_full_hits
                );
                // on_frame ran `pumps` times over `elapsed_ms`. We ASKED the host
                // for a 5 Hz UI cadence, so if on_frame ran at ~30 Hz the host
                // woke us for VIDEO independently — the decoupling works. ~5 Hz
                // would mean it did not and playback stuttered.
                let hz = if elapsed_ms > 0 { self.pumps as f64 * 1000.0 / elapsed_ms as f64 } else { 0.0 };
                println!(
                    "player: on_frame ran {} times in {} ms = {:.0} Hz \
                     (asked host for 5 Hz UI; ~30 Hz here means video woke us independently)",
                    self.pumps, elapsed_ms, hz
                );
                println!(
                    "player: timing = {} | out-of-order {} ({})",
                    self.timing.label(),
                    self.out_of_order,
                    match (self.timing, self.out_of_order) {
                        (Timing::Container, 0) => "0 = display order OK",
                        (Timing::Container, _) =>
                            "‼️ NON-ZERO WITH REAL PTS — decoder reordered or mispaired timestamps",
                        (Timing::SynthesizedDts, 0) =>
                            "0, but the tags were DTS-shaped — this says the decoder REORDERED them to ascending",
                        (Timing::SynthesizedDts, _) =>
                            "expected: DTS-shaped tags on a B-frame stream come back permuted",
                    }
                );
            }
        }
    }
}

thread_local! {
    static PLAYER: RefCell<Player> = const { RefCell::new(Player::new()) };
    static WCTX: RefCell<Option<wembed::CanvasContext>> = const { RefCell::new(None) };
}

/// The canvas context, created once and kept — `get-context` is the embedding
/// handshake, not a per-frame call.
/// A solid ARGB fill — the caption bar / progress line drawn over the video.
fn bar_paint(argb: u32) -> wtypes::Paint<'static> {
    wtypes::Paint {
        style: wtypes::PaintStyle::Fill,
        color: argb,
        alpha: (argb >> 24) as u8,
        blend: wtypes::BlendMode::SrcOver,
        anti_alias: true,
        shader: None,
        stroke_width: 0.0,
        stroke_cap: wtypes::StrokeCap::Butt,
        stroke_join: wtypes::StrokeJoin::Miter,
        stroke_miter: 4.0,
        blur: None,
        filter: None,
    }
}

/// Draw one subtitle cue centered along the bottom of the video rect.
///
/// This is the whole subtitle pipeline the compositing work was for: text laid
/// out GUEST-SIDE with host-owned fonts (`wasi:canvas/layout`), at DISPLAY
/// resolution, over the video's placed rect. A drop shadow keeps it legible over
/// bright or busy frames — the black-outline trick every player uses, done here
/// with a shadow because that is what the contract exposes. Multi-line cues wrap
/// and each line centers.
fn draw_subtitle(cv: &Canvas, text: &str, vx: f32, vy: f32, vw: f32, vh: f32) {
    // Size relative to the VIDEO height (media3's DEFAULT_TEXT_SIZE_FRACTION is
    // 0.0533 of the view; a touch smaller reads well here).
    let size = (vh * 0.05).clamp(16.0, 48.0);
    let style = wlayout::TextStyle {
        family: "sans-serif".into(),
        size,
        weight: 600,
        italic: false,
        color: 0xFFFFFFFF,
        letter_spacing: 0.0,
        line_height: 1.15,
        baseline_shift: 0.0,
        decoration: None,
        // Soft black shadow = the legibility outline, in the one primitive the
        // layout contract gives us.
        shadows: vec![wlayout::TextShadow {
            color: 0xE0000000,
            offset: wtypes::Point { x: 0.0, y: 0.0 },
            sigma: size * 0.14,
        }],
        background: None,
    };
    let b = wlayout::ParagraphBuilder::new(&style);
    b.set_align(wlayout::Align::Center);
    b.add_text(text);
    let para = wlayout::ParagraphBuilder::build(b);
    // Lay out to ~90% of the video width, so long lines wrap inside the picture.
    let wrap_w = vw * 0.9;
    para.layout(wrap_w);
    // Baseline near the bottom of the video rect, with a margin. Multi-line cues
    // grow upward from there.
    let margin = vh * 0.06;
    let x = vx + (vw - wrap_w) * 0.5;
    let y = vy + vh - margin - para.height();
    para.paint(cv, wtypes::Point { x, y });
}

fn wctx<R>(f: impl FnOnce(&wembed::CanvasContext) -> R) -> R {
    WCTX.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(wembed::get_context());
        }
        f(c.borrow().as_ref().unwrap())
    })
}

struct Component;

impl FrameGuest for Component {
    fn on_frame(nanos: u64) {
        PLAYER.with(|p| {
            let mut p = p.borrow_mut();
            if !p.started {
                p.start(nanos);
            }
            p.pump(nanos);
        });

        // behind-ui: clear TRANSPARENT so the host's video shows through where we
        // do not paint, then draw SUBTITLES over the picture. Guest-side, at
        // display resolution, positioned against the video's PLACED rect
        // (presented-rect) — exactly what mpv/VLC/media3 do, and the whole reason
        // for the behind-ui + presented-rect work. The decoded pixels never
        // reach us; the captions never reach the video surface.
        let cv = wctx(|x| x.get_current_buffer());
        cv.clear(0x0000_0000);

        let (sw, sh) = PLAYER.with(|p| p.borrow().surface);
        // Placed video rect if the host has one yet, else the whole surface.
        let (clock_us, text, (vx, vy, vw, vh)) = PLAYER.with(|p| {
            let p = p.borrow();
            let clock_us = (nanos.saturating_sub(p.origin_ns) / 1_000) as i64;
            let rect = p
                .surf
                .as_ref()
                .and_then(|s| s.presented_rect())
                .map(|r| (r.x as f32, r.y as f32, r.width as f32, r.height as f32))
                .unwrap_or((0.0, 0.0, sw as f32, sh as f32));
            // The active cue for `clock_us`. Cues are short and few; a linear
            // scan is right, and a binary search would be premature.
            let text = p
                .cues
                .iter()
                .find(|c| clock_us >= c.start_us && clock_us < c.end_us)
                .map(|c| c.text.clone());
            (clock_us, text, rect)
        });
        let _ = clock_us;

        if let Some(text) = text {
            draw_subtitle(&cv, &text, vx, vy, vw, vh);
        }
        drop(cv);
        wctx(|x| x.present());
    }

    /// The host tells us the surface size here — this is how we learn the
    /// geometry without importing a canvas interface.
    fn on_resize(w: u32, h: u32) {
        let (w, h) = (w.max(1), h.max(1));
        PLAYER.with(|p| {
            let mut p = p.borrow_mut();
            if p.surface == (w, h) {
                return;
            }
            p.surface = (w, h);
            // Reconcile a LIVE decoder's surface to the new geometry instead of
            // waiting for a restart: `start()` fixed the rect from whatever
            // `surface` was at open (a placeholder until the first resize), so a
            // resize/rotation would otherwise leave the video mis-placed until
            // the next replay. `set_rect` is exactly the live-reconcile verb.
            if let Some(surf) = p.surf.as_ref() {
                surf.set_rect(video_rect(w, h));
            }
        });
    }
}

impl KeyGuest for Component {
    /// H/S/N pick the acceleration; 4/5 pick the clip (bbb-h264 / bbb-h265) — so
    /// every SW/HW × H.264/H.265 combo is one keypress away. All replay from the
    /// top. Down-edge only — a key repeat would restart continuously while held.
    fn on_key(ev: KeyEvent) {
        if !ev.down || ev.repeat {
            return;
        }
        PLAYER.with(|p| {
            let mut p = p.borrow_mut();
            match ev.text.to_ascii_lowercase().as_str() {
                "h" => p.accel = Acceleration::PreferHardware,
                "s" => p.accel = Acceleration::PreferSoftware,
                "n" => p.accel = Acceleration::NoPreference,
                "4" => p.clip = Some("/samples/bbb-h264.mp4"),
                "5" => p.clip = Some("/samples/bbb-h265.mp4"),
                _ => return, // ignore other keys (no replay)
            }
            p.restart = true;
        });
    }
}

impl PacingGuest for Component {
    /// Drive at the clip's frame rate while playing; idle afterwards. On-demand
    /// pacing is the host's contract — asking for 0 would spin a core.
    fn next_frame_delay() -> u32 {
        PLAYER.with(|p| {
            let p = p.borrow();
            // A pending restart must not wait out the idle delay — the keypress
            // would feel dead for a quarter second after playback finished.
            if p.done && !p.restart {
                250
            } else {
                // ‼️ IDLE-PACE DURING PLAYBACK. A video player's UI is static while
                // the picture moves — only the progress bar and subtitles change,
                // a few times a second at most.
                //
                // ‼️ But the guest MUST pump at the video frame rate, because
                // pumping is what pulls decoded frames and schedules their
                // presentation. On DESKTOP a lazy UI cadence (~5 Hz) was enough
                // because the host had an INDEPENDENT video wake source — the
                // scheduled-frame deadline (video_desktop::time_until_next_scheduled)
                // drained the present queue at frame rate on its own. ON ANDROID
                // THAT SOURCE DOES NOT EXIST: present(at-ns) hands the buffer
                // straight to SurfaceFlinger (AMediaCodec_releaseOutputBufferAtTime),
                // so there is no host-side queue to wake on. If we idle at 5 Hz here
                // the guest pumps at ~5 Hz, cannot feed/pull/schedule fast enough to
                // stay ahead of real time, every deadline lands in the past and
                // playback runs at ~0.5x. So ask for a frame-rate cadence directly.
                // ~60 Hz gives headroom over 30 fps content and is harmless on
                // desktop (it just wakes at 60 Hz instead of relying on the queue).
                16
            }
        })
    }
}

export!(Component);
