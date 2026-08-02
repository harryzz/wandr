//! wandr-media-engine — the shared guest-side playback ENGINE (task 119).
//!
//! Extracted verbatim from wandr.jellyfin (the first consumer): container demux
//! (MP4 sample-table + MKV/Matroska), guest-side audio decode (AAC/MP3/Opus/AC-3),
//! the A/V master clock + present(at-ns) sync, the wandr:video decode-feed, and the
//! on-screen transport/scrub/subtitle overlay.
//!
//! It owns an IMPORTS-ONLY `wit_bindgen::generate!` (video + audio + canvas) so it
//! composes with each consumer app's own EXPORTS `generate!` in the same cdylib
//! (no cabi_realloc clash — the same trick wandr-reqwest uses). A thin app depends
//! only on this crate + wandr-reqwest and CALLS the engine fns.
//!
//! Increment 1: this crate compiles STANDALONE for wasm32-wasip2. App coupling
//! (Engine/Phase/Item/session-reporting) is severed at three seams: `log` prints
//! (no on-screen ring), `Controls`/`CONTROLS` replaces the `ENGINE` reads the
//! pump/overlay need, and the open fns take `title`+`duration_us` instead of `Item`.
#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

pub mod bindings {
    wit_bindgen::generate!({ path: "wit", world: "media-engine-imports", generate_all });
}

// Re-export the canvas bindings so a consumer app can paint its own UI against the
// SAME generated types the engine's overlay uses (one bindgen, no type mismatch).
pub use self::bindings::wasi::canvas;

// Re-export the container reader's prefetch driver + handle so the app's bg-tick
// can keep the stream's cache warm (the reader's block_on fallback stays quiet).
pub use httprange::{drive_prefetch, PrefetchHandle};

use bindings::wandr::video::decoder::{Acceleration, VideoDecoder};
use bindings::wandr::video::types::{Codec, DecoderConfig, TimedFrame, VideoError, VideoRect, ZLayer};
use bindings::wasi::audio::pcm as wpcm;
use bindings::wasi::canvas::{draw::Canvas, embedding as wembed, layout as wlayout, types as wtypes};

use symphonia::core::audio::{Channels, SampleBuffer};
use symphonia::core::codecs::{CodecParameters, Decoder, DecoderOptions, CODEC_TYPE_AAC};
use symphonia::core::formats::Packet;

// Fragmented-MP4 (DASH/CMAF) demux — see Demux::Fmp4 / open_fmp4. `oxideav_core`'s
// Demuxer trait methods resolve through the `dyn` type, so no `use` is needed.
use oxideav_core::{MediaType, NullCodecResolver};

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Cursor;

mod net;
mod streaming;
mod mkv;
mod httprange;

/// The wasi:audio device rate. Symphonia output is resampled to this; the device
/// exposes only mono/stereo, so multichannel is downmixed to stereo first.
const OUT_RATE: u32 = 48_000;

/// Bytes to pull per range request. Larger = fewer TLS handshakes (wandr-reqwest
/// closes the connection per request), at the cost of memory held in the window.
const FETCH_WINDOW: u64 = 8 * 1024 * 1024;
/// Decode-ahead cushion in frames — must exceed the decoder's reorder depth or
/// playback deadlocks (the lesson baked into wandr.video.player's DECODE_AHEAD).
const DECODE_AHEAD: usize = 20;

/// Demux no more than this far ahead of the playback clock (µs). Capping by TIME
/// (not per-queue count) keeps BOTH tracks equally buffered when the software
/// decoder can't keep realtime — otherwise a full, slow-draining video queue
/// would stop audio demux and starve the ring (silence). 3 s of lead.
const LOOKAHEAD_US: i64 = 4_000_000;
/// Hard safety caps (memory backstop). Sized near the lookahead so a
/// video-bottleneck can't over-buffer audio to a huge lead: 150 video frames
/// ≈ 6 s @25fps, 250 audio frames ≈ 5.3 s.
const VQ_CAP: usize = 150;
const AQ_CAP: usize = 250;

// ---- app-coupling seams ----------------------------------------------------

/// Simple log — prints to stdout (headless proof). The app keeps its own on-screen
/// ring; the engine no longer writes into it (that was the `ENGINE.log` coupling).
pub fn log(msg: impl Into<String>) {
    println!("engine: {}", msg.into());
}

/// A generic HTTP client (no server auth — Jellyfin/DASH put auth in the URL).
fn build_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("wandr-media-engine/0.1 ( https://github.com/harryzz/wandr )")
        .build()
        .ok()
}

/// The transport intents the pump + overlay read. In wandr.jellyfin these lived on
/// the app's `Engine`; the engine crate reads them through this decoupled seam so
/// the consumer app sets them (pause/mute/volume/seek/audio-switch/subtitles) and
/// the engine never names the app's state.
pub struct Controls {
    pub surface: (u32, u32),
    pub paused: bool,
    pub muted: bool,
    pub volume: f32,
    pub controls_until_ns: u64,
    pub controls_bump: bool,
    pub seek_request: Option<i64>,
    pub scrubbing: bool,
    pub scrub_frac: f32,
    pub sub_sel: Option<usize>,
    pub sub_dirty: bool,
    pub audio_pref: usize,
    pub audio_switch: bool,
    pub stop_requested: bool,
}
impl Controls {
    pub const fn new() -> Self {
        Controls {
            surface: (520, 1040),
            paused: false,
            muted: false,
            volume: 1.0,
            controls_until_ns: 0,
            controls_bump: false,
            seek_request: None,
            scrubbing: false,
            scrub_frac: 0.0,
            sub_sel: None,
            sub_dirty: false,
            audio_pref: 0,
            audio_switch: false,
            stop_requested: false,
        }
    }
}
impl Default for Controls {
    fn default() -> Self { Self::new() }
}

thread_local! {
    pub static CONTROLS: RefCell<Controls> = RefCell::new(Controls::new());
    /// The wasi:canvas embedding context, opened lazily on first paint.
    pub static WCTX: RefCell<Option<wembed::CanvasContext>> = const { RefCell::new(None) };
}

// ---- streaming playback ----------------------------------------------------

/// One demuxed video frame — already Annex-B framed (H.264/5) or raw (VP9/AV1),
/// ready to submit with its presentation time.
pub struct VFrame {
    pts_us: i64,
    keyframe: bool,
    data: Vec<u8>,
}
/// One demuxed audio frame — a raw codec packet (AAC) with its presentation time.
pub struct AFrame {
    pts_us: i64,
    data: Vec<u8>,
}

/// Container-specific demux state. Both containers feed the same frame queues, so
/// everything downstream (decode, audio, A/V sync) is container-agnostic.
pub enum Demux {
    /// MP4/MOV: the `mp4` crate over a blocking HTTP-Range reader (`read_sample`
    /// fetches through it). Random-access, so we pull whichever track is behind.
    Mp4(Box<Mp4Source>),
    /// MKV/WebM: the oxideav-mkv `Demuxer` over a blocking HTTP-Range reader.
    /// It handles all the container detail (lacing, block groups, Cues seek) that a
    /// hand-rolled parser misses. Boxed because MatroskaFile is large.
    Mkv(Box<MkvSource>),
    /// DASH/HLS fragmented MP4 (CMAF): STREAMING per-segment demux. Video and audio
    /// arrive as SEPARATE segment streams (each a DASH Representation), so this holds
    /// two independent `SegStream`s whose packets merge into the frame queues. Each
    /// fetches ONE CMAF media segment at a time (init + segment = a tiny self-
    /// contained fragmented file oxideav-mp4 opens); bounded memory, fast startup.
    /// (The `mp4` crate can't read fragments at all — broken sample offsets.)
    Fmp4(Box<Fmp4Source>),
}

/// One DASH Representation streamed segment-by-segment. `oxideav_mp4::demux::open`
/// walks the whole input to EOF to index fragments, so we DON'T hand it the entire
/// stream — instead we open a fresh demuxer per `init + segment` (each segment is a
/// keyframe-aligned, self-contained fragmented file whose PTS is ABSOLUTE from its
/// `tfdt`, verified in repros/fmp4-probe). Fetches are blocking (`block_on`), legal
/// because fill_queues runs in the async bg-tick (same as the MP4/MKV Range reader).
/// One media segment reference: a URL, optionally a byte range within it. DASH and
/// whole-file HLS use `range: None` (fetch the whole URL); HLS byte-range playlists
/// (`#EXT-X-BYTERANGE`, one file split into segments) use `Some((offset, length))`.
#[derive(Clone)]
pub struct Seg {
    pub url: String,
    pub range: Option<(u64, u64)>, // (offset, length)
}

/// A one-segment read-ahead buffer, shared with the async prefetch task so the NEXT
/// segment's bytes are ready before the current one is exhausted (no boundary stall).
#[derive(Default)]
struct Prefetch {
    ready: Option<(usize, Vec<u8>)>, // (segment idx, bytes)
    inflight: bool,
}

struct SegStream {
    /// ftyp+moov init segment — the demux prefix for every media segment.
    init: Vec<u8>,
    /// Media segments (url + optional byte range), in presentation order.
    segs: Vec<Seg>,
    /// Each segment's absolute start time (µs) — the seek target → segment map.
    starts_us: Vec<i64>,
    client: reqwest::Client,
    /// Next segment to fetch+open.
    idx: usize,
    /// Demuxer over the current `init + segment` (None until the next is opened).
    dmx: Option<Box<dyn oxideav_core::Demuxer>>,
    /// time_base (num,den) for pts_us = pts * num * 1e6 / den.
    num: i64,
    den: i64,
    /// All segments consumed (or a fatal fetch/open error).
    done: bool,
    /// Read-ahead for the next segment (filled by `drive_prefetch`, consumed by
    /// `open_next` — so the segment-boundary fetch is off the critical path).
    pf: std::rc::Rc<RefCell<Prefetch>>,
}

/// Fetch one segment's bytes (whole URL or byte range).
async fn fetch_seg_bytes(client: &reqwest::Client, s: &Seg) -> Result<Vec<u8>, String> {
    match s.range {
        Some((off, len)) => net::fetch_range(client, &s.url, off, Some(off + len - 1)).await.map(|r| r.bytes),
        None => net::fetch_url(client, &s.url).await,
    }
}

impl SegStream {
    fn ticks_to_us(pts: i64, num: i64, den: i64) -> i64 {
        if den == 0 { 0 } else { (pts as i128 * num as i128 * 1_000_000 / den as i128) as i64 }
    }
    /// Spawn an async fetch of the NEXT segment into the shared prefetch slot (if not
    /// already ready/in-flight). Called from bg-tick so the boundary fetch is hidden.
    fn drive_prefetch(&self) {
        let idx = self.idx;
        if idx >= self.segs.len() {
            return;
        }
        {
            let pf = self.pf.borrow();
            if pf.inflight || pf.ready.as_ref().map(|(i, _)| *i == idx).unwrap_or(false) {
                return;
            }
        }
        self.pf.borrow_mut().inflight = true;
        let pf = self.pf.clone();
        let s = self.segs[idx].clone();
        let client = self.client.clone();
        reqwest::task::spawn(async move {
            let bytes = fetch_seg_bytes(&client, &s).await;
            let mut p = pf.borrow_mut();
            p.inflight = false;
            if let Ok(b) = bytes {
                p.ready = Some((idx, b));
            }
        });
    }
    /// Fetch + open the next media segment (`init + segs[idx]`). False = no more / error.
    /// Uses the prefetched bytes if ready; otherwise blocks (legal in the bg-tick).
    fn open_next(&mut self) -> bool {
        if self.idx >= self.segs.len() {
            self.done = true;
            return false;
        }
        // Prefer the prefetched bytes for this idx; else block-fetch.
        let prefetched = {
            let mut pf = self.pf.borrow_mut();
            match &pf.ready {
                Some((i, _)) if *i == self.idx => pf.ready.take().map(|(_, b)| b),
                _ => None,
            }
        };
        let seg = match prefetched {
            Some(b) => b,
            None => {
                let s = self.segs[self.idx].clone();
                let client = self.client.clone();
                match wit_bindgen::rt::async_support::block_on(async move { fetch_seg_bytes(&client, &s).await }) {
                    Ok(b) => b,
                    Err(e) => {
                        log(format!("fmp4: segment {} fetch: {e}", self.idx));
                        self.done = true;
                        return false;
                    }
                }
            }
        };
        let mut buf = Vec::with_capacity(self.init.len() + seg.len());
        buf.extend_from_slice(&self.init);
        buf.extend_from_slice(&seg);
        match oxideav_mp4::demux::open(Box::new(Cursor::new(buf)), &NullCodecResolver) {
            Ok(d) => {
                self.dmx = Some(d);
                self.idx += 1;
                true
            }
            Err(e) => {
                log(format!("fmp4: segment {} open: {e:?}", self.idx));
                self.done = true;
                false
            }
        }
    }
    /// Next packet → (pts_us, keyframe, bytes). None at end of stream. Rolls to the
    /// next segment on the current demuxer's EOF.
    fn next_packet(&mut self) -> Option<(i64, bool, Vec<u8>)> {
        loop {
            if self.dmx.is_none() && !self.open_next() {
                return None;
            }
            match self.dmx.as_mut().unwrap().next_packet() {
                Ok(pkt) => {
                    let pts = pkt.pts.or(pkt.dts).unwrap_or(0);
                    return Some((Self::ticks_to_us(pts, self.num, self.den), pkt.flags.keyframe, pkt.data));
                }
                Err(_) => self.dmx = None, // segment EOF → open the next one
            }
        }
    }
    /// Reposition to the segment covering `target_us` (segments are keyframe-aligned,
    /// so this lands on a keyframe). Returns the landed segment's start time (µs).
    fn seek(&mut self, target_us: i64) -> i64 {
        let seg = self.starts_us.iter().rposition(|&s| s <= target_us).unwrap_or(0);
        self.idx = seg;
        self.dmx = None;
        self.done = false;
        // Drop any read-ahead for the pre-seek position.
        *self.pf.borrow_mut() = Prefetch::default();
        self.starts_us.get(seg).copied().unwrap_or(0)
    }
}

/// A DASH/CMAF session: a video `SegStream` + an optional audio one, streamed
/// per-segment. Presents the same next_video/next_audio/seek surface `fill_queues`
/// and `do_seek` use for the other containers.
pub struct Fmp4Source {
    video: SegStream,
    audio: Option<SegStream>,
}

impl Fmp4Source {
    fn has_audio(&self) -> bool {
        self.audio.is_some()
    }
    fn video_done(&self) -> bool {
        self.video.done
    }
    fn audio_done(&self) -> bool {
        self.audio.as_ref().map(|a| a.done).unwrap_or(true)
    }
    fn next_video(&mut self) -> Option<(i64, bool, Vec<u8>)> {
        self.video.next_packet()
    }
    fn next_audio(&mut self) -> Option<(i64, Vec<u8>)> {
        self.audio.as_mut()?.next_packet().map(|(pts, _kf, data)| (pts, data))
    }
    /// Seek both streams to the segment covering `target_us`; returns the landed
    /// (video-segment-start) time, which is ≤ target since segments are ~seconds long.
    fn seek(&mut self, target_us: i64) -> i64 {
        let landed = self.video.seek(target_us);
        if let Some(a) = self.audio.as_mut() {
            a.seek(target_us);
        }
        landed
    }
    /// Kick off read-ahead of the next video + audio segment (off the critical path).
    fn drive_prefetch(&self) {
        self.video.drive_prefetch();
        if let Some(a) = &self.audio {
            a.drive_prefetch();
        }
    }
}

/// Drive the DASH/CMAF read-ahead for the active stream (no-op for MP4/MKV, which
/// have their own HttpRangeReader prefetch). Called from the consumer's bg-tick.
pub fn drive_fmp4_prefetch() {
    STREAM.with(|s| {
        if let Some(p) = s.borrow().as_ref() {
            if let Demux::Fmp4(src) = &p.demux {
                src.drive_prefetch();
            }
        }
    });
}

/// A live `mp4`-crate demux session over the blocking Range reader.
pub struct Mp4Source {
    reader: mp4::Mp4Reader<httprange::HttpRangeReader>,
    video_track_id: u32,
    audio_track_id: u32, // 0 = no audio
    next_v: u32,         // next sample id (1-based)
    next_a: u32,
    video_count: u32,
    audio_count: u32,
    timescale_v: u32,
    timescale_a: u32,
    /// Edit-list presentation offsets (µs) — added to each track's raw sample PTS
    /// to put both tracks on one movie timeline (audio +delay, video −trim).
    video_edit_us: i64,
    audio_edit_us: i64,
}

/// A `Send` wrapper over the `Rc`-based `HttpRangeReader`. oxideav's `ReadSeek`
/// requires `Send`, but wasip2 is single-threaded and the reader never crosses a
/// thread (the prefetch task shares its `Rc` on the same thread), so this is sound.
struct SendReader(httprange::HttpRangeReader);
unsafe impl Send for SendReader {}
impl std::io::Read for SendReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> { self.0.read(buf) }
}
impl std::io::Seek for SendReader {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> { self.0.seek(from) }
}

/// One MKV audio stream's decode params — kept for all tracks so the audio-track
/// switch (C2) can rebuild the decoder in-place without re-opening the stream.
struct MkvAudioStream {
    index: u32,              // oxideav stream index
    codec_id: String,        // "aac" / "opus" / "ac3" / "eac3"
    extradata: Vec<u8>,      // ASC for AAC
    sample_rate: u32,
    channels: u16,
    num: i64,
    den: i64,                // time_base
    label: String,           // language / name for the on-screen readout
}

/// A live `oxideav-mkv` demux session over the (Send-wrapped) Range reader. Video +
/// audio arrive interleaved from ONE demuxer (`next_packet`), routed by stream index;
/// seek is Cues-indexed (SeekHead-jump, no whole-file scan — the git-pinned fix).
pub struct MkvSource {
    dmx: Box<dyn oxideav_core::Demuxer>,
    video_stream: u32,
    /// Currently-consumed audio stream index (`u32::MAX` = none).
    audio_stream: u32,
    v_num: i64,
    v_den: i64,
    a_num: i64,
    a_den: i64,
    /// All audio streams in container order (index = the cycle position for `a`).
    audio_streams: Vec<MkvAudioStream>,
}

pub struct StreamPlayer {
    url: String,
    total_len: u64,
    demux: Demux,
    /// Start-coded parameter-set prefix (H.264/H.265), prepended at sync frames.
    ps_prefix: Vec<u8>,
    nal_len: usize,
    /// true = H.264/H.265 (length-prefixed NALs → Annex-B); false = VP9/AV1 raw.
    video_annexb: bool,
    dec: VideoDecoder,
    buf: streaming::RollingBuffer,
    fetch_inflight: bool,
    /// Demuxed-but-not-yet-submitted frames.
    video_q: VecDeque<VFrame>,
    audio_q: VecDeque<AFrame>,
    /// Set once the demuxer has produced its last frame.
    demux_done: bool,
    submitted: usize,
    presented: usize,
    /// PTS (µs) of the last video frame actually shown — for the sync diagnostic.
    last_pres_pts: i64,
    /// Host clock (ns) at the first pump — to measure the true wall-clock rate of
    /// the audio clock (is position() advancing at realtime?).
    t0_ns: u64,
    /// Total video frames if known (MP4 sample count); 0 = unknown (MKV stream).
    total_video: usize,
    /// PTS of the first decoded frame; playback clock anchors to its emergence.
    first_pts_us: Option<i64>,
    origin_ns: u64,
    flushed: bool,
    done: bool,
    title: String,
    duration_us: i64,
    /// Playback clock in µs (for the overlay), derived from the host clock.
    clock_us: i64,
    /// Audio time from the DEVICE's played position — shown on the overlay next to
    /// the video clock so A/V sync is verifiable on-screen for any format.
    audio_pos_us: i64,

    // ---- audio ----
    audio_dec: Option<AudioDec>,
    /// Whether this stream has decodable audio (the DEVICE is shared + persistent
    /// in AUDIO_DEV — never opened/closed per stream, which churns COM on Windows).
    has_audio: bool,
    resampler: Option<LinearResampler>,
    /// Device `position()` captured at this stream's first audio write — the
    /// origin for turning the cumulative device clock into this stream's media time.
    dev_start: u64,
    dev_start_set: bool,
    /// Host time (ns) when this stream's audio actually began playing (position
    /// first advanced past dev_start). The A/V clock advances at WALL rate from
    /// here — position()'s own rate is mis-scaled when the device's native rate
    /// differs from the 48 kHz we open (e.g. 44100 → position runs ~8% slow).
    audio_start_ns: u64,
    /// PTS of the first audio sample fed — the origin of the audio master clock.
    /// Recorded by the DECODE stage (`decode_audio`, bg-tick) on the first frame.
    audio_first_pts_us: i64,
    /// Set once `audio_first_pts_us` is known — the pre-roll/anchor key the pump
    /// gates on. (Distinct from `audio_pts_known` = "audio has started playing".)
    audio_first_pts_known: bool,
    audio_pts_known: bool,
    /// Decoded interleaved-stereo PCM @ 48 kHz — the ONLY thing crossing the
    /// decode→output boundary. Filled by `decode_audio` (bg-tick, off the
    /// real-time path); drained to the device ring by the pump. Codec-agnostic.
    pending_pcm: Vec<f32>,

    /// Handle to the container reader's shared cache, so bg-tick can drive async
    /// prefetch (keeping the reader's block_on fallback from firing = no hiccup).
    prefetch: Option<httprange::PrefetchHandle>,

    /// Pause state applied by the pump (Part B): `paused` = currently honoring a
    /// pause (audio device paused, clock frozen); `paused_at_ns` = host time it
    /// began, so resume shifts `audio_start_ns`/`origin_ns` forward by the elapsed
    /// pause and `media_now` stays continuous.
    paused: bool,
    paused_at_ns: u64,
}

impl StreamPlayer {
    /// Playback clock (movie µs) — the overlay/report position the app reads.
    pub fn clock_us(&self) -> i64 { self.clock_us }
    /// Total duration (µs) as passed at open.
    pub fn duration_us(&self) -> i64 { self.duration_us }
    /// Handle for driving async prefetch from the app's bg-tick (None on file://).
    pub fn prefetch_handle(&self) -> Option<httprange::PrefetchHandle> { self.prefetch.clone() }
    /// Number of switchable audio tracks (MKV) — 1 for single-track / MP4.
    pub fn audio_track_count(&self) -> usize {
        match &self.demux { Demux::Mkv(src) => src.audio_streams.len().max(1), _ => 1 }
    }
}

thread_local! {
    pub static STREAM: RefCell<Option<StreamPlayer>> = const { RefCell::new(None) };
    /// The ONE audio output device, opened lazily and kept open for the app's
    /// lifetime — reused (with `flush` between streams) rather than reopened.
    /// Reopening a wasi:audio device per stream churns COM on Windows/WASAPI and
    /// breaks the second playback's audio (audio.player keeps one device too).
    pub static AUDIO_DEV: RefCell<Option<wpcm::Playback>> = const { RefCell::new(None) };
}

/// Open the shared audio device once (48 kHz stereo). Returns false if the host
/// has no audio output. Idempotent — subsequent calls reuse the open device.
pub fn ensure_audio_device() -> bool {
    AUDIO_DEV.with(|d| {
        if d.borrow().is_some() {
            return true;
        }
        let cfg = wpcm::StreamConfig {
            sample_rate: OUT_RATE,
            channel_layout: wpcm::ChannelLayout::Stereo,
            format: wpcm::Format::PcmF32,
            class: wpcm::StreamClass::Media,
        };
        match wpcm::Playback::open(cfg) {
            Ok(pb) => {
                let _ = pb.start();
                *d.borrow_mut() = Some(pb);
                true
            }
            Err(_) => false,
        }
    })
}

/// Run `f` against the shared device if it's open.
pub fn with_audio<R>(f: impl FnOnce(&wpcm::Playback) -> R) -> Option<R> {
    AUDIO_DEV.with(|d| d.borrow().as_ref().map(f))
}

/// 16:9 letterbox centered vertically, full width — one source of truth so open
/// and resize place the video identically (mirrors wandr.video.player).
fn video_rect(w: u32, h: u32) -> VideoRect {
    let vh = (w * 9 / 16).min(h);
    VideoRect { x: 0, y: (h.saturating_sub(vh)) / 2, width: w, height: vh }
}

/// Update the render surface size (the app's on-resize forwards here): records it
/// on CONTROLS (drives the overlay layout) and live-reconciles the decoder rect.
pub fn set_surface(w: u32, h: u32) {
    CONTROLS.with(|c| c.borrow_mut().surface = (w.max(1), h.max(1)));
    STREAM.with(|s| {
        if let Some(p) = s.borrow().as_ref() {
            p.dec.set_rect(video_rect(w.max(1), h.max(1)));
        }
    });
}

/// Linear resampler src→48 kHz on interleaved frames (ported verbatim from
/// wandr.audio.player — same simple, low-cost interpolation).
pub struct LinearResampler {
    src: u32,
    step: f64,
    ch: usize,
    pos: f64,
    last: Vec<f32>,
}
impl LinearResampler {
    fn new(src: u32, ch: usize) -> Self {
        Self { src, step: src as f64 / OUT_RATE as f64, ch, pos: 0.0, last: vec![0.0; ch] }
    }
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let ch = self.ch;
        let n = input.len() / ch;
        if n == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(((n as f64 / self.step) as usize + 2) * ch);
        while self.pos < n as f64 {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            for c in 0..ch {
                let a = if i == 0 { self.last[c] } else { input[(i - 1) * ch + c] };
                let b = input[i * ch + c];
                out.push(a + (b - a) * frac);
            }
            self.pos += self.step;
        }
        self.last.copy_from_slice(&input[(n - 1) * ch..n * ch]);
        self.pos -= n as f64;
        out
    }
}

/// Downmix any channel layout to interleaved stereo by CHANNEL ROLE (not by
/// guessing order): the device only offers mono/stereo, but AAC here is 5.1.
/// ITU-style coefficients — centre and surrounds fold in at -3 dB, LFE dropped.
fn downmix_to_stereo(samples: &[f32], chans: &[Channels]) -> Vec<f32> {
    let n = chans.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        // Mono → duplicate.
        let mut out = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            out.push(s);
            out.push(s);
        }
        return out;
    }
    let frames = samples.len() / n;
    let mut out = Vec::with_capacity(frames * 2);
    const C: f32 = 0.707;
    for f in 0..frames {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for (k, ch) in chans.iter().enumerate() {
            let s = samples[f * n + k];
            match *ch {
                Channels::FRONT_LEFT => l += s,
                Channels::FRONT_RIGHT => r += s,
                Channels::FRONT_CENTRE => { l += C * s; r += C * s; }
                Channels::REAR_LEFT | Channels::SIDE_LEFT => l += C * s,
                Channels::REAR_RIGHT | Channels::SIDE_RIGHT => r += C * s,
                Channels::LFE1 => {} // drop the sub
                _ => { l += 0.5 * s; r += 0.5 * s; }
            }
        }
        out.push(l.clamp(-1.0, 1.0));
        out.push(r.clamp(-1.0, 1.0));
    }
    out
}

// ---- subtitles (generic WebVTT overlay) ------------------------------------

/// One timed subtitle cue (µs).
pub struct Cue { start_us: i64, end_us: i64, text: String }

thread_local! {
    /// The currently-loaded subtitle track's cues (sorted by start). Swapped
    /// wholesale when the track changes/clears; read by render_playing.
    pub static SUBTITLES: RefCell<Vec<Cue>> = const { RefCell::new(Vec::new()) };
}

/// Parse a WebVTT timestamp `[HH:]MM:SS.mmm` → µs.
fn vtt_ts(s: &str) -> Option<i64> {
    let s = s.trim();
    let (hms, frac) = s.split_once('.').or_else(|| s.split_once(','))?;
    let ms: i64 = frac.get(..3).unwrap_or(frac).parse().ok()?;
    let p: Vec<&str> = hms.split(':').collect();
    let (h, m, sec): (i64, i64, i64) = match p.as_slice() {
        [h, m, s] => (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?),
        [m, s] => (0, m.parse().ok()?, s.parse().ok()?),
        _ => return None,
    };
    Some(((h * 3600 + m * 60 + sec) * 1000 + ms) * 1000)
}

/// Strip inline VTT tags (`<c>`, `<i>`, `<00:00:00.000>`…) from a cue line.
fn strip_vtt_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Minimal WebVTT → timed cues. Ignores the WEBVTT header, NOTE blocks and cue ids.
pub fn parse_vtt(text: &str) -> Vec<Cue> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    let mut it = text.lines().peekable();
    while let Some(line) = it.next() {
        let Some((a, rest)) = line.split_once("-->") else { continue };
        let Some(start) = vtt_ts(a) else { continue };
        let end = rest.trim().split_whitespace().next().and_then(vtt_ts).unwrap_or(start + 3_000_000);
        let mut buf = String::new();
        while let Some(t) = it.peek() {
            if t.trim().is_empty() { break; }
            if !buf.is_empty() { buf.push('\n'); }
            buf.push_str(strip_vtt_tags(t).trim());
            it.next();
        }
        if !buf.is_empty() {
            cues.push(Cue { start_us: start, end_us: end.max(start), text: buf });
        }
    }
    cues.sort_by_key(|c| c.start_us);
    cues
}

// ---- audio decode ----------------------------------------------------------

/// C2: switch the active MKV audio track IN PLACE — re-route the demux + rebuild
/// the decoder (codecs may differ) + flush the ring and re-anchor. Video untouched.
pub fn switch_audio(p: &mut StreamPlayer, pref: usize) {
    let info = match &p.demux {
        Demux::Mkv(src) => src.audio_streams.get(pref).map(|t|
            (t.codec_id.clone(), t.extradata.clone(), t.sample_rate, t.channels, t.index, t.num, t.den, t.label.clone())),
        Demux::Mp4(_) => { log("audio: MP4 is single-track (switch is MKV-only for now)"); return; }
        Demux::Fmp4(_) => { log("audio: DASH audio-track switch = re-open a rep (not in-place; TODO)"); return; }
    };
    let Some((cid, cpriv, sr, ch, index, num, den, label)) = info else { return };
    let (dec, ok, resampler) = setup_audio_by_codec(&cid, &cpriv, sr, ch);
    if !ok { log(format!("audio: {label} not decodable — keeping current")); return; }
    if let Demux::Mkv(src) = &mut p.demux { src.audio_stream = index; src.a_num = num; src.a_den = den; }
    p.audio_dec = dec;
    p.has_audio = ok;
    p.resampler = resampler;
    // Reset the audio path for the new track (video keeps running).
    p.audio_q.clear();
    p.pending_pcm.clear();
    with_audio(|pb| pb.flush());
    p.audio_pts_known = false;
    p.audio_first_pts_known = false;
    p.audio_first_pts_us = 0;
    p.audio_start_ns = 0;
    p.dev_start = 0;
    p.dev_start_set = false;
    log(format!("audio → {label} ({cid})"));
}

pub fn install_player(
    url: String, total_len: u64, demux: Demux, ps_prefix: Vec<u8>, nal_len: usize,
    video_annexb: bool, codec: Codec, width: u32, height: u32, surface: (u32, u32),
    audio_dec: Option<AudioDec>, has_audio: bool, resampler: Option<LinearResampler>,
    title: String, duration_us: i64, total_video: usize, seed: Option<(u64, Vec<u8>)>, first_off: u64,
) -> Result<String, String> {
    let (w, h) = surface;
    let dec = VideoDecoder::open_accelerated(
        DecoderConfig { codec, width, height, rect: video_rect(w, h), rotation: 0, layer: ZLayer::BehindUi },
        Acceleration::NoPreference,
    )
    .map_err(|e| format!("stream: decoder open: {e:?}"))?;
    let impl_name = dec.implementation().name;
    STREAM.with(|s| {
        let mut buf = streaming::RollingBuffer::new();
        match seed {
            // MKV: the header fetch is real contiguous file bytes — keep them so
            // the first cluster plays without an extra round-trip.
            Some((at, bytes)) => { buf.append(at, &bytes); buf.drop_before(first_off); }
            // MP4: header may be a spliced synthetic buffer; start empty + fetch.
            None => buf.reset_to(first_off),
        }
        *s.borrow_mut() = Some(StreamPlayer {
            url, total_len, demux, ps_prefix, nal_len, video_annexb, dec, buf,
            fetch_inflight: false,
            video_q: VecDeque::new(), audio_q: VecDeque::new(), demux_done: false,
            submitted: 0, presented: 0, last_pres_pts: 0, t0_ns: 0, total_video,
            first_pts_us: None, origin_ns: 0, flushed: false, done: false,
            title, duration_us, clock_us: 0, audio_pos_us: 0,
            audio_dec, has_audio, resampler, dev_start: 0, dev_start_set: false, audio_start_ns: 0,
            audio_first_pts_us: 0, audio_first_pts_known: false, audio_pts_known: false, pending_pcm: Vec::new(),
            prefetch: None,
            paused: false, paused_at_ns: 0,
        });
        // A new stream starts clean: drop any audio the previous stream left in
        // the shared device ring (the device itself stays open).
        with_audio(|pb| pb.flush());
    });
    Ok(impl_name)
}

/// The guest's audio decoders behind one interface. AAC/MP3 via Symphonia; Opus
/// and AC-3/E-AC-3 via the pure-Rust OxideAV crates (Symphonia has neither). Each
/// variant decodes one packet to interleaved STEREO f32 at the codec's native rate;
/// the StreamPlayer's `resampler` then converts that rate → 48 kHz.
pub enum AudioDec {
    Aac(Box<dyn Decoder>),
    /// ropus decoder + channel count (needed to interleave→stereo).
    Opus(ropus::Decoder, usize),
    Ac3(Box<dyn oxideav_core::Decoder>),
}

impl AudioDec {
    /// Decode one compressed audio packet → interleaved STEREO f32 in [-1, 1] at
    /// the codec's native sample rate. Empty on a decode hiccup (skip the packet).
    fn decode(&mut self, data: &[u8]) -> Vec<f32> {
        match self {
            AudioDec::Aac(dec) => {
                let packet = Packet::new_from_slice(0, 0, 1024, data);
                match dec.decode(&packet) {
                    Ok(decoded) => {
                        let spec = *decoded.spec();
                        let chans: Vec<Channels> = spec.channels.iter().collect();
                        let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                        sb.copy_interleaved_ref(decoded);
                        downmix_to_stereo(sb.samples(), &chans)
                    }
                    Err(_) => Vec::new(),
                }
            }
            AudioDec::Opus(dec, ch) => {
                let ch = *ch;
                // Buffer = max 48 kHz frame (120 ms = 5760/ch); ropus recovers the real
                // frame size from the packet and returns samples/channel (interleaved f32).
                let mut out = vec![0.0f32; 5760 * ch];
                match dec.decode_float(data, &mut out, ropus::DecodeMode::Normal) {
                    Ok(n) => {
                        out.truncate(n * ch);
                        f32_ilv_to_stereo(&out, ch)
                    }
                    Err(_) => Vec::new(),
                }
            }
            AudioDec::Ac3(dec) => {
                use oxideav_core::{Frame as OxFrame, Packet as OxPacket, TimeBase};
                let pkt = OxPacket::new(0, TimeBase::new(1, OUT_RATE as i64), data.to_vec());
                if dec.send_packet(&pkt).is_err() {
                    return Vec::new();
                }
                match dec.receive_frame() {
                    // make_decoder_ltrt outputs interleaved S16, already stereo.
                    Ok(OxFrame::Audio(af)) => {
                        let bytes = af.data.first().map(|v| v.as_slice()).unwrap_or(&[]);
                        let i16s: Vec<i16> = bytes
                            .chunks_exact(2)
                            .map(|b| i16::from_le_bytes([b[0], b[1]]))
                            .collect();
                        let ch = (i16s.len() / af.samples.max(1) as usize).max(1);
                        i16_ilv_to_stereo_f32(&i16s, ch)
                    }
                    _ => Vec::new(),
                }
            }
        }
    }
}

/// Interleaved f32 (any channel count) → interleaved STEREO f32. Stereo passes
/// through; mono duplicates; ≥3ch takes front L/R (acceptable first pass). opus-rs
/// already decodes to f32 in [-1, 1], so no scaling.
fn f32_ilv_to_stereo(pcm: &[f32], ch: usize) -> Vec<f32> {
    if ch == 0 {
        return Vec::new();
    }
    if ch == 2 {
        return pcm.to_vec();
    }
    let frames = pcm.len() / ch;
    let mut out = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let base = f * ch;
        let l = pcm[base];
        let r = if ch == 1 { l } else { pcm[base + 1] };
        out.push(l);
        out.push(r);
    }
    out
}

/// Interleaved i16 (any channel count) → interleaved STEREO f32. Mono duplicates;
/// ≥2ch takes the first two (front L/R) — AC-3 is already LtRt-downmixed to stereo,
/// stereo Opus is L/R; multichannel Opus front-L/R is an acceptable first pass.
fn i16_ilv_to_stereo_f32(pcm: &[i16], ch: usize) -> Vec<f32> {
    if ch == 0 {
        return Vec::new();
    }
    let frames = pcm.len() / ch;
    let mut out = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let base = f * ch;
        if ch == 1 {
            let s = pcm[base] as f32 / 32768.0;
            out.push(s);
            out.push(s);
        } else {
            out.push(pcm[base] as f32 / 32768.0);
            out.push(pcm[base + 1] as f32 / 32768.0);
        }
    }
    out
}

fn setup_aac_audio(asc: &[u8], src_rate: u32) -> (Option<AudioDec>, bool, Option<LinearResampler>) {
    if asc.len() < 2 {
        log("audio: ASC too short — video only");
        return (None, false, None);
    }
    let mut params = CodecParameters::new();
    params.for_codec(CODEC_TYPE_AAC).with_sample_rate(src_rate).with_extra_data(asc.to_vec().into_boxed_slice());
    let dec = match symphonia::default::get_codecs().make(&params, &DecoderOptions::default()) {
        Ok(d) => d,
        Err(e) => {
            log(format!("audio: decoder init failed ({e}) — video only"));
            return (None, false, None);
        }
    };
    if !ensure_audio_device() {
        log("audio: device unavailable — video only");
        return (None, false, None);
    }
    let resampler = if src_rate != OUT_RATE { Some(LinearResampler::new(src_rate, 2)) } else { None };
    (Some(AudioDec::Aac(dec)), true, resampler)
}

/// Opus (pure-Rust opus-decoder / Rusopus, RFC-8251-conformant) — decodes to 48 kHz,
/// so no resampler. `channels` (1 or 2) sizes the interleaved output.
fn setup_opus_audio(channels: usize) -> (Option<AudioDec>, bool, Option<LinearResampler>) {
    if !ensure_audio_device() {
        log("audio: device unavailable — video only");
        return (None, false, None);
    }
    // ropus::Decoder handles mono/stereo only. TODO(5.1): multichannel Opus (a few
    // titles ship 5.1) needs ropus's OpusMultistreamDecoder + a 5.1→stereo downmix;
    // for now clamp to stereo — a >2ch stream's packets won't decode here (no audio).
    let ch = channels.clamp(1, 2);
    let rch = if ch == 1 { ropus::Channels::Mono } else { ropus::Channels::Stereo };
    match ropus::Decoder::new(OUT_RATE, rch) {
        Ok(dec) => {
            log(format!("audio: Opus (ropus) {ch}ch → stereo 48k"));
            (Some(AudioDec::Opus(dec, ch)), true, None)
        }
        Err(e) => {
            log(format!("audio: ropus init failed: {e:?} — video only"));
            (None, false, None)
        }
    }
}

/// Build the audio decoder for an MKV audio stream by its oxideav codec id (reused
/// by open + the in-place audio-track switch).
/// Audio decoder for an oxideav-normalized codec id ("aac"/"opus"/"ac3"/"eac3").
/// `extradata` is the ASC for AAC. (oxideav-mp4 and oxideav-mkv report the same ids.)
fn setup_audio_by_codec(codec_id: &str, extradata: &[u8], sr: u32, chans: u16)
    -> (Option<AudioDec>, bool, Option<LinearResampler>)
{
    match codec_id {
        "aac" => setup_aac_audio(extradata, sr),
        "opus" => setup_opus_audio(chans as usize),
        "ac3" => setup_ac3_audio(sr, false),
        "eac3" => setup_ac3_audio(sr, true),
        _ => (None, false, None),
    }
}

/// AC-3 / E-AC-3 (pure-Rust oxideav) via the LtRt stereo-downmix decoder. AC-3 is
/// 32/44.1/48 kHz — resample to 48 k if needed. `eac3` selects the E-AC-3 path.
fn setup_ac3_audio(src_rate: u32, eac3: bool) -> (Option<AudioDec>, bool, Option<LinearResampler>) {
    if !ensure_audio_device() {
        log("audio: device unavailable — video only");
        return (None, false, None);
    }
    let mut params = oxideav_core::CodecParameters::audio(
        oxideav_core::CodecId::new(if eac3 { "eac3" } else { "ac3" }),
    );
    params.channels = Some(2); // request stereo (LtRt downmix)
    params.sample_rate = Some(src_rate);
    let made = if eac3 {
        oxideav_ac3::decoder::make_eac3_decoder(&params)
    } else {
        oxideav_ac3::decoder::make_decoder_ltrt(&params)
    };
    let dec = match made {
        Ok(d) => d,
        Err(e) => { log(format!("audio: {} init failed ({e:?}) — video only", if eac3 { "E-AC-3" } else { "AC-3" })); return (None, false, None); }
    };
    log(format!("audio: {} (oxideav) @ {src_rate}Hz → stereo 48k", if eac3 { "E-AC-3" } else { "AC-3" }));
    let resampler = if src_rate != OUT_RATE { Some(LinearResampler::new(src_rate, 2)) } else { None };
    (Some(AudioDec::Ac3(dec)), true, resampler)
}

/// Presentation offset (µs) an MP4 edit list applies to a track, mapping raw
/// sample times onto the movie timeline. `+` for an EMPTY edit (media_time = -1
/// = a DELAY, e.g. an 11 s audio pre-roll); `−` for a media_time TRIM (skips the
/// front, e.g. the 83 ms B-frame reorder trim). Ignoring these is what plays the
/// audio seconds ahead of the video. `movie_ts` = mvhd timescale (the unit of
/// `segment_duration`); `media_ts` = the track's mdhd timescale (`media_time`).
fn edit_offset_us(track: &mp4::Mp4Track, movie_ts: u32, media_ts: u32) -> i64 {
    let Some(elst) = track.trak.edts.as_ref().and_then(|e| e.elst.as_ref()) else { return 0 };
    let mut off = 0i64;
    for e in &elst.entries {
        // media_time == -1 is the empty-edit sentinel: u64::MAX (v1) / 0xFFFF_FFFF (v0).
        if e.media_time == u64::MAX || e.media_time == 0xFFFF_FFFF {
            off += e.segment_duration as i64 * 1_000_000 / movie_ts.max(1) as i64;
        } else {
            off -= e.media_time as i64 * 1_000_000 / media_ts.max(1) as i64;
        }
    }
    off
}

// ---- open / demux / seek ---------------------------------------------------

/// Open an MP4/MOV stream via the `mp4` crate over the blocking Range reader
/// (`read_sample` fetches through it). SYNCHRONOUS (block_on) — called from
/// the consumer's bg-tick. `title`/`duration_us` come from the app's catalog item.
pub fn open_mp4_sync(url: String, total_len: u64, title: String, duration_us: i64, surface: (u32, u32)) -> Result<(), String> {
    const START: [u8; 4] = [0, 0, 0, 1];
    let fail = |msg: String| -> Result<(), String> {
        log(msg.clone());
        Err(msg)
    };
    // LOCAL-FILE test mode (transport exclusion): `file://<path>` reads from disk
    // via the SAME reader type — everything downstream (demux/audio/clock) is
    // identical to the HTTP path, so this isolates whether the bug is transport.
    let (reader, handle) = if let Some(path) = url.strip_prefix("file://") {
        match httprange::HttpRangeReader::new_local(path, total_len) {
            Ok(r) => (r, None),
            Err(e) => return fail(format!("stream: local open {path}: {e}")),
        }
    } else {
        let Some(client) = build_client() else {
            return fail("stream: no HTTP client".into());
        };
        let r = httprange::HttpRangeReader::new(url.clone(), total_len, client);
        let h = r.handle();
        (r, h)
    };
    let mp4r = match mp4::Mp4Reader::read_header(reader, total_len) {
        Ok(r) => r,
        Err(e) => return fail(format!("stream: mp4 header: {e}")),
    };

    // Movie (mvhd) timescale — the unit of edit-list `segment_duration`.
    let movie_ts = mp4r.timescale();

    // Pull codec config + track ids out of the (borrowed) track map, then drop
    // the borrow so `mp4r` can move into Mp4Source.
    struct V { tid: u32, ts: u32, w: u32, h: u32, is264: bool, count: u32, sps: Option<Vec<u8>>, pps: Option<Vec<u8>>, edit_us: i64 }
    struct A { tid: u32, ts: u32, count: u32, asc: Option<Vec<u8>>, edit_us: i64 }
    let (vopt, aopt): (Option<V>, Option<A>) = {
        let tracks = mp4r.tracks();
        let v = tracks.values()
            .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H264 | mp4::MediaType::H265)))
            .map(|t| {
                let is264 = matches!(t.media_type(), Ok(mp4::MediaType::H264));
                let (sps, pps) = if is264 {
                    (t.sequence_parameter_set().ok().map(|s| s.to_vec()),
                     t.picture_parameter_set().ok().map(|s| s.to_vec()))
                } else { (None, None) };
                V { tid: t.track_id(), ts: t.timescale(), w: t.width() as u32, h: t.height() as u32,
                    is264, count: t.sample_count(), sps, pps,
                    edit_us: edit_offset_us(t, movie_ts, t.timescale()) }
            });
        let a = tracks.values()
            .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::AAC)))
            .map(|t| {
                let edit_us = edit_offset_us(t, movie_ts, t.timescale());
                // Reconstruct the AAC ASC from freq/chan (LC only — Symphonia
                // rejects >2ch, which channel_config() reports as Err → None).
                let asc = match (t.sample_freq_index(), t.channel_config()) {
                    (Ok(fi), Ok(cc)) => {
                        let freq_idx = freq_to_index(fi.freq());
                        let chan = match cc { mp4::ChannelConfig::Mono => 1u8, mp4::ChannelConfig::Stereo => 2, _ => 0 };
                        if chan == 0 { None } else {
                            Some(vec![(2u8 << 3) | (freq_idx >> 1), ((freq_idx & 1) << 7) | (chan << 3)])
                        }
                    }
                    _ => None,
                };
                A { tid: t.track_id(), ts: t.timescale(), count: t.sample_count(), asc, edit_us }
            });
        (v, a)
    };

    let Some(v) = vopt else { return fail("stream: mp4 has no H.264/H.265 track".into()); };

    // Video parameter sets. NAL length prefix is 4 in practice for MP4 (h264);
    // for h265 we read it from the fetched hvcC.
    let (codec, ps_prefix, nal_len) = if v.is264 {
        let (Some(sps), Some(pps)) = (v.sps.as_ref(), v.pps.as_ref()) else {
            return fail("stream: mp4 missing SPS/PPS".into());
        };
        let mut p = Vec::new();
        p.extend_from_slice(&START); p.extend_from_slice(sps);
        p.extend_from_slice(&START); p.extend_from_slice(pps);
        (Codec::H264, p, 4usize)
    } else {
        // H.265: the crate discards hvcC, so fetch the moov and parse it.
        match fetch_moov_for_config(&url, total_len) {
            Some(moov) => match streaming::parse_hvcc(&moov) {
                Some(pfx) => (Codec::H265, pfx, streaming::nal_length_size(&moov, false).unwrap_or(4)),
                None => return fail("stream: mp4 hvcC parse".into()),
            },
            None => return fail("stream: mp4 h265 moov fetch".into()),
        }
    };

    // Audio.
    let mut audio_track_id = 0u32;
    let mut timescale_a = 1u32;
    let mut audio_count = 0u32;
    // Edit-list offsets (µs): the audio empty-edit DELAY and the video reorder TRIM.
    // The net audio-vs-video skew is what plays audio ahead when ignored.
    let audio_edit_us = aopt.as_ref().map(|a| a.edit_us).unwrap_or(0);
    if v.edit_us != 0 || audio_edit_us != 0 {
        log(format!(
            "edit-list: video {:+}ms, audio {:+}ms (audio starts {:+}ms vs video)",
            v.edit_us / 1000, audio_edit_us / 1000, (audio_edit_us - v.edit_us) / 1000
        ));
    }
    let (audio_dec, has_audio, resampler) = match aopt.and_then(|a| a.asc.map(|asc| (a.tid, a.ts, a.count, asc))) {
        Some((tid, ts, count, asc)) => {
            let asc_sr = aac_freq_hz(((asc[0] & 0x07) << 1) | (asc[1] >> 7)).unwrap_or(OUT_RATE);
            // The TRUE playback rate is the media timescale (mdhd), not the ASC's
            // frequency index — an SBR/HE-AAC ASC signals the CORE rate while the
            // track plays at 2×, and some muxes simply disagree. Playing at the
            // wrong rate makes audio drift FAST ahead of video. Trust the timescale.
            let true_sr = if ts >= 8000 && ts <= 192_000 { ts } else { asc_sr };
            let (d, ok, r) = setup_aac_audio(&asc, true_sr);
            if ok {
                audio_track_id = tid; timescale_a = ts.max(1); audio_count = count;
                log(format!(
                    "audio: AAC ASC={asc_sr}Hz timescale={ts}Hz → using {true_sr}Hz → stereo 48k{}",
                    if asc_sr != true_sr { " (RATE MISMATCH — timescale wins)" } else { "" }
                ));
                (d, ok, r)
            } else { (None, false, None) }
        }
        None => { log("audio: no decodable AAC — video only"); (None, false, None) }
    };

    let title = title;
    let duration_us = duration_us;
    let src = Box::new(Mp4Source {
        reader: mp4r,
        video_track_id: v.tid,
        audio_track_id,
        next_v: 1, next_a: 1,
        video_count: v.count, audio_count,
        timescale_v: v.ts.max(1), timescale_a,
        video_edit_us: v.edit_us, audio_edit_us,
    });

    match install_player(
        url, total_len, Demux::Mp4(src), ps_prefix, nal_len, true, codec, v.w, v.h, surface,
        audio_dec, has_audio, resampler, title.clone(), duration_us, v.count as usize, None, 0,
    ) {
        Ok(name) => {
            STREAM.with(|s| if let Some(p) = s.borrow_mut().as_mut() { p.prefetch = handle; });
            log(format!("streaming \"{title}\" (mp4): {} frames, {}x{}, decoder={name}", v.count, v.w, v.h));
            Ok(())
        }
        Err(e) => { log(e.clone()); Err(e) }
    }
}

/// MPEG-4 sampling-frequency Hz → index (for ASC reconstruction).
fn freq_to_index(hz: u32) -> u8 {
    const T: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    T.iter().position(|&f| f == hz).unwrap_or(3) as u8
}

/// Block_on fetch of the moov region (faststart head, else tail) for H.265 hvcC —
/// the mp4 crate discards hvcC. Called from bg-tick (block_on legal there).
fn fetch_moov_for_config(url: &str, total_len: u64) -> Option<Vec<u8>> {
    let client = build_client()?;
    let head_end = (16 * 1024 * 1024).min(total_len);
    let head = wit_bindgen::rt::async_support::block_on(async {
        net::fetch_range(&client, url, 0, Some(head_end - 1)).await
    }).ok()?.bytes;
    if streaming::top_level_box(&head, b"moov").is_some() {
        return Some(head);
    }
    let tail_start = total_len.saturating_sub(32 * 1024 * 1024);
    let tail = wit_bindgen::rt::async_support::block_on(async {
        net::fetch_range(&client, url, tail_start, None).await
    }).ok()?.bytes;
    if streaming::top_level_box(&tail, b"moov").is_some() { Some(tail) } else { None }
}

/// Open an MKV/WebM stream via the oxideav-mkv `Demuxer`. SYNCHRONOUS: the demuxer
/// pulls bytes through a blocking HTTP-Range reader (block_on), which must run in a
/// sync context — so this is called from the consumer's bg-tick open path. The
/// SeekHead-directed open reads only headers + Cues (no whole-file scan).
pub fn open_mkv_sync(url: String, total_len: u64, title: String, duration_us: i64, surface: (u32, u32)) -> Result<(), String> {
    let fail = |msg: String| -> Result<(), String> {
        log(msg.clone());
        Err(msg)
    };
    // LOCAL-FILE test mode: `file://<path>` reads from disk via the SAME reader.
    let (reader, handle) = if let Some(path) = url.strip_prefix("file://") {
        match httprange::HttpRangeReader::new_local(path, total_len) {
            Ok(r) => (r, None),
            Err(e) => return fail(format!("stream: local open {path}: {e}")),
        }
    } else {
        let Some(client) = build_client() else {
            return fail("stream: no HTTP client".into());
        };
        let r = httprange::HttpRangeReader::new(url.clone(), total_len, client);
        let h = r.handle();
        (r, h)
    };
    // oxideav-mkv over the Send-wrapped reader — the wandr fork's `open_streaming`:
    // header-only open (front + SeekHead-reachable Cues parsed, but NO whole-file
    // Cluster scan). A cue-less remux over HTTP-Range therefore opens with one small
    // read instead of a per-Cluster fetch storm; seek then reports Unsupported below.
    let input: Box<dyn oxideav_core::ReadSeek> = Box::new(SendReader(reader));
    let dmx = match oxideav_mkv::demux::open_streaming(input, &NullCodecResolver) {
        Ok(d) => d,
        Err(e) => return fail(format!("stream: mkv open: {e:?}")),
    };
    let streams: Vec<oxideav_core::StreamInfo> = dmx.streams().to_vec();

    // Video config from the video stream (avcC/hvcC in extradata, like the MP4 path).
    let Some(vs) = streams.iter().find(|s| s.params.media_type == MediaType::Video) else {
        return fail("stream: mkv has no video track".into());
    };
    let vp = &vs.params;
    let (codec, video_annexb, ps_prefix, nal_len) = match vp.codec_id.as_str() {
        "h264" | "avc1" => match mkv::parse_avcc(&vp.extradata) {
            Some((pfx, n)) => (Codec::H264, true, pfx, n),
            None => return fail("stream: mkv avcC parse".into()),
        },
        "hevc" | "h265" => match mkv::parse_hvcc(&vp.extradata) {
            Some((pfx, n)) => (Codec::H265, true, pfx, n),
            None => return fail("stream: mkv hvcC parse".into()),
        },
        "vp9" => (Codec::Vp9, false, Vec::new(), 0),
        "vp8" => (Codec::Vp8, false, Vec::new(), 0),
        "av1" => (Codec::Av1, false, Vec::new(), 0),
        other => return fail(format!("stream: mkv video codec {other} unsupported")),
    };
    let (width, height) = (vp.width.unwrap_or(0), vp.height.unwrap_or(0));
    let (v_num, v_den) = (vs.time_base.num(), vs.time_base.den());
    let video_stream = vs.index;

    // All audio streams (container order) — C2's 'a' cycles them in place.
    let audio_streams: Vec<MkvAudioStream> = streams
        .iter()
        .filter(|s| s.params.media_type == MediaType::Audio)
        .map(|a| MkvAudioStream {
            index: a.index,
            codec_id: a.params.codec_id.as_str().to_string(),
            extradata: a.params.extradata.clone(),
            sample_rate: a.params.sample_rate.unwrap_or(OUT_RATE),
            channels: a.params.channels.unwrap_or(2),
            num: a.time_base.num(),
            den: a.time_base.den(),
            label: a.params.language.clone().unwrap_or_else(|| "und".to_string()),
        })
        .collect();

    // Set up the FIRST audio stream; 'a' cycles the rest in place.
    let mut audio_stream = u32::MAX;
    let mut a_num = 1i64;
    let mut a_den = OUT_RATE as i64;
    let (audio_dec, has_audio, resampler) = match audio_streams.first() {
        Some(t) => {
            let (d, ok, r) = setup_audio_by_codec(&t.codec_id, &t.extradata, t.sample_rate, t.channels);
            if ok {
                audio_stream = t.index;
                a_num = t.num;
                a_den = t.den;
                let more = if audio_streams.len() > 1 {
                    format!("  (+{} more — press 'a')", audio_streams.len() - 1)
                } else {
                    String::new()
                };
                log(format!("audio: {} {}ch @ {} Hz [{}]{more}", t.codec_id, t.channels, t.sample_rate, t.label));
                (d, ok, r)
            } else {
                log(format!("audio: {} not decodable — video only", t.codec_id));
                (None, false, None)
            }
        }
        None => (None, false, None),
    };

    let src = Box::new(MkvSource {
        dmx,
        video_stream,
        audio_stream,
        v_num,
        v_den,
        a_num,
        a_den,
        audio_streams,
    });

    match install_player(
        url, total_len, Demux::Mkv(src), ps_prefix, nal_len, video_annexb, codec, width, height,
        surface, audio_dec, has_audio, resampler, title.clone(), duration_us, 0, None, 0,
    ) {
        Ok(impl_name) => {
            STREAM.with(|s| if let Some(p) = s.borrow_mut().as_mut() { p.prefetch = handle; });
            log(format!("streaming \"{title}\" (mkv): {width}x{height}, decoder={impl_name}"));
            Ok(())
        }
        Err(e) => { log(e.clone()); Err(e) }
    }
}

/// Extract the video codec config from a DASH rep's init segment (ftyp+moov):
/// (codec, ps_prefix, nal_len, width, height, time_base num, den). Used by both the
/// initial open and the mid-stream bitrate switch.
fn video_config_from_init(init: &[u8]) -> Result<(Codec, Vec<u8>, usize, u32, u32, i64, i64), String> {
    let vdmx = oxideav_mp4::demux::open(Box::new(Cursor::new(init.to_vec())), &NullCodecResolver)
        .map_err(|e| format!("fmp4: open video init: {e:?}"))?;
    let vstream = vdmx
        .streams()
        .iter()
        .find(|s| s.params.media_type == MediaType::Video)
        .ok_or_else(|| "fmp4: no video stream".to_string())?;
    let vp = &vstream.params;
    let (codec, ps_prefix, nal_len) = match vp.codec_id.as_str() {
        "h264" | "avc1" => mkv::parse_avcc(&vp.extradata)
            .map(|(p, n)| (Codec::H264, p, n))
            .ok_or_else(|| "fmp4: avcC parse failed".to_string())?,
        "hevc" | "h265" | "hvc1" => mkv::parse_hvcc(&vp.extradata)
            .map(|(p, n)| (Codec::H265, p, n))
            .ok_or_else(|| "fmp4: hvcC parse failed".to_string())?,
        other => return Err(format!("fmp4: unsupported video codec {other}")),
    };
    Ok((
        codec, ps_prefix, nal_len,
        vp.width.unwrap_or(0), vp.height.unwrap_or(0),
        vstream.time_base.num(), vstream.time_base.den(),
    ))
}

/// ABR: switch the video Representation mid-stream. Re-opens the video decoder with
/// the new rep's config (resolution/SPS change ⇒ a genuine decoder re-init), swaps
/// the video `SegStream` to the new rep, and re-syncs to the current playback
/// position (segment-aligned, lands on a keyframe). AUDIO IS UNTOUCHED — only the
/// video bitrate changes — so the audio-master clock keeps running and the video
/// catches back up. Runs in bg-tick (the config-init fetch + do_seek do I/O).
pub fn switch_video_rep(
    new_init: Vec<u8>,
    new_segs: Vec<Seg>,
    new_starts_us: Vec<i64>,
) -> Result<(), String> {
    let (codec, ps_prefix, nal_len, width, height, num, den) = video_config_from_init(&new_init)?;
    let (sw, sh) = CONTROLS.with(|c| c.borrow().surface);
    // Open the NEW decoder before touching the stream, so a failure leaves playback
    // running on the old rep.
    let dec = VideoDecoder::open_accelerated(
        DecoderConfig { codec, width, height, rect: video_rect(sw, sh), rotation: 0, layer: ZLayer::BehindUi },
        Acceleration::NoPreference,
    )
    .map_err(|e| format!("switch: decoder open: {e:?}"))?;

    STREAM.with(|s| {
        let mut g = s.borrow_mut();
        let Some(p) = g.as_mut() else { return Err("switch: no active stream".to_string()) };
        let Demux::Fmp4(src) = &mut p.demux else { return Err("switch: not a DASH stream".to_string()) };
        let client = src.video.client.clone();
        src.video = SegStream {
            init: new_init, segs: new_segs, starts_us: new_starts_us, client,
            idx: 0, dmx: None, num, den, done: false, pf: Default::default(),
        };
        p.dec = dec;
        p.ps_prefix = ps_prefix;
        p.nal_len = nal_len;
        p.video_annexb = true;
        // Re-sync both streams to the current position on the new video rep.
        let clk = p.clock_us;
        do_seek(p, clk);
        log(format!("switched video rep → {width}x{height}"));
        Ok(())
    })
}

/// Open a DASH/CMAF stream and PLAY IT STREAMING — fetch one segment at a time as
/// playback advances (bounded memory, fast startup), instead of downloading the
/// whole rep first. Inputs per rep: the init segment bytes + the ordered media
/// segment URLs + each segment's absolute start time (µs, for seek), plus a shared
/// HTTP client. Codec config is read once from the init segment (avcC → ps_prefix/
/// nal_len; ASC → AAC decoder); each `SegStream` then fetches+opens segments on
/// demand (oxideav-mp4 walks a whole input to EOF, so we feed it ONE init+segment
/// at a time). Framing is identical to the MP4 path (AVCC → Annex-B via
/// `video_annexb`); demux_cursor stays u64::MAX (the RollingBuffer path is idle).
pub fn open_fmp4_streaming(
    video_init: Vec<u8>,
    video_segs: Vec<Seg>,
    video_starts_us: Vec<i64>,
    audio: Option<(Vec<u8>, Vec<Seg>, Vec<i64>)>,
    client: reqwest::Client,
    title: String,
    duration_us: i64,
    surface: (u32, u32),
) -> Result<(), String> {
    // ---- video config from the init segment (ftyp+moov, no fragments) ----
    let (codec, ps_prefix, nal_len, width, height, v_num, v_den) =
        video_config_from_init(&video_init)?;

    let video = SegStream {
        init: video_init,
        segs: video_segs,
        starts_us: video_starts_us,
        client: client.clone(),
        idx: 0,
        dmx: None,
        num: v_num,
        den: v_den,
        done: false,
        pf: Default::default(),
    };

    // ---- audio config from its init segment (optional) ----
    let (audio, audio_dec, has_audio, resampler) = match audio {
        Some((ainit, aurls, astarts)) => {
            match oxideav_mp4::demux::open(Box::new(Cursor::new(ainit.clone())), &NullCodecResolver) {
                Ok(admx) => {
                    let astream = admx
                        .streams()
                        .iter()
                        .find(|s| s.params.media_type == MediaType::Audio)
                        .cloned();
                    match astream {
                        Some(a) => {
                            let ap = &a.params;
                            let (a_num, a_den) = (a.time_base.num(), a.time_base.den());
                            let rate = ap.sample_rate.unwrap_or(OUT_RATE);
                            let (dec, ok, resamp) = setup_aac_audio(&ap.extradata, rate);
                            drop(admx);
                            if ok {
                                log(format!("audio: aac {}ch @ {} Hz", ap.channels.unwrap_or(2), rate));
                                let ss = SegStream {
                                    init: ainit, segs: aurls, starts_us: astarts, client: client.clone(),
                                    idx: 0, dmx: None, num: a_num, den: a_den, done: false, pf: Default::default(),
                                };
                                (Some(ss), dec, true, resamp)
                            } else {
                                log("audio: aac setup failed — video only".to_string());
                                (None, None, false, None)
                            }
                        }
                        None => {
                            log("audio: no audio stream — video only".to_string());
                            (None, None, false, None)
                        }
                    }
                }
                Err(e) => {
                    log(format!("audio: init open failed ({e:?}) — video only"));
                    (None, None, false, None)
                }
            }
        }
        None => (None, None, false, None),
    };

    let src = Box::new(Fmp4Source { video, audio });

    match install_player(
        "dash".to_string(), 0, Demux::Fmp4(src), ps_prefix, nal_len, true, codec, width, height,
        surface, audio_dec, has_audio, resampler, title.clone(), duration_us, 0, None, 0,
    ) {
        Ok(impl_name) => {
            log(format!("streaming \"{title}\" (dash/cmaf, per-segment): {width}x{height}, decoder={impl_name}"));
            Ok(())
        }
        Err(e) => {
            log(e.clone());
            Err(e)
        }
    }
}

/// MPEG-4 sampling-frequency index → Hz.
fn aac_freq_hz(idx: u8) -> Option<u32> {
    const T: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    T.get(idx as usize).copied()
}

/// Range-fetch one window and append it to the rolling buffer. Spawned by the
/// fetch driver in bg-tick; never holds the STREAM borrow across the await.
async fn fetch_window(url: String, start: u64) {
    let clear = |()| STREAM.with(|s| { if let Some(p) = s.borrow_mut().as_mut() { p.fetch_inflight = false; } });
    let Some(client) = build_client() else { return clear(()); };
    let end = start + FETCH_WINDOW - 1;
    match net::fetch_range(&client, &url, start, Some(end)).await {
        Ok(r) => STREAM.with(|s| {
            if let Some(p) = s.borrow_mut().as_mut() {
                p.buf.append(start, &r.bytes);
                p.fetch_inflight = false;
            }
        }),
        Err(e) => {
            log(format!("stream: fetch @{start} failed: {e}"));
            clear(());
        }
    }
}

/// Both container sources fetch through their own blocking Range readers, so
/// there is no external RollingBuffer to fill or reclaim — nothing to prefetch.
fn demux_cursor(_d: &Demux) -> u64 {
    u64::MAX
}

// ---- seek ------------------------------------------------------------------

/// Walk an `stts` table (entries = (sample_count, sample_delta); the crate's
/// `SttsEntry` is `pub(crate)`, so we pass the fields as tuples) → (1-based sample
/// id, its start tick) for the sample whose [start, start+delta) contains `target`.
fn stts_sample_at(entries: &[(u32, u32)], target_tick: u64) -> (u32, u64) {
    let mut sid: u32 = 1;
    let mut t: u64 = 0;
    for &(count, delta) in entries {
        let d = delta as u64;
        let span = d * count as u64;
        if d > 0 && target_tick < t + span {
            let n = (target_tick - t) / d;
            return (sid + n as u32, t + n * d);
        }
        t += span;
        sid += count;
    }
    (sid.saturating_sub(1).max(1), t)
}

/// Start tick of a 1-based `sample_id` (walk `stts` tuples).
fn stts_time_of(entries: &[(u32, u32)], sample_id: u32) -> u64 {
    let before = sample_id.saturating_sub(1); // samples before it
    let mut seen: u32 = 0;
    let mut t: u64 = 0;
    for &(count, delta) in entries {
        if seen + count >= before {
            return t + (before - seen) as u64 * delta as u64;
        }
        t += delta as u64 * count as u64;
        seen += count;
    }
    t
}

/// Current playback clock + `delta_us`, clamped to [0, duration]. Read from the
/// active stream so input handlers (which have no `nanos`) can build a seek target.
pub fn seek_from_clock(delta_us: i64) -> i64 {
    STREAM.with(|s| {
        s.borrow()
            .as_ref()
            .map(|p| (p.clock_us + delta_us).clamp(0, p.duration_us.max(0)))
            .unwrap_or(0)
    })
}

/// Seek the active stream to `target_us` (absolute movie µs). Repositions the demux
/// to a keyframe at/just-before the target (MP4: `stts`/`stss` → sample id; MKV:
/// `MatroskaFile::seek` via Cues), drops the queues + audio ring, resets the
/// decoder, and clears the A/V clock anchors so `media_now` re-anchors at the landed
/// time. Returns the landed time (µs). Runs in bg-tick (MKV seek does I/O).
pub fn do_seek(p: &mut StreamPlayer, target_us: i64) -> i64 {
    let target_us = target_us.clamp(0, p.duration_us.max(0));
    let landed_us = match &mut p.demux {
        Demux::Mp4(src) => {
            let (kf, land_us, a_sid) = {
                let tracks = src.reader.tracks();
                let vstbl = &tracks[&src.video_track_id].trak.mdia.minf.stbl;
                let v_stts: Vec<(u32, u32)> =
                    vstbl.stts.entries.iter().map(|e| (e.sample_count, e.sample_delta)).collect();
                let v_raw = (target_us - src.video_edit_us).max(0) as u64;
                let v_tick = v_raw.saturating_mul(src.timescale_v as u64) / 1_000_000;
                let (s_at, _) = stts_sample_at(&v_stts, v_tick);
                // Largest sync sample ≤ s_at (no stss ⇒ every sample is a keyframe).
                let kf = match &vstbl.stss {
                    Some(stss) => stss.entries.iter().rev().copied().find(|&s| s <= s_at).unwrap_or(1),
                    None => s_at.max(1),
                };
                let land_tick = stts_time_of(&v_stts, kf);
                let land_us = land_tick as i64 * 1_000_000 / src.timescale_v.max(1) as i64
                    + src.video_edit_us;
                // Audio: the sample at the landed time (no keyframe constraint).
                let a_sid = if src.audio_track_id != 0 {
                    let astbl = &tracks[&src.audio_track_id].trak.mdia.minf.stbl;
                    let a_stts: Vec<(u32, u32)> =
                        astbl.stts.entries.iter().map(|e| (e.sample_count, e.sample_delta)).collect();
                    let a_raw = (land_us - src.audio_edit_us).max(0) as u64;
                    let a_tick = a_raw.saturating_mul(src.timescale_a as u64) / 1_000_000;
                    stts_sample_at(&a_stts, a_tick).0
                } else {
                    1
                };
                (kf, land_us, a_sid)
            };
            src.next_v = kf;
            src.next_a = a_sid;
            Some(land_us)
        }
        Demux::Mkv(src) => {
            // us → video-stream ticks; oxideav Cues-seek returns the ACTUAL landed
            // pts, so the clock re-anchors to where we truly landed (not the request).
            let ticks = if src.v_num == 0 {
                0
            } else {
                (target_us.max(0) as i128 * src.v_den as i128 / (src.v_num as i128 * 1_000_000)) as i64
            };
            match src.dmx.seek_to(src.video_stream, ticks) {
                Ok(landed) => Some(SegStream::ticks_to_us(landed, src.v_num, src.v_den)),
                // Cue-less MKV (streaming open skipped the scan): seek is unsupported.
                // Return None so the seek is a NO-OP — do NOT move the clock or reset
                // the queues (the demuxer never moved; playback continues untouched).
                Err(e) => {
                    log(format!("seek: mkv unsupported (no Cues index): {e:?} — staying put"));
                    None
                }
            }
        }
        Demux::Fmp4(src) => Some(src.seek(target_us)),
    };
    let Some(landed_us) = landed_us else { return p.clock_us };
    // Discontinuity reset: drop queued frames + buffered PCM, the audio ring, and
    // the decoder's reorder state; clear the clock anchors so the pump re-anchors
    // `media_now` at the first frame after the seek.
    p.video_q.clear();
    p.audio_q.clear();
    p.pending_pcm.clear();
    let _ = p.dec.reset();
    with_audio(|pb| pb.flush());
    p.first_pts_us = None;
    p.origin_ns = 0;
    p.audio_pts_known = false;
    p.audio_first_pts_known = false;
    p.audio_first_pts_us = 0;
    p.audio_start_ns = 0;
    p.dev_start = 0;
    p.dev_start_set = false;
    p.submitted = 0;
    p.presented = 0;
    p.last_pres_pts = 0;
    p.flushed = false;
    p.done = false;
    p.demux_done = false;
    p.clock_us = landed_us;
    log(format!("seek → {:.1}s (asked {:.1}s)", landed_us as f64 / 1e6, target_us as f64 / 1e6));
    landed_us
}

/// Demux buffered bytes into the frame queues (bounded by the queue caps). Both
/// containers consume forward, producing already-framed video AUs + raw audio
/// packets. Queued frames own their bytes, so the buffer behind the cursor can
/// be dropped.
pub fn fill_queues(p: &mut StreamPlayer) {
    // Stop demuxing this far ahead of the current playback position. Before audio
    // starts, clock_us is ~0, which still yields a healthy startup lead.
    let limit = p.clock_us + LOOKAHEAD_US;
    let has_audio = p.audio_dec.is_some();
    // Track each track's last-produced PTS SEPARATELY. Video PTS is non-monotonic
    // (B-frame reorder), so a single video frame spiking past the limit must NOT
    // stop us before the interleaved audio (lower PTS) is produced — that starves
    // the audio ring and freezes the (audio-master) clock. Stop only when BOTH
    // tracks are demuxed past the limit.
    let mut last_v = i64::MIN;
    let mut last_a = i64::MIN;
    loop {
        // Hard backstop: stop only when BOTH queues are full. (A `||` here would
        // let a full, slowly-draining video queue block audio production entirely
        // — which froze the audio-master clock and collapsed video to ~1 fps.)
        let v_full = p.video_q.len() >= VQ_CAP;
        let a_full = p.audio_q.len() >= AQ_CAP;
        if v_full && a_full {
            break;
        }
        // Both tracks demuxed past the lookahead → done for now.
        if last_v > limit && (!has_audio || last_a > limit) {
            break;
        }
        // Produce one frame.
        let produced: Prod = match &mut p.demux {
            // mp4 crate read_sample (random access; blocks on its reader). Pull
            // whichever track is behind in PTS so both stay fed — but never a track
            // whose queue is already full (produce the other instead).
            Demux::Mp4(src) => {
                // Gate each track by BOTH queue space AND the lookahead limit, so a
                // full/slow video queue never keeps producing audio past the lead
                // (which over-buffered audio to a ~13 s lead).
                let v_left = src.next_v <= src.video_count && !v_full && last_v <= limit;
                let a_left = src.audio_track_id != 0 && src.next_a <= src.audio_count && !a_full && last_a <= limit;
                let take_v = v_left && (!a_left || last_v <= last_a);
                if take_v {
                    match src.reader.read_sample(src.video_track_id, src.next_v) {
                        Ok(Some(s)) => {
                            let pts = (s.start_time as i64 + s.rendering_offset as i64) * 1_000_000
                                / src.timescale_v as i64
                                + src.video_edit_us;
                            let data = if p.video_annexb {
                                streaming::to_annexb(&s.bytes, p.nal_len, &p.ps_prefix, s.is_sync)
                            } else {
                                s.bytes.to_vec()
                            };
                            p.video_q.push_back(VFrame { pts_us: pts, keyframe: s.is_sync, data });
                            src.next_v += 1;
                            Prod::Frame(pts, true)
                        }
                        Ok(None) => { src.next_v = src.video_count + 1; Prod::Skip }
                        Err(e) => {
                            log(format!("stream: mp4 read video {}: {e}", src.next_v));
                            src.next_v = src.video_count + 1;
                            Prod::Skip
                        }
                    }
                } else if a_left {
                    match src.reader.read_sample(src.audio_track_id, src.next_a) {
                        Ok(Some(s)) => {
                            let pts = s.start_time as i64 * 1_000_000 / src.timescale_a as i64
                                + src.audio_edit_us;
                            p.audio_q.push_back(AFrame { pts_us: pts, data: s.bytes.to_vec() });
                            src.next_a += 1;
                            Prod::Frame(pts, false)
                        }
                        Ok(None) => { src.next_a = src.audio_count + 1; Prod::Skip }
                        Err(e) => {
                            log(format!("stream: mp4 read audio {}: {e}", src.next_a));
                            src.next_a = src.audio_count + 1;
                            Prod::Skip
                        }
                    }
                } else {
                    // Neither track can produce now. Mark done only if both are
                    // truly EXHAUSTED (by sample count) — not merely queue-blocked.
                    let v_exhausted = src.next_v > src.video_count;
                    let a_exhausted = src.audio_track_id == 0 || src.next_a > src.audio_count;
                    if v_exhausted && a_exhausted {
                        p.demux_done = true;
                    }
                    Prod::Stop
                }
            }
            // oxideav-mkv: one interleaved packet stream, routed by index. Any Err
            // (incl. Eof) ends the stream. Video packets are length-prefixed AVCC
            // (→ Annex-B like the MP4 path); audio packets are raw codec frames.
            Demux::Mkv(src) => match src.dmx.next_packet() {
                Ok(pkt) => {
                    let si = pkt.stream_index;
                    if si == src.video_stream {
                        let pts = SegStream::ticks_to_us(pkt.pts.or(pkt.dts).unwrap_or(0), src.v_num, src.v_den);
                        let kf = pkt.flags.keyframe;
                        let data = if p.video_annexb {
                            streaming::to_annexb(&pkt.data, p.nal_len, &p.ps_prefix, kf)
                        } else {
                            pkt.data
                        };
                        p.video_q.push_back(VFrame { pts_us: pts, keyframe: kf, data });
                        Prod::Frame(pts, true)
                    } else if si == src.audio_stream {
                        let pts = SegStream::ticks_to_us(pkt.pts.or(pkt.dts).unwrap_or(0), src.a_num, src.a_den);
                        p.audio_q.push_back(AFrame { pts_us: pts, data: pkt.data });
                        Prod::Frame(pts, false)
                    } else {
                        Prod::Skip // other track (subtitle / inactive audio) — keep going
                    }
                }
                Err(_) => { p.demux_done = true; Prod::Stop }
            },
            // DASH/CMAF: pull whichever of the two rep demuxers is behind in PTS.
            // Each yields already-length-prefixed AVCC (video) / raw AAC (audio),
            // framed exactly like the MP4 path (video_annexb → Annex-B).
            Demux::Fmp4(src) => {
                let v_left = !src.video_done() && !v_full && last_v <= limit;
                let a_left = src.has_audio() && !src.audio_done() && !a_full && last_a <= limit;
                let take_v = v_left && (!a_left || last_v <= last_a);
                if take_v {
                    match src.next_video() {
                        Some((pts, kf, bytes)) => {
                            let data = if p.video_annexb {
                                streaming::to_annexb(&bytes, p.nal_len, &p.ps_prefix, kf)
                            } else {
                                bytes
                            };
                            p.video_q.push_back(VFrame { pts_us: pts, keyframe: kf, data });
                            Prod::Frame(pts, true)
                        }
                        None => Prod::Skip, // video EOF (video_done set) — retry loop stops via `else`
                    }
                } else if a_left {
                    match src.next_audio() {
                        Some((pts, bytes)) => {
                            p.audio_q.push_back(AFrame { pts_us: pts, data: bytes });
                            Prod::Frame(pts, false)
                        }
                        None => Prod::Skip,
                    }
                } else {
                    if src.video_done() && (!src.has_audio() || src.audio_done()) {
                        p.demux_done = true;
                    }
                    Prod::Stop
                }
            }
        };
        match produced {
            Prod::Frame(pts, true) => { last_v = last_v.max(pts); }
            Prod::Frame(pts, false) => { last_a = last_a.max(pts); }
            Prod::Skip => {}
            Prod::Stop => break,
        }
    }
}

/// One demuxer step outcome.
enum Prod {
    /// Produced a frame with (pts, is_video).
    Frame(i64, bool),
    /// Produced a frame on an ignored track — keep going.
    Skip,
    /// Can't produce more right now (buffer/EOF) — stop for this pump.
    Stop,
}

/// Audio DECODE stage — runs in bg-tick, OFF the real-time path. Drains raw
/// encoded frames from `audio_q`, decodes + resamples each to interleaved-stereo
/// f32 @ 48 kHz, and appends to the bounded `pending_pcm` buffer the pump drains.
/// The pump never sees a codec: only PCM crosses this boundary, so ONE codec-
/// agnostic output pump serves every codec (Opus/AAC/AC-3/…). Bounded — decode at
/// most ~2 s ahead, then stop (backpressure; no unbounded memory).
pub fn decode_audio(p: &mut StreamPlayer) {
    if !p.has_audio {
        return;
    }
    // ~2 s of interleaved-stereo PCM is ample cushion over the ~0.5 s device ring.
    let cap = OUT_RATE as usize * 2 /* stereo */ * 2 /* seconds */;
    while p.pending_pcm.len() < cap {
        let Some(af) = p.audio_q.pop_front() else { break };
        // Codec decode → native-rate stereo f32 (oxideav Opus/AC-3, Symphonia AAC).
        let stereo = match p.audio_dec.as_mut() {
            Some(dec) => dec.decode(&af.data),
            None => break,
        };
        if stereo.is_empty() {
            continue;
        }
        // Anchor: record the first frame's (edit-corrected) PTS for the audio-master
        // clock; the pump releases audio once media-time reaches it (pre-roll).
        if !p.audio_first_pts_known {
            p.audio_first_pts_us = af.pts_us;
            p.audio_first_pts_known = true;
        }
        // Resample native rate → 48 kHz (the resampler is a no-op / absent at 48k).
        let pcm = match p.resampler.as_mut() {
            Some(r) => r.process(&stereo),
            None => stereo,
        };
        p.pending_pcm.extend_from_slice(&pcm);
    }
}

pub fn pump_stream(nanos: u64) {
    STREAM.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(p) = guard.as_mut() else { return };
        if p.t0_ns == 0 {
            p.t0_ns = nanos;
            // Reveal the transport bar briefly when playback starts.
            CONTROLS.with(|e| e.borrow_mut().controls_until_ns = nanos + 3_000_000_000);
        }

        // 0. Pause + control-bar reveal. The pump owns applying the Controls'
        //    intents. Pause: pause/resume the wasi:audio device (pause keeps the
        //    ring; start resumes) and, on RESUME, shift the clock anchors forward by
        //    the paused duration so the audio-master / free-run `media_now` stays
        //    continuous. While paused we return early: last video frame stays, and
        //    the clock/decode/present/PCM-write are all idle. Reveal: input sets
        //    `controls_bump`; timestamp it to now + 3 s here.
        let want_pause = CONTROLS.with(|e| {
            let mut e = e.borrow_mut();
            if e.controls_bump {
                e.controls_until_ns = nanos + 3_000_000_000;
                e.controls_bump = false;
            }
            e.paused
        });
        if want_pause && !p.paused {
            p.paused = true;
            p.paused_at_ns = nanos;
            let _ = with_audio(|pb| pb.pause());
        } else if !want_pause && p.paused {
            let dt = nanos.saturating_sub(p.paused_at_ns);
            if p.audio_start_ns > 0 {
                p.audio_start_ns = p.audio_start_ns.saturating_add(dt);
            }
            p.origin_ns = p.origin_ns.saturating_add(dt);
            p.paused = false;
            let _ = with_audio(|pb| pb.start());
        }
        if p.paused {
            return;
        }

        // (Demux/fill runs in bg-tick — see the note there. on-frame is sync and
        // cannot block on the MKV reader.)

        // 1. Unified playback clock (movie µs): AUDIO-MASTER once audio is playing,
        //    else the VIDEO free-run clock. Both tracks' PTS are edit-list-corrected
        //    onto this ONE timeline (audio +pre-roll, video −trim), so the free-run →
        //    audio-master hand-off is continuous and video plays from movie-0 while
        //    a pre-rolled audio track stays silent until its (shifted) start.
        let audio_master = p.audio_pts_known && p.has_audio && p.audio_start_ns > 0;
        let media_now: i64 = if audio_master {
            let buffered_us = with_audio(|pb| pb.buffered_frames()).unwrap_or(0) as i64
                * 1_000_000 / OUT_RATE as i64;
            (p.audio_first_pts_us + nanos.saturating_sub(p.audio_start_ns) as i64 / 1000 - buffered_us).max(0)
        } else if let Some(first) = p.first_pts_us {
            first + nanos.saturating_sub(p.origin_ns) as i64 / 1000
        } else {
            0
        };

        // 2. Audio OUTPUT stage — drain already-decoded PCM to the device ring.
        //    NO codec work here: decode runs in bg-tick (`decode_audio`), so this
        //    pump is codec-agnostic and never blocks the on-frame path. Two output
        //    concerns stay here (they are about WHEN PCM hits the device):
        //      * pre-roll — hold until the clock reaches audio's (edit-shifted)
        //        first PTS, so a pre-rolled track begins together with video;
        //      * anchor — pin the audio-master clock at that instant.
        if p.has_audio && !p.dev_start_set {
            // Origin of this stream on the (cumulative) device clock.
            p.dev_start = with_audio(|pb| pb.position()).unwrap_or(0);
            p.dev_start_set = true;
        }
        if p.has_audio && p.audio_first_pts_known {
            if !p.audio_pts_known && media_now >= p.audio_first_pts_us && !p.pending_pcm.is_empty() {
                p.audio_pts_known = true;
                p.audio_start_ns = nanos;
            }
            if p.audio_pts_known && !p.pending_pcm.is_empty() {
                // Backpressure: write returns frames accepted; the rest waits for the
                // next pump (the ring caps ~0.5 s and paces itself at realtime).
                // Mute/volume is a GAIN on the slice handed to the device, applied
                // here (not in decode) so a level change takes effect on the
                // still-buffered pending PCM; the ~0.5 s already in the ring keeps
                // its gain. gain==1.0 writes the buffer directly (no copy).
                let gain = CONTROLS.with(|e| { let e = e.borrow(); if e.muted { 0.0 } else { e.volume } });
                let accepted = if (gain - 1.0).abs() < f32::EPSILON {
                    with_audio(|pb| pb.write(&p.pending_pcm)).unwrap_or(0) as usize
                } else {
                    let scaled: Vec<f32> = p.pending_pcm.iter().map(|s| s * gain).collect();
                    with_audio(|pb| pb.write(&scaled)).unwrap_or(0) as usize
                };
                let consumed = (accepted * 2).min(p.pending_pcm.len());
                p.pending_pcm.drain(0..consumed);
            }
        }

        // 3. Video: submit queued AUs, bounded by TWO cushions:
        //   * count — keep ≥ DECODE_AHEAD in flight so the decoder's reorder buffer
        //     never starves (the wandr.video.player lesson); and
        //   * TIME — never feed a frame whose PTS is more than SUBMIT_LEAD_US beyond
        //     the playback clock. Without the time cap, a stream that decodes far
        //     faster than realtime (e.g. a tiny 224×100 DASH rep) races: `presented`
        //     counts every pulled frame — including ones scheduled for the future —
        //     so the count gate alone is defeated and the guest submits the whole
        //     movie in seconds. Large video (jellyfin) is decode-bound and never hits
        //     this cap, so its pacing is unchanged. SUBMIT_LEAD_US ≫ DECODE_AHEAD
        //     frames' worth, so the reorder cushion is always satisfiable.
        //   ‼️ The TIME cap applies ONLY once the clock is ANCHORED. At stream start
        //     and right after a seek, `media_now` reads 0 until the first frame
        //     presents and seeds `first_pts_us`/the audio-master anchor — but the
        //     frames are at the seek target (e.g. 1400 s), so gating on `media_now`
        //     would reject every frame, nothing would decode, the clock would never
        //     anchor, and seek would DEADLOCK. Until it anchors, submit freely (the
        //     count cushion still bounds the burst); once anchored, apply the cap.
        const SUBMIT_LEAD_US: i64 = 2_000_000;
        let clock_anchored = p.first_pts_us.is_some() || p.audio_pts_known;
        while p.submitted < p.presented + DECODE_AHEAD {
            let Some(vf) = p.video_q.front() else { break };
            if clock_anchored && vf.pts_us > media_now + SUBMIT_LEAD_US {
                break; // far enough ahead of the clock — let realtime catch up
            }
            let frame = TimedFrame { data: vf.data.clone(), timestamp_us: vf.pts_us, keyframe: vf.keyframe };
            match p.dec.submit_timed(&frame) {
                Ok(()) => { p.submitted += 1; p.video_q.pop_front(); }
                Err(VideoError::QueueFull) => break,
                Err(e) => { log(format!("stream: submit: {e:?}")); p.video_q.pop_front(); break; }
            }
        }

        // 4. Reclaim buffer behind the demux cursor (queued frames own their bytes).
        let keep = demux_cursor(&p.demux);
        if keep != u64::MAX {
            p.buf.drop_before(keep);
        }

        // 5. EOS: flush so the decoder releases its reorder-held tail.
        if p.demux_done && p.video_q.is_empty() && !p.flushed {
            let _ = p.dec.flush();
            p.flushed = true;
        }

        // 6. Present decoded video against the unified `media_now` (computed at the
        //    top). Both tracks share ONE edit-corrected movie timeline, so video
        //    ALWAYS presents: free-run before audio starts, audio-master after —
        //    a continuous hand-off, no "hold video until audio" gate.
        const LATE_DROP_US: i64 = 150_000;
        while let Some(frame) = p.dec.next_decoded() {
            let pts = frame.timestamp_us();
            p.presented += 1; // count for decode-ahead pacing regardless
            p.last_pres_pts = pts;
            // Establish the free-run origin at the first presented frame — media_now
            // reads it until audio takes over.
            if p.first_pts_us.is_none() {
                p.first_pts_us = Some(pts);
                p.origin_ns = nanos;
            }
            // Drop frames far behind the clock so video catches up instead of
            // replaying a backlog at realtime. (Safe — decoder already produced it.)
            if pts < media_now - LATE_DROP_US {
                continue;
            }
            let at_ns = nanos.saturating_add((pts - media_now).max(0) as u64 * 1_000);
            frame.present(at_ns);
        }

        // Overlay clock = the unified playback clock; plus the audio device's own
        // played position (for the on-screen A/V-sync readout).
        p.clock_us = media_now;
        if audio_master {
            let pos = with_audio(|pb| pb.position()).unwrap_or(0);
            p.audio_pos_us = p.audio_first_pts_us
                + pos.saturating_sub(p.dev_start) as i64 * 1_000_000 / OUT_RATE as i64;
        }

        // Done when the demuxer is exhausted and the decoder has fully drained.
        if p.flushed && p.video_q.is_empty() && p.submitted == p.presented && !p.done {
            p.done = true;
            log(format!("stream: DONE — presented {} frames", p.presented));
        }
    });
}

/// Called from bg-tick while Playing: keep the buffer filled ahead of the demux
/// cursor by spawning at most one window fetch at a time. Both containers consume
/// forward, so a single forward-fetch cursor serves either.
pub fn drive_fetch() {
    let start = STREAM.with(|s| {
        let mut guard = s.borrow_mut();
        let p = guard.as_mut()?;
        if p.fetch_inflight {
            return None;
        }
        let cursor = demux_cursor(&p.demux);
        if cursor == u64::MAX || cursor >= p.total_len {
            return None; // fully demuxed
        }
        let covered_ahead = p.buf.end().saturating_sub(cursor);
        let cursor_buffered = cursor >= p.buf.base && cursor < p.buf.end();
        if cursor_buffered && covered_ahead >= FETCH_WINDOW / 2 {
            return None; // enough ahead
        }
        // Continue contiguously from the window end, unless a gap (a skipped MKV
        // element, or a seek) put the cursor outside the window.
        let contiguous = !p.buf.data.is_empty() && cursor >= p.buf.base && cursor <= p.buf.end();
        let start = if contiguous { p.buf.end() } else { cursor };
        if start >= p.total_len {
            return None;
        }
        p.fetch_inflight = true;
        Some((p.url.clone(), start))
    });
    if let Some((url, start)) = start {
        reqwest::task::spawn(fetch_window(url, start));
    }
}

// ---- render (player overlay) -----------------------------------------------

fn fill_paint(argb: u32) -> wtypes::Paint<'static> {
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

pub fn draw_rect(cv: &Canvas, x: f32, y: f32, w: f32, h: f32, argb: u32) {
    cv.draw_rect(wtypes::Rect { x, y, width: w, height: h }, &fill_paint(argb));
}

pub fn draw_text(cv: &Canvas, text: &str, x: f32, y: f32, size: f32, weight: u32, color: u32, wrap_w: f32) {
    let style = wlayout::TextStyle {
        family: "sans-serif".into(),
        size,
        weight,
        italic: false,
        color,
        letter_spacing: 0.0,
        line_height: 1.2,
        baseline_shift: 0.0,
        decoration: None,
        shadows: vec![],
        background: None,
    };
    let b = wlayout::ParagraphBuilder::new(&style);
    b.add_text(text);
    let para = wlayout::ParagraphBuilder::build(b);
    para.layout(wrap_w);
    para.paint(cv, wtypes::Point { x, y });
}

pub fn wctx<R>(f: impl FnOnce(&wembed::CanvasContext) -> R) -> R {
    WCTX.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(wembed::get_context());
        }
        f(c.borrow().as_ref().unwrap())
    })
}

pub fn fmt_dur(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Transport-bar geometry. Shared by `render_playing` (draw) and the app's pointer
/// hit-test so buttons line up with clicks. Rects are (x, y, w, h).
pub struct CtlBar {
    pub panel: (f32, f32, f32, f32),
    pub scrub: (f32, f32, f32, f32),
    pub playpause: (f32, f32, f32, f32),
    pub stop: (f32, f32, f32, f32),
    pub mute: (f32, f32, f32, f32),
    pub vol: (f32, f32, f32, f32),
}

pub fn control_bar(sw: f32, sh: f32) -> CtlBar {
    let ph = 78.0;
    let py = sh - ph;
    CtlBar {
        panel:     (0.0,        py,        sw,        ph),
        scrub:     (16.0,       py + 16.0, sw - 32.0, 6.0),
        playpause: (16.0,       py + 38.0, 34.0,      34.0),
        stop:      (58.0,       py + 38.0, 34.0,      34.0),
        mute:      (sw - 150.0, py + 40.0, 30.0,      30.0),
        vol:       (sw - 112.0, py + 50.0, 96.0,      8.0),
    }
}

pub fn hit(x: f32, y: f32, r: (f32, f32, f32, f32)) -> bool {
    x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
}

/// Right-pointing play triangle from vertical rects (font-independent).
pub fn draw_play(cv: &Canvas, x: f32, y: f32, s: f32, color: u32) {
    let n = 12usize;
    let cw = s / n as f32;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let h = s * (1.0 - t);
        draw_rect(cv, x + i as f32 * cw, y + (s - h) / 2.0, cw + 0.6, h, color);
    }
}

/// Two-bar pause glyph.
pub fn draw_pause(cv: &Canvas, x: f32, y: f32, s: f32, color: u32) {
    let bw = s * 0.28;
    draw_rect(cv, x, y, bw, s, color);
    draw_rect(cv, x + s - bw, y, bw, s, color);
}

pub fn render_playing(nanos: u64) {
    let cv = wctx(|x| x.get_current_buffer());
    cv.clear(0x0000_0000); // transparent — the decoded video shows through

    let (sw, sh) = CONTROLS.with(|e| e.borrow().surface);
    let (sw, sh) = (sw as f32, sh as f32);
    let (paused, muted, volume, controls_until_ns, scrubbing, scrub_frac, subs_on) = CONTROLS.with(|e| {
        let e = e.borrow();
        (e.paused, e.muted, e.volume, e.controls_until_ns, e.scrubbing, e.scrub_frac, e.sub_sel.is_some())
    });

    let (title, clock_us, dur_us, presented, total, aud_buf_s, audio_pos_us) = STREAM.with(|s| {
        let b = s.borrow();
        match b.as_ref() {
            Some(p) => (
                p.title.clone(), p.clock_us, p.duration_us, p.presented, p.total_video,
                // Audio buffered ahead, in seconds (queue frames + device ring).
                p.audio_q.len() as f32 * 1024.0 / OUT_RATE as f32
                    + if p.has_audio { with_audio(|pb| pb.buffered_frames()).unwrap_or(0) as f32 / OUT_RATE as f32 } else { 0.0 },
                p.audio_pos_us,
            ),
            None => (String::new(), 0, 0, 0, 0, 0.0, 0),
        }
    });

    // ---- transport chrome: title bar + bottom control panel, auto-hiding after
    //      ~3 s; any key/pointer (or being paused) reveals it.
    let show = paused || scrubbing || nanos < controls_until_ns;
    let pct = if dur_us > 0 { (clock_us as f32 / dur_us as f32).clamp(0.0, 1.0) } else { 0.0 };

    // Always-on: a thin progress sliver at the very bottom + the small A/V-sync
    // dev readout (VIDEO clock vs AUDIO device position; Δ≈0 = in sync).
    draw_rect(&cv, 0.0, sh - 3.0, sw, 3.0, 0x40FF_FFFF);
    draw_rect(&cv, 0.0, sh - 3.0, sw * pct, 3.0, 0xFF7B_FFB0);
    let diag = format!(
        "V {:.2}s | A {:.2}s | Δ {:+.2}s",
        clock_us as f64 / 1e6, audio_pos_us as f64 / 1e6,
        (audio_pos_us - clock_us) as f64 / 1e6,
    );
    draw_text(&cv, &diag, 12.0, 8.0, 12.0, 600, 0xC0FF_D060, sw - 24.0);

    if show {
        // Title bar.
        draw_rect(&cv, 0.0, 0.0, sw, 44.0, 0xCC10_1216);
        draw_text(&cv, &title, 16.0, 12.0, 20.0, 700, 0xFFFF_FFFF, sw - 120.0);
        draw_text(&cv, "Esc: list", sw - 92.0, 15.0, 14.0, 500, 0xFF8A_9098, 92.0);

        // Bottom transport panel.
        let c = control_bar(sw, sh);
        draw_rect(&cv, c.panel.0, c.panel.1, c.panel.2, c.panel.3, 0xCC10_1216);

        // Scrub: track + buffered-ahead + played + knob.
        let buf_pct = if dur_us > 0 {
            ((clock_us as f64 + aud_buf_s as f64 * 1e6) / dur_us as f64).clamp(0.0, 1.0) as f32
        } else { 0.0 };
        // While dragging, the played fill + knob preview the drag position.
        let disp_frac = if scrubbing { scrub_frac } else { pct };
        draw_rect(&cv, c.scrub.0, c.scrub.1, c.scrub.2, c.scrub.3, 0x40FF_FFFF);
        draw_rect(&cv, c.scrub.0, c.scrub.1, c.scrub.2 * buf_pct, c.scrub.3, 0x66FF_FFFF);
        draw_rect(&cv, c.scrub.0, c.scrub.1, c.scrub.2 * disp_frac, c.scrub.3,
            if scrubbing { 0xFFFF_D060 } else { 0xFF7B_FFB0 });
        let kx = c.scrub.0 + c.scrub.2 * disp_frac;
        draw_rect(&cv, kx - 3.0, c.scrub.1 - 4.0, 6.0, c.scrub.3 + 8.0, 0xFFFF_FFFF);

        // Times + frame counter. While scrubbing, show the DRAG-TARGET time.
        let frames = if total > 0 { format!("{presented}/{total}") } else { format!("{presented}") };
        let tline = if scrubbing {
            format!("→ {} / {}", fmt_dur(scrub_frac as f64 * dur_us as f64 / 1e6), fmt_dur(dur_us as f64 / 1e6))
        } else {
            format!(
                "{} / {}   ·   {frames} fr   ·   {:.1}s buf",
                fmt_dur(clock_us as f64 / 1e6), fmt_dur(dur_us as f64 / 1e6), aud_buf_s
            )
        };
        draw_text(&cv, &tline, c.scrub.0, c.scrub.1 + 11.0, 12.0, 400, 0xFFB0_B4BC, sw - 32.0);

        // Play/pause button — icon shows the ACTION (play when paused, else pause).
        draw_rect(&cv, c.playpause.0, c.playpause.1, c.playpause.2, c.playpause.3, 0x33FF_FFFF);
        if paused {
            draw_play(&cv, c.playpause.0 + 11.0, c.playpause.1 + 8.0, 18.0, 0xFFFF_FFFF);
        } else {
            draw_pause(&cv, c.playpause.0 + 10.0, c.playpause.1 + 8.0, 18.0, 0xFFFF_FFFF);
        }
        // Stop button (filled square).
        draw_rect(&cv, c.stop.0, c.stop.1, c.stop.2, c.stop.3, 0x33FF_FFFF);
        draw_rect(&cv, c.stop.0 + 10.0, c.stop.1 + 10.0, 14.0, 14.0, 0xFFFF_FFFF);

        // Mute button + volume slider (red when muted).
        let vcol: u32 = if muted { 0xFFE0_5050 } else { 0xFFFF_FFFF };
        draw_rect(&cv, c.mute.0, c.mute.1, c.mute.2, c.mute.3, 0x33FF_FFFF);
        draw_rect(&cv, c.mute.0 + 9.0, c.mute.1 + 10.0, 12.0, 10.0, vcol);
        draw_rect(&cv, c.vol.0, c.vol.1, c.vol.2, c.vol.3, 0x40FF_FFFF);
        let vlev = if muted { 0.0 } else { volume };
        draw_rect(&cv, c.vol.0, c.vol.1, c.vol.2 * vlev, c.vol.3, vcol);
    }

    // Subtitles: the active cue at the current clock, bottom-center, lifted above
    // the control bar when it's visible. Centering is approximate (no text-measure
    // API): estimate width at ~0.52·fontsize per char.
    if subs_on {
        let cue = SUBTITLES.with(|s| {
            s.borrow().iter().find(|c| clock_us >= c.start_us && clock_us < c.end_us).map(|c| c.text.clone())
        });
        if let Some(text) = cue {
            let lines: Vec<&str> = text.lines().collect();
            let fs = 22.0_f32;
            let lh = fs + 8.0;
            let bottom = sh - if show { 90.0 } else { 28.0 };
            let base_y = (bottom - lines.len() as f32 * lh).max(48.0);
            for (i, ln) in lines.iter().enumerate() {
                let y = base_y + i as f32 * lh;
                let w = (ln.chars().count() as f32 * fs * 0.52).min(sw - 24.0);
                let x = ((sw - w) / 2.0).max(12.0);
                draw_rect(&cv, x - 8.0, y - 2.0, w + 16.0, lh, 0xA000_0000);
                draw_text(&cv, ln, x, y, fs, 600, 0xFFFF_FFFF, sw - 24.0);
            }
        }
    }

    // Paused overlay: a centered pill with a font-independent two-bar pause glyph.
    if paused {
        let (pw, ph) = (150.0_f32, 46.0_f32);
        let px = (sw - pw) / 2.0;
        let py = (sh - ph) / 2.0;
        draw_rect(&cv, px, py, pw, ph, 0xB01A_1D22);
        draw_rect(&cv, px + 20.0, py + 13.0, 7.0, 20.0, 0xFFFF_FFFF);
        draw_rect(&cv, px + 32.0, py + 13.0, 7.0, 20.0, 0xFFFF_FFFF);
        draw_text(&cv, "PAUSED", px + 54.0, py + 14.0, 18.0, 700, 0xFFFF_FFFF, pw - 54.0);
    }

    drop(cv);
    wctx(|x| x.present());
}
