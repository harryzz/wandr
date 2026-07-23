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
use crate::wandr::video::decoder::{self as decoder_api, Acceleration, VideoDecoder};
use crate::wandr::video::types::{Codec, DecoderConfig, TimedFrame, VideoError, VideoRect, ZLayer};
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::types as wtypes;
use std::cell::RefCell;

/// Tried in order; the first that opens wins. MP4 first because it carries real
/// presentation times — swap the order to run the elementary-stream half of the
/// finding-#6 comparison against identical content.
const CLIPS: &[&str] = &["/samples/bbb-h264.mp4", "/samples/bbb.h264"];
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
const MAX_SUBMITS_PER_FRAME: usize = 4;

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
fn demux(bytes: &[u8]) -> Result<(Vec<Au>, Timing), String> {
    if bytes.len() > 8 && &bytes[4..8] == b"ftyp" {
        demux_mp4(bytes).map(|aus| (aus, Timing::Container))
    } else {
        Ok((
            access_units(bytes)
                .into_iter()
                .enumerate()
                .map(|(i, (data, keyframe))| Au { data, pts_us: synth_pts_us(i as i64), keyframe })
                .collect(),
            Timing::SynthesizedDts,
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
fn demux_mp4(bytes: &[u8]) -> Result<Vec<Au>, String> {
    let size = bytes.len() as u64;
    let mut mp4 = mp4::Mp4Reader::read_header(std::io::Cursor::new(bytes), size)
        .map_err(|e| format!("mp4 header: {e}"))?;

    let (tid, timescale, sps, pps, nal_len, count) = {
        let t = mp4
            .tracks()
            .values()
            .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H264)))
            .ok_or("no H.264 track")?;
        (
            t.track_id(),
            t.timescale() as i64,
            t.sequence_parameter_set().map_err(|e| format!("sps: {e}"))?.to_vec(),
            t.picture_parameter_set().map_err(|e| format!("pps: {e}"))?.to_vec(),
            // The NAL length prefix is 1..4 bytes, declared per file in avcC's
            // lengthSizeMinusOne. Almost always 4 — but "almost always" is how you
            // get a decoder fed garbage on the one file that differs, so read it.
            avcc_nal_length_size(bytes).unwrap_or(4),
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
            au.extend_from_slice(&START);
            au.extend_from_slice(&sps);
            au.extend_from_slice(&START);
            au.extend_from_slice(&pps);
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
    Ok(out)
}

/// `lengthSizeMinusOne` from the avcC box: the low 2 bits of the byte at offset 4
/// of the box payload, plus one. The `mp4` crate parses avcC but does not expose
/// this field, so read it from the raw box — the same shape as the host's hvcC
/// parser, and for the same reason.
fn avcc_nal_length_size(file: &[u8]) -> Option<usize> {
    let p = file.windows(4).position(|w| w == b"avcC")?;
    let payload = file.get(p + 4..)?;
    Some((*payload.get(4)? & 0b11) as usize + 1)
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

struct Player {
    dec: Option<VideoDecoder>,
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
}

impl Player {
    const fn new() -> Self {
        Self {
            dec: None,
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
            surface: (1280, 720),
            accel: Acceleration::NoPreference,
            restart: false,
            pumps: 0,
        }
    }

    fn start(&mut self, nanos: u64) {
        self.started = true;
        self.origin_ns = nanos;
        // First clip that opens wins; a missing one is not an error, a missing
        // mount is.
        let Some((path, buf)) = CLIPS.iter().find_map(|p| std::fs::read(p).ok().map(|b| (*p, b)))
        else {
            println!("player: none of {CLIPS:?} could be read — is the /samples mount resolving?");
            self.done = true;
            return;
        };
        match demux(&buf) {
            Ok((aus, timing)) => {
                self.aus = aus;
                self.timing = timing;
            }
            Err(e) => {
                println!("player: demux {path} failed: {e}");
                self.done = true;
                return;
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
        let available = decoder_api::list_decoders();
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
        println!("player: keys — [h] hardware  [s] software  [n] no-preference  (restarts playback)");

        // Decode-to-surface: a REAL rect is what makes the host composite video.
        // above-ui = the video covers its rect and the UI lays out around it, so
        // we don't need the transparent-hole dance behind-ui requires.
        let (w, h) = self.surface;
        let vh = (w * 9 / 16).min(h);
        match VideoDecoder::open_accelerated(
            DecoderConfig {
                codec: Codec::H264,
                width: 1280,
                height: 720,
                rect: VideoRect { x: 0, y: (h.saturating_sub(vh)) / 2, width: w, height: vh },
                rotation: 0,
                // behind-ui: the video sits UNDER our UI, so the caption bar we draw
                // below lands on top of the picture. This is what a real player wants.
                layer: ZLayer::BehindUi,
            },
            self.accel,
        ) {
            Ok(d) => {
                // Report what we GOT, not what we asked for: `prefer-*` is a hint,
                // so a request for hardware can be quietly served by software and
                // the difference is exactly what we are here to measure.
                let got = d.implementation();
                println!(
                    "player: decoder OPEN ✓ decode-to-surface {w}x{vh} — asked {:?}, got {} ({})",
                    self.accel,
                    got.name,
                    if got.hardware { "HARDWARE" } else { "software" }
                );
                self.dec = Some(d);
            }
            Err(e) => {
                println!("player: decoder open FAILED: {e:?}");
                self.done = true;
            }
        }
    }

    /// Tear down and replay from the top with the currently selected
    /// acceleration. Dropping the decoder is what releases the host's surface and
    /// any frames it still holds — there is no reopen-in-place verb, and `reset`
    /// would keep the SAME backend, which is precisely what we want to change.
    fn restart(&mut self, nanos: u64) {
        self.dec = None;
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
            let frame = TimedFrame {
                data: au.data.clone(),
                timestamp_us: au.pts_us,
                keyframe: au.keyframe,
            };
            match dec.submit_timed(&frame) {
                Ok(()) => {
                    self.submitted += 1;
                    self.next_au += 1;
                    submits += 1;
                }
                // Back-pressure, NOT loss: retry this same frame next pump. A file
                // player cannot resync at the next keyframe the way RTP can.
                Err(VideoError::QueueFull) => {
                    self.queue_full_hits += 1;
                    break;
                }
                Err(e) => {
                    println!("player: submit-timed failed at AU {}: {e:?}", self.next_au);
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
            let first = *self.first_pts_us.get_or_insert(pts);
            let at_ns = self.origin_ns.saturating_add(((pts - first).max(0) as u64) * 1_000);
            frame.present(at_ns);
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

        // behind-ui proof: clear TRANSPARENT so the host's video shows through
        // everywhere we do not paint, then draw one opaque caption bar. If the
        // bar appears OVER the picture, guest-UI-on-top-of-video works — which is
        // what subtitles and controls need. (A real player would lay out text
        // here; a solid bar is enough to prove the compositing order.)
        let cv = wctx(|x| x.get_current_buffer());
        cv.clear(0x0000_0000);
        let (w, h) = PLAYER.with(|p| p.borrow().surface);
        let bar_h = (h as f32 * 0.14).max(48.0);
        cv.draw_rect(
            wtypes::Rect { x: 0.0, y: h as f32 - bar_h, width: w as f32, height: bar_h },
            &bar_paint(0xC8000000),
        );
        cv.draw_rect(
            wtypes::Rect { x: 0.0, y: h as f32 - bar_h, width: w as f32 * 0.62, height: 6.0 },
            &bar_paint(0xFF40C0FF),
        );
        drop(cv);
        wctx(|x| x.present());
    }

    /// The host tells us the surface size here — this is how we learn the
    /// geometry without importing a canvas interface.
    fn on_resize(w: u32, h: u32) {
        PLAYER.with(|p| p.borrow_mut().surface = (w.max(1), h.max(1)));
    }
}

impl KeyGuest for Component {
    /// H / S / N pick the acceleration and replay. Down-edge only — a key repeat
    /// would otherwise restart playback continuously while held.
    fn on_key(ev: KeyEvent) {
        if !ev.down || ev.repeat {
            return;
        }
        let want = match ev.text.to_ascii_lowercase().as_str() {
            "h" => Some(Acceleration::PreferHardware),
            "s" => Some(Acceleration::PreferSoftware),
            "n" => Some(Acceleration::NoPreference),
            _ => None,
        };
        if let Some(accel) = want {
            PLAYER.with(|p| {
                let mut p = p.borrow_mut();
                p.accel = accel;
                p.restart = true;
            });
        }
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
                (1000 / FPS) as u32
            }
        })
    }
}

export!(Component);
