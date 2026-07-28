//! wandr.jellyfin — task 119 Part A. A real Jellyfin streaming client.
//!
//! Two halves, exactly the two shipped precedents fused:
//!   * the ASYNC ENGINE runs in `bg-tick` (like wandr.audio.player): it does all
//!     HTTPS through wandr-reqwest's p3 wasi:tls backend — Quick Connect pairing,
//!     library browse, PlaybackInfo/DirectPlay negotiation, and (A2) byte-range
//!     media fetch. Long/looping work is detached with `reqwest::task::spawn`, so
//!     a tick returns promptly and the UI stays live.
//!   * the RENDER LOOP runs in `on-frame` (like wandr.video.player): it draws the
//!     pairing screen / browse list / player overlay on wasi:canvas, and (A2)
//!     composites the host-decoded video surface behind the UI.
//!
//! A1 (this file today): pair → persist token to /state → browse → resolve a
//! DirectPlay stream URL, all verifiable from the logs + on-screen. A2 wires the
//! wandr:video decoder + Symphonia audio + present(at-ns)/position() A/V sync at
//! the marked seams.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    world: "wandr:jellyfin/jellyfin-app",
    path: "wit",
    generate_all,
});

mod jellyfin;
mod streaming;
mod mkv; // parse_avcc / parse_hvcc codec-config helpers (demux via matroska-demuxer)
mod httprange;
use jellyfin::{Item, Playback, Session};

use crate::wandr::video::decoder::VideoDecoder;
use crate::wandr::video::types::{Codec, DecoderConfig, TimedFrame, VideoError, VideoRect, ZLayer};
use crate::wandr::video::decoder::Acceleration;
use crate::wasi::audio::pcm as wpcm;
use symphonia::core::audio::{Channels, SampleBuffer};
use symphonia::core::codecs::{CodecParameters, Decoder, DecoderOptions, CODEC_TYPE_AAC};
use symphonia::core::formats::Packet;
use matroska_demuxer::TrackType;

/// The wasi:audio device rate. Symphonia output is resampled to this; the device
/// exposes only mono/stereo, so multichannel is downmixed to stereo first.
const OUT_RATE: u32 = 48_000;

use crate::exports::wandr::background::background::Guest as BgGuest;
use crate::exports::wandr::ui_shell::frame_pacing::Guest as PacingGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::key_handler::{Guest as KeyGuest, KeyEvent};
use crate::exports::wasi::input_handlers::pointer_handler::{
    Button, Guest as PointerGuest, Kind as PtrKind, PointerEvent,
};
use crate::wasi::canvas::draw::Canvas;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::types as wtypes;

use std::cell::RefCell;
use std::time::Duration;

const STATE_DIR: &str = "/state/jellyfin";
const SESSION_PATH: &str = "/state/jellyfin/session.json";
const DEVICE_ID_PATH: &str = "/state/jellyfin/device_id";
/// Optional override for `DEFAULT_SERVER` — write a bare URL here to point the
/// client at a different server without touching code (keeps the default a
/// single, overridable source of truth rather than a baked-in policy value).
const SERVER_OVERRIDE_PATH: &str = "/state/jellyfin/server";

/// Where the engine is in the pair → browse → resolve → play lifecycle. The
/// render loop reads this to decide what to draw; the async engine advances it.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Nothing started yet; the first bg-tick spawns the driver.
    Boot,
    /// Showing the Quick Connect code; polling for approval.
    Pairing,
    /// Authenticated; fetching the library.
    LoadingLibrary,
    /// Library shown; user navigates and picks an item.
    Browse,
    /// PlaybackInfo in flight for the selected item.
    Resolving,
    /// DirectPlay URL resolved; fetching the moov + opening the decoder.
    Ready,
    /// Streaming: range-fetch → demux → decode → present on screen.
    Playing,
    /// A fatal step failed; message is the last log line.
    Failed,
}

struct Engine {
    phase: Phase,
    session: Option<Session>,
    /// The Quick Connect code to type into the Jellyfin web UI (Pairing phase).
    pairing_code: Option<String>,
    items: Vec<Item>,
    selected: usize,
    /// Set by on-key (Enter): the engine picks it up next tick and resolves it.
    /// Kept as an index so on-key never touches the network.
    pending_resolve: Option<usize>,
    /// A1 proof: the last resolved (item, playback, stream URL).
    resolved: Option<(Item, Playback, String)>,
    /// True once the driver task has been spawned (spawn exactly once).
    driver_spawned: bool,
    /// True while a resolve task is in flight (so we don't double-spawn).
    resolving: bool,
    /// On-screen status ring; also mirrored to println! for headless checks.
    log: Vec<String>,
    surface: (u32, u32),
    /// Geometry of the browse list as last rendered — so on-pointer can hit-test
    /// a click to a row. (top-y, row-height, first-visible-index, visible-count).
    list_view: (f32, f32, usize, usize),
    /// Accumulated scroll delta (surface units) until it crosses one row.
    scroll_accum: f32,
    /// Set by Escape/Backspace while Playing; bg-tick tears the stream down and
    /// returns to the list (on-key must not drop the decoder itself).
    stop_requested: bool,
    /// A resolved title awaiting a SYNCHRONOUS open in bg-tick — both container
    /// demuxers (mp4 crate / matroska-demuxer) pull through a block_on Range
    /// reader, which is only legal in the async bg-tick, not on-frame.
    /// (url, total_len, item, surface, is_mkv).
    pending_open: Option<(String, u64, Item, (u32, u32), bool)>,

    // ---- playback controls (Part B, task 119) -----------------------------
    // Command channel: input handlers set these; the pump reads them each tick
    // and applies them to the active StreamPlayer / wasi:audio device. Input and
    // engine share the single store thread, so plain fields (no queue) are safe.
    /// Transport paused (user intent). The pump pauses the audio device and
    /// freezes the clock; resume shifts the anchors so `media_now` stays continuous.
    paused: bool,
    /// Muted (gain 0) — independent of `volume`, so unmute restores the level.
    muted: bool,
    /// Output gain, 0.0..=1.0. Applied to PCM at the ring-write.
    volume: f32,
    /// Transport bar auto-hide (A2): visible while `nanos < controls_until_ns`.
    /// Input sets `controls_bump`; the pump (which has the host clock) timestamps
    /// it to `now + 3 s` — input handlers get no `nanos`, hence the bump indirection.
    controls_until_ns: u64,
    controls_bump: bool,
    /// Absolute seek target (movie µs, Chunk B). Input sets it; bg-tick executes
    /// the reposition + reset (MKV seek does blocking I/O, so it can't run on-frame).
    seek_request: Option<i64>,
    /// Scrub drag (B2): true while dragging the scrub bar; `scrub_frac` (0..1) is the
    /// live drag position. The playhead previews `scrub_frac` during the drag; the
    /// seek commits on pointer-up.
    scrubbing: bool,
    scrub_frac: f32,
    /// B3: media-time (µs) of the last progress report to Jellyfin — throttles
    /// reporting to ~10 s intervals; reset to 0 when a new stream starts.
    last_report_us: i64,
    /// Subtitles (Tier 3): selected track = index into `resolved.1.subtitles`
    /// (None = off); `sub_dirty` flags a change for bg-tick to (re)fetch the VTT.
    sub_sel: Option<usize>,
    sub_dirty: bool,
}

impl Engine {
    const fn new() -> Self {
        Engine {
            phase: Phase::Boot,
            session: None,
            pairing_code: None,
            items: Vec::new(),
            selected: 0,
            pending_resolve: None,
            resolved: None,
            driver_spawned: false,
            resolving: false,
            log: Vec::new(),
            surface: (520, 1040),
            list_view: (0.0, 46.0, 0, 0),
            scroll_accum: 0.0,
            stop_requested: false,
            pending_open: None,
            paused: false,
            muted: false,
            volume: 1.0,
            controls_until_ns: 0,
            controls_bump: false,
            seek_request: None,
            scrubbing: false,
            scrub_frac: 0.0,
            last_report_us: 0,
            sub_sel: None,
            sub_dirty: false,
        }
    }
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::new());
    static WCTX: RefCell<Option<wembed::CanvasContext>> = const { RefCell::new(None) };
}

/// Log to both the on-screen ring (last 12 lines) and stdout (headless proof).
fn log(msg: impl Into<String>) {
    let msg = msg.into();
    println!("jellyfin: {msg}");
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        e.log.push(msg);
        let n = e.log.len();
        if n > 12 {
            e.log.drain(0..n - 12);
        }
    });
}

fn set_phase(p: Phase) {
    ENGINE.with(|e| e.borrow_mut().phase = p);
}

// ---- /state persistence (plain std::fs over the /state preopen) ------------

fn ensure_state_dir() {
    let _ = std::fs::create_dir_all(STATE_DIR);
}

fn load_session() -> Option<Session> {
    let bytes = std::fs::read(SESSION_PATH).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Session::from_json(&v)
}

fn save_session(s: &Session) -> std::io::Result<()> {
    ensure_state_dir();
    std::fs::write(SESSION_PATH, serde_json::to_vec_pretty(&s.to_json()).unwrap())
}

fn load_server_override() -> Option<String> {
    let s = std::fs::read_to_string(SERVER_OVERRIDE_PATH).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// A stable per-install device id (part of the MediaBrowser auth header).
/// Generated once from the wall clock and kept — derived from a runtime input,
/// not a hardcoded constant.
fn load_or_make_device_id() -> String {
    if let Ok(s) = std::fs::read_to_string(DEVICE_ID_PATH) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("wandr-{nanos:032x}");
    ensure_state_dir();
    let _ = std::fs::write(DEVICE_ID_PATH, &id);
    id
}

/// True for items the shipped pipeline can DirectPlay end-to-end: a video codec
/// the host GStreamer decoder handles AND an audio codec Symphonia decodes.
/// (Dolby ac3/eac3/truehd and Opus are excluded — no Symphonia decoder — so the
/// first proof lands on AAC/MP3 titles; that is a client filter, not a server
/// transcode.)
fn is_playable(it: &Item) -> bool {
    // Container + video-codec gate. Audio is best-effort: a title with a
    // supported video codec still plays (video-only) when its audio is 5.1 AAC /
    // Opus / Dolby that the guest can't decode — so audio does NOT gate here.
    let container_ok = matches!(it.container.as_str(), "mp4" | "mov" | "m4v" | "qt" | "mkv" | "webm" | "");
    let video_ok = matches!(it.video_codec.as_str(), "h264" | "hevc" | "h265" | "vp9" | "vp8" | "av1");
    container_ok && video_ok
}

// ---- the async engine ------------------------------------------------------

fn build_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("wandr-jellyfin/0.1 ( https://codeberg.org/harryzz/wandr )")
        .build()
        .ok()
}

/// LOCAL-FILE test: if `/state/jellyfin/local_mp4.txt` names a readable file,
/// return (guest_path, size) so the driver plays it through the SAME pipeline
/// with disk as the transport instead of HTTP.
fn local_test_file() -> Option<(String, u64)> {
    let raw = match std::fs::read_to_string("/state/jellyfin/local_mp4.txt") {
        Ok(s) => s,
        Err(e) => { log(format!("local-test: no config (/state/jellyfin/local_mp4.txt): {e}")); return None; }
    };
    let path = raw.trim().to_string();
    if path.is_empty() {
        return None;
    }
    match std::fs::metadata(&path) {
        Ok(m) => Some((path, m.len())),
        Err(e) => { log(format!("local-test: config points to {path} but metadata failed: {e}")); None }
    }
}

/// The one-shot driver: authenticate (from /state or via Quick Connect), then
/// browse the library. Spawned once from the first bg-tick.
async fn driver() {
    // Transport-exclusion test: play a LOCAL file (same demux/audio/clock, disk
    // transport) and skip auth/browse entirely.
    if let Some((path, size)) = local_test_file() {
        // Sniff the container from the first bytes: EBML magic 1A 45 DF A3 = MKV.
        let is_mkv = {
            use std::io::Read;
            std::fs::File::open(&path).ok().and_then(|mut f| {
                let mut m = [0u8; 4];
                f.read_exact(&mut m).ok().map(|_| m == [0x1A, 0x45, 0xDF, 0xA3])
            }).unwrap_or(false)
        };
        log(format!("LOCAL TEST: playing {path} ({size} B, {}) — HTTP transport bypassed",
            if is_mkv { "mkv" } else { "mp4" }));
        let item = Item {
            id: String::new(), name: "LOCAL FILE".into(),
            media_source_id: String::new(), container: if is_mkv { "mkv" } else { "mp4" }.into(),
            video_codec: "h264".into(), audio_codec: String::new(),
            size, run_time_ticks: 0, image_tag: None, resume_ticks: 0,
        };
        let surface = ENGINE.with(|e| e.borrow().surface);
        set_phase(Phase::Ready);
        ENGINE.with(|e| {
            e.borrow_mut().pending_open = Some((format!("file://{path}"), size, item, surface, is_mkv));
        });
        return;
    }

    let Some(client) = build_client() else {
        log("could not build HTTP client");
        set_phase(Phase::Failed);
        return;
    };

    // Auth: reuse a saved session, else pair with Quick Connect.
    let session = match load_session() {
        Some(s) => {
            log(format!("loaded session for user {} @ {}", s.user_id, s.server_url));
            s
        }
        None => match pair(&client).await {
            Ok(s) => s,
            Err(e) => {
                log(format!("pairing failed: {e}"));
                set_phase(Phase::Failed);
                return;
            }
        },
    };
    ENGINE.with(|e| e.borrow_mut().session = Some(session.clone()));

    // Browse.
    set_phase(Phase::LoadingLibrary);
    log("browsing library…");
    match jellyfin::browse_movies(&client, &session, 200).await {
        Ok(items) => {
            let playable = items.iter().filter(|i| is_playable(i)).count();
            log(format!("library: {} movies ({} DirectPlay-able here)", items.len(), playable));
            ENGINE.with(|e| {
                let mut e = e.borrow_mut();
                // Sort playable first so the default selection lands on one.
                let mut items = items;
                items.sort_by_key(|i| !is_playable(i));
                e.selected = 0;
                e.items = items;
            });
            set_phase(Phase::Browse);
        }
        Err(e) => {
            log(format!("browse failed: {e}"));
            set_phase(Phase::Failed);
        }
    }
}

/// Quick Connect pairing: initiate → show code → poll → exchange → persist.
async fn pair(client: &reqwest::Client) -> Result<Session, String> {
    let server = load_server_override().unwrap_or_else(|| jellyfin::DEFAULT_SERVER.to_string());
    let device_id = load_or_make_device_id();
    log(format!("pairing with {server} (Quick Connect)…"));

    if !jellyfin::qc_enabled(client, &server).await {
        return Err("Quick Connect is disabled on the server".into());
    }
    let qc = jellyfin::qc_initiate(client, &server, &device_id).await?;
    log(format!("Quick Connect code: {}  — approve it in Jellyfin", qc.code));
    ENGINE.with(|e| e.borrow_mut().pairing_code = Some(qc.code.clone()));
    set_phase(Phase::Pairing);

    // Poll ~5 min (150 × 2 s).
    for _ in 0..150 {
        if jellyfin::qc_poll(client, &server, &device_id, &qc.secret).await? {
            let (token, user_id) = jellyfin::qc_exchange(client, &server, &device_id, &qc.secret).await?;
            let s = Session { server_url: server, user_id, device_id, access_token: token };
            save_session(&s).map_err(|e| format!("save session: {e}"))?;
            ENGINE.with(|e| e.borrow_mut().pairing_code = None);
            log(format!("paired ✓ user {} — token saved to {SESSION_PATH}", s.user_id));
            return Ok(s);
        }
        reqwest::task::sleep(Duration::from_secs(2)).await;
    }
    Err("Quick Connect timed out (code not approved)".into())
}

/// Resolve one item's DirectPlay stream URL (spawned when the user hits Enter).
/// A1's proof: it logs SupportsDirectPlay + the raw stream URL. A2 hands that URL
/// to the range-fetch → demux → decode path instead of just logging it.
async fn resolve(index: usize) {
    let (client, session, item) = ENGINE.with(|e| {
        let e = e.borrow();
        (build_client(), e.session.clone(), e.items.get(index).cloned())
    });
    let (Some(client), Some(session), Some(item)) = (client, session, item) else {
        ENGINE.with(|e| e.borrow_mut().resolving = false);
        return;
    };

    set_phase(Phase::Resolving);
    log(format!("resolving \"{}\" ({} / {} / {})", item.name, item.container, item.video_codec, item.audio_codec));

    match jellyfin::playback_info(&client, &session, &item.id).await {
        Ok(pb) => {
            let url = jellyfin::stream_url(&session, &item.id, &pb.media_source_id, &pb.container, &pb.play_session_id);
            log(format!(
                "DirectPlay={} transcode={} — {}",
                pb.direct_play,
                pb.transcode_url.as_deref().unwrap_or("none"),
                // Log the URL without the token (it carries api_key=<token>).
                url.split("&api_key=").next().unwrap_or(&url)
            ));
            ENGINE.with(|e| {
                let mut e = e.borrow_mut();
                e.resolved = Some((item.clone(), pb.clone(), url.clone()));
                e.resolving = false;
            });
            if pb.direct_play && pb.transcode_url.is_none() {
                log("✓ DirectPlay negotiated — starting stream");
                set_phase(Phase::Ready);
                let surface = ENGINE.with(|e| e.borrow().surface);
                // A2: fetch moov → parse sample table → open decoder → stream.
                reqwest::task::spawn(prepare_stream(url, item, surface));
            } else {
                log("⚠ server would transcode — NOT the DirectPlay proof (item skipped)");
                set_phase(Phase::Browse);
            }
        }
        Err(e) => {
            log(format!("PlaybackInfo failed: {e}"));
            ENGINE.with(|en| en.borrow_mut().resolving = false);
            set_phase(Phase::Browse);
        }
    }
}

// ---- streaming playback (A2) -----------------------------------------------

/// Bytes to pull per range request. Larger = fewer TLS handshakes (wandr-reqwest
/// closes the connection per request), at the cost of memory held in the window.
const FETCH_WINDOW: u64 = 8 * 1024 * 1024;
/// Decode-ahead cushion in frames — must exceed the decoder's reorder depth or
/// playback deadlocks (the lesson baked into wandr.video.player's DECODE_AHEAD).
const DECODE_AHEAD: usize = 20;

/// A live streaming session: the video sample table + the decoder + the rolling
/// mdat window + present bookkeeping. Holds the (!Clone) decoder resource, so it
/// lives in its own thread-local rather than in `Engine`.
use std::collections::VecDeque;

/// One demuxed video frame — already Annex-B framed (H.264/5) or raw (VP9/AV1),
/// ready to submit with its presentation time.
struct VFrame {
    pts_us: i64,
    keyframe: bool,
    data: Vec<u8>,
}
/// One demuxed audio frame — a raw codec packet (AAC) with its presentation time.
struct AFrame {
    pts_us: i64,
    data: Vec<u8>,
}

/// Container-specific demux state. Both containers feed the same frame queues, so
/// everything downstream (decode, audio, A/V sync) is container-agnostic.
enum Demux {
    /// MP4/MOV: the `mp4` crate over a blocking HTTP-Range reader (`read_sample`
    /// fetches through it). Random-access, so we pull whichever track is behind.
    Mp4(Box<Mp4Source>),
    /// MKV/WebM: the matroska-demuxer library over a blocking HTTP-Range reader.
    /// It handles all the container detail (lacing, block groups, Cues seek) that a
    /// hand-rolled parser misses. Boxed because MatroskaFile is large.
    Mkv(Box<MkvSource>),
}

/// A live `mp4`-crate demux session over the blocking Range reader.
struct Mp4Source {
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

/// A live matroska-demuxer session: the demuxer over our blocking Range reader,
/// a reusable Frame, and the track numbers to route.
struct MkvSource {
    file: matroska_demuxer::MatroskaFile<httprange::HttpRangeReader>,
    frame: matroska_demuxer::Frame,
    video_track: u64,
    audio_track: u64,
    /// Nanoseconds per timestamp tick (Frame.timestamp is in ticks).
    ts_scale_ns: u64,
}

struct StreamPlayer {
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

    // ---- audio (A2b) ----
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

thread_local! {
    static STREAM: RefCell<Option<StreamPlayer>> = const { RefCell::new(None) };
    /// The ONE audio output device, opened lazily and kept open for the app's
    /// lifetime — reused (with `flush` between streams) rather than reopened.
    /// Reopening a wasi:audio device per stream churns COM on Windows/WASAPI and
    /// breaks the second playback's audio (audio.player keeps one device too).
    static AUDIO_DEV: RefCell<Option<wpcm::Playback>> = const { RefCell::new(None) };
}

/// Open the shared audio device once (48 kHz stereo). Returns false if the host
/// has no audio output. Idempotent — subsequent calls reuse the open device.
fn ensure_audio_device() -> bool {
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
fn with_audio<R>(f: impl FnOnce(&wpcm::Playback) -> R) -> Option<R> {
    AUDIO_DEV.with(|d| d.borrow().as_ref().map(f))
}

/// 16:9 letterbox centered vertically, full width — one source of truth so open
/// and resize place the video identically (mirrors wandr.video.player).
fn video_rect(w: u32, h: u32) -> VideoRect {
    let vh = (w * 9 / 16).min(h);
    VideoRect { x: 0, y: (h.saturating_sub(vh)) / 2, width: w, height: vh }
}

/// Linear resampler src→48 kHz on interleaved frames (ported verbatim from
/// wandr.audio.player — same simple, low-cost interpolation).
struct LinearResampler {
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

/// Probe the head, sniff the container, and dispatch to the MP4 or MKV setup.
async fn prepare_stream(url: String, item: Item, surface: (u32, u32)) {
    let Some(client) = build_client() else {
        log("stream: no HTTP client");
        return set_phase(Phase::Browse);
    };
    let probe = match jellyfin::fetch_range(&client, &url, 0, Some(65_535)).await {
        Ok(r) => r,
        Err(e) => {
            log(format!("stream: head probe: {e}"));
            return set_phase(Phase::Browse);
        }
    };
    let total_len = probe.total_len;
    // EBML magic 1A 45 DF A3 = Matroska/WebM; otherwise ISO-BMFF (MP4/MOV). Both
    // demuxers pull through a block_on reader, so the actual open must happen in
    // the async bg-tick (not on-frame). Hand off via pending_open.
    let is_mkv = probe.bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]);
    log(format!("opening via library demux ({})…", if is_mkv { "mkv" } else { "mp4" }));
    ENGINE.with(|e| e.borrow_mut().pending_open = Some((url, total_len, item, surface, is_mkv)));
}

/// Common tail: open the decoder, set up audio, install the StreamPlayer, Playing.
#[allow(clippy::too_many_arguments)]
// ---- subtitles (Tier 3: Jellyfin VTT overlay) ------------------------------

/// One timed subtitle cue (µs).
struct Cue { start_us: i64, end_us: i64, text: String }

thread_local! {
    /// The currently-loaded subtitle track's cues (sorted by start). Swapped
    /// wholesale when the track changes/clears; read by render_playing.
    static SUBTITLES: RefCell<Vec<Cue>> = const { RefCell::new(Vec::new()) };
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
fn parse_vtt(text: &str) -> Vec<Cue> {
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

/// Fetch + parse a subtitle track into SUBTITLES (spawned from bg-tick).
async fn fetch_subtitles(client: reqwest::Client, vtt_url: String) {
    match jellyfin::fetch_vtt(&client, vtt_url).await {
        Ok(text) => {
            let cues = parse_vtt(&text);
            let n = cues.len();
            SUBTITLES.with(|s| *s.borrow_mut() = cues);
            log(format!("subtitles: {n} cues loaded"));
        }
        Err(e) => log(format!("subtitles: {e}")),
    }
}

// ---- Jellyfin session reporting + resume (B3) -------------------------------

enum Report { Playing, Progress, Stopped }

/// Spawn a `/Sessions/Playing{,/Progress,/Stopped}` report for the active stream.
/// Reads the resolved item/playback + session; no-ops if nothing is playing.
fn spawn_report(kind: Report, position_us: i64, paused: bool) {
    let info = ENGINE.with(|e| {
        let e = e.borrow();
        e.resolved.clone().map(|(item, pb, _)| (item, pb)).zip(e.session.clone())
    });
    let (Some(((item, pb), session)), Some(client)) = (info, build_client()) else { return };
    let (id, msid, psid, ticks) =
        (item.id, pb.media_source_id, pb.play_session_id, position_us.max(0) * 10);
    match kind {
        Report::Playing => { reqwest::task::spawn(jellyfin::report_playing(client, session, id, msid, psid, ticks)); }
        Report::Progress => { reqwest::task::spawn(jellyfin::report_progress(client, session, id, msid, psid, ticks, paused)); }
        Report::Stopped => { reqwest::task::spawn(jellyfin::report_stopped(client, session, id, msid, psid, ticks)); }
    }
}

/// Called when a fresh stream reaches Playing (B3): resume to the saved position
/// (only for a meaningful mid-file point) and report playback start.
fn on_stream_started() {
    ENGINE.with(|e| e.borrow_mut().last_report_us = 0);
    if let Some(item) = ENGINE.with(|e| e.borrow().resolved.clone().map(|(i, _, _)| i)) {
        let dur_us = (item.duration_s() * 1e6) as i64;
        let resume_us = item.resume_ticks / 10; // 100 ns ticks → µs
        if resume_us > 5_000_000 && (dur_us == 0 || resume_us < dur_us * 9 / 10) {
            ENGINE.with(|e| e.borrow_mut().seek_request = Some(resume_us));
            log(format!("resume: seeking to {:.0}s", resume_us as f64 / 1e6));
        }
    }
    spawn_report(Report::Playing, 0, false);
}

fn install_player(
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
    set_phase(Phase::Playing);
    Ok(impl_name)
}

/// AAC-LC only (Symphonia's limit). Returns (decoder, has_audio, resampler).
/// The output device is the shared persistent one (opened once) — not per stream.
/// The guest's audio decoders behind one interface. AAC/MP3 via Symphonia; Opus
/// and AC-3/E-AC-3 via the pure-Rust OxideAV crates (Symphonia has neither). Each
/// variant decodes one packet to interleaved STEREO f32 at the codec's native rate;
/// the StreamPlayer's `resampler` then converts that rate → 48 kHz.
enum AudioDec {
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
    match ropus::Decoder::new(OUT_RATE as u32, rch) {
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

/// MP4/MOV setup: fetch the moov index, parse the sample table, install.
/// Open an MP4/MOV stream via the `mp4` crate over the blocking Range reader
/// (`read_sample` fetches through it). SYNCHRONOUS (block_on) — called from
/// bg-tick. Replaces the hand-rolled sample-table demuxer.
fn open_mp4_sync(url: String, total_len: u64, item: Item, surface: (u32, u32)) {
    const START: [u8; 4] = [0, 0, 0, 1];
    let fail = |msg: String| {
        log(msg);
        set_phase(Phase::Browse);
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
    let mut mp4r = match mp4::Mp4Reader::read_header(reader, total_len) {
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

    let title = item.name.clone();
    let duration_us = (item.duration_s() * 1e6) as i64;
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
            on_stream_started(); // B3: resume + report /Sessions/Playing
        }
        Err(e) => { log(e); set_phase(Phase::Browse); }
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
        jellyfin::fetch_range(&client, url, 0, Some(head_end - 1)).await
    }).ok()?.bytes;
    if streaming::top_level_box(&head, b"moov").is_some() {
        return Some(head);
    }
    let tail_start = total_len.saturating_sub(32 * 1024 * 1024);
    let tail = wit_bindgen::rt::async_support::block_on(async {
        jellyfin::fetch_range(&client, url, tail_start, None).await
    }).ok()?.bytes;
    if streaming::top_level_box(&tail, b"moov").is_some() { Some(tail) } else { None }
}

/// MKV/WebM setup: fetch the EBML header region, parse Info + Tracks, install.
/// Open an MKV/WebM stream via the matroska-demuxer library. SYNCHRONOUS: the
/// library pulls bytes through a blocking HTTP-Range reader (block_on), which must
/// run in a sync context — so this is called from bg-tick's open path. It handles
/// all the container detail (lacing, block groups, Cues seek) a hand-rolled parser missed.
fn open_mkv_sync(url: String, total_len: u64, item: Item, surface: (u32, u32)) {
    let fail = |msg: String| {
        log(msg);
        set_phase(Phase::Browse);
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
    let mut file = match matroska_demuxer::MatroskaFile::open(reader) {
        Ok(f) => f,
        Err(e) => return fail(format!("stream: mkv open: {e:?}")),
    };
    let ts_scale_ns = file.info().timestamp_scale().get();

    // Pull everything we need out of the (borrowed) track list, then release the
    // borrow so `file` can move into MkvSource.
    let (codec_id, codec_private, width, height, video_track_num, default_dur_ns, audio_info) = {
        let tracks = file.tracks();
        let Some(v) = tracks.iter().find(|t| t.track_type() == TrackType::Video) else {
            return fail("stream: mkv has no video track".into());
        };
        let audio_info = tracks.iter().find(|t| t.track_type() == TrackType::Audio).map(|a| {
            (
                a.codec_id().to_string(),
                a.codec_private().map(|c| c.to_vec()).unwrap_or_default(),
                a.audio().map(|au| au.sampling_frequency() as u32).unwrap_or(OUT_RATE),
                a.audio().map(|au| au.channels().get()).unwrap_or(2),
                a.track_number().get(),
            )
        });
        (
            v.codec_id().to_string(),
            v.codec_private().map(|c| c.to_vec()).unwrap_or_default(),
            v.video().map(|vv| vv.pixel_width().get() as u32).unwrap_or(0),
            v.video().map(|vv| vv.pixel_height().get() as u32).unwrap_or(0),
            v.track_number().get(),
            v.default_duration().map(|d| d.get()).unwrap_or(0),
            audio_info,
        )
    };
    // Estimate the total frame count from the segment duration and the frame
    // duration (matroska has no explicit sample count like MP4).
    let total_video = if default_dur_ns > 0 {
        (file.info().duration().unwrap_or(0.0) * ts_scale_ns as f64 / default_dur_ns as f64) as usize
    } else {
        0
    };

    let (codec, video_annexb, ps_prefix, nal_len) = match codec_id.as_str() {
        "V_MPEG4/ISO/AVC" => match mkv::parse_avcc(&codec_private) {
            Some((p, n)) => (Codec::H264, true, p, n),
            None => return fail("stream: mkv avcC parse".into()),
        },
        "V_MPEGH/ISO/HEVC" => match mkv::parse_hvcc(&codec_private) {
            Some((p, n)) => (Codec::H265, true, p, n),
            None => return fail("stream: mkv hvcC parse".into()),
        },
        "V_VP9" => (Codec::Vp9, false, Vec::new(), 0),
        "V_VP8" => (Codec::Vp8, false, Vec::new(), 0),
        "V_AV1" => (Codec::Av1, false, Vec::new(), 0),
        other => return fail(format!("stream: mkv video codec {other} unsupported")),
    };

    // Audio: AAC-LC only (CodecPrivate is the ASC).
    let mut audio_track_num = u64::MAX; // never matches → audio frames skipped
    let (audio_dec, has_audio, resampler) = match audio_info {
        Some((aid, asc, sr, chans, num)) if aid.starts_with("A_AAC") => {
            let (d, ok, r) = setup_aac_audio(&asc, sr);
            if ok {
                audio_track_num = num;
                log(format!("audio: AAC-LC {chans}ch @ {sr} Hz → stereo 48k"));
                (d, ok, r)
            } else {
                (None, false, None)
            }
        }
        Some((aid, _, sr, chans, num)) if aid == "A_OPUS" => {
            // oxideav Opus derives channels from the packet TOC; the OpusHead
            // (CodecPrivate) pre-skip is ~6.5 ms — ignored for now.
            let (d, ok, r) = setup_opus_audio(chans as usize);
            if ok { audio_track_num = num; log(format!("audio: Opus {chans}ch @ {sr} Hz")); (d, ok, r) }
            else { (None, false, None) }
        }
        Some((aid, _, sr, chans, num)) if aid == "A_AC3" || aid == "A_EAC3" => {
            let eac3 = aid == "A_EAC3";
            let (d, ok, r) = setup_ac3_audio(sr, eac3);
            if ok { audio_track_num = num; log(format!("audio: {aid} {chans}ch @ {sr} Hz")); (d, ok, r) }
            else { (None, false, None) }
        }
        Some((aid, _, _, _, _)) => {
            log(format!("audio: {aid} not decodable — video only"));
            (None, false, None)
        }
        None => (None, false, None),
    };

    let title = item.name.clone();
    let duration_us = (item.duration_s() * 1e6) as i64;
    let src = Box::new(MkvSource {
        file,
        frame: matroska_demuxer::Frame::default(),
        video_track: video_track_num,
        audio_track: audio_track_num,
        ts_scale_ns,
    });

    match install_player(
        url, total_len, Demux::Mkv(src), ps_prefix, nal_len, video_annexb, codec, width, height,
        surface, audio_dec, has_audio, resampler, title.clone(), duration_us, total_video, None, 0,
    ) {
        Ok(impl_name) => {
            STREAM.with(|s| if let Some(p) = s.borrow_mut().as_mut() { p.prefetch = handle; });
            log(format!("streaming \"{title}\" (mkv): {width}x{height}, decoder={impl_name}"));
            on_stream_started(); // B3: resume + report /Sessions/Playing
        }
        Err(e) => { log(e); set_phase(Phase::Browse); }
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
    match jellyfin::fetch_range(&client, &url, start, Some(end)).await {
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

/// Sync decode/present step — runs in on-frame (which supplies the host clock).
/// Submits buffered AUs (bounded), then presents every decoded frame at its
/// deadline, anchoring the clock to the first frame's emergence.
/// Both container sources fetch through their own blocking Range readers, so
/// there is no external RollingBuffer to fill or reclaim — nothing to prefetch.
fn demux_cursor(_d: &Demux) -> u64 {
    u64::MAX
}

// ---- seek (Chunk B) --------------------------------------------------------

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
fn seek_from_clock(delta_us: i64) -> i64 {
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
fn do_seek(p: &mut StreamPlayer, target_us: i64) -> i64 {
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
            land_us
        }
        Demux::Mkv(src) => {
            let ticks = (target_us.max(0) as u64) * 1000 / src.ts_scale_ns.max(1);
            if let Err(e) = src.file.seek(ticks) {
                log(format!("seek: mkv seek to {ticks} ticks failed: {e:?}"));
            }
            target_us
        }
    };
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
fn fill_queues(p: &mut StreamPlayer) {
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
            // matroska-demuxer pulls a frame (blocking on its Range reader as
            // needed); it handles lacing / block groups / seeking internally.
            // matroska-demuxer pulls a frame (blocking on its Range reader as
            // needed); it handles lacing / block groups / seeking internally.
            Demux::Mkv(src) => match src.file.next_frame(&mut src.frame) {
                Ok(true) => {
                    let f = &src.frame;
                    let pts = f.timestamp as i64 * src.ts_scale_ns as i64 / 1000;
                    if f.track == src.video_track {
                        let data = if p.video_annexb {
                            streaming::to_annexb(&f.data, p.nal_len, &p.ps_prefix, f.is_keyframe.unwrap_or(false))
                        } else {
                            f.data.clone()
                        };
                        p.video_q.push_back(VFrame { pts_us: pts, keyframe: f.is_keyframe.unwrap_or(true), data });
                        Prod::Frame(pts, true)
                    } else if f.track == src.audio_track {
                        p.audio_q.push_back(AFrame { pts_us: pts, data: f.data.clone() });
                        Prod::Frame(pts, false)
                    } else {
                        Prod::Skip // subtitle/other track — ignore, keep going
                    }
                }
                Ok(false) => { p.demux_done = true; Prod::Stop }
                Err(e) => { log(format!("stream: mkv demux: {e:?}")); p.demux_done = true; Prod::Stop }
            },
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
fn decode_audio(p: &mut StreamPlayer) {
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

fn pump_stream(nanos: u64) {
    STREAM.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(p) = guard.as_mut() else { return };
        if p.t0_ns == 0 {
            p.t0_ns = nanos;
            // Reveal the transport bar briefly when playback starts (A2).
            ENGINE.with(|e| e.borrow_mut().controls_until_ns = nanos + 3_000_000_000);
        }

        // 0. Pause + control-bar reveal (Part B). The pump owns applying the
        //    Engine's intents. Pause: pause/resume the wasi:audio device (pause
        //    keeps the ring; start resumes) and, on RESUME, shift the clock anchors
        //    forward by the paused duration so the audio-master / free-run
        //    `media_now` stays continuous. While paused we return early: last video
        //    frame stays, and the clock/decode/present/PCM-write are all idle.
        //    Reveal: input sets `controls_bump`; timestamp it to now + 3 s here.
        let want_pause = ENGINE.with(|e| {
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
                // Mute/volume (Part B) is a GAIN on the slice handed to the device,
                // applied here (not in decode) so a level change takes effect on the
                // still-buffered pending PCM; the ~0.5 s already in the ring keeps
                // its gain. gain==1.0 writes the buffer directly (no copy).
                let gain = ENGINE.with(|e| { let e = e.borrow(); if e.muted { 0.0 } else { e.volume } });
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

        // 3. Video: submit queued AUs, bounded by the decode-ahead cushion.
        while p.submitted < p.presented + DECODE_AHEAD {
            let Some(vf) = p.video_q.front() else { break };
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
fn drive_fetch() {
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

// ---- render ----------------------------------------------------------------

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

fn draw_rect(cv: &Canvas, x: f32, y: f32, w: f32, h: f32, argb: u32) {
    cv.draw_rect(wtypes::Rect { x, y, width: w, height: h }, &fill_paint(argb));
}

fn draw_text(cv: &Canvas, text: &str, x: f32, y: f32, size: f32, weight: u32, color: u32, wrap_w: f32) {
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

fn wctx<R>(f: impl FnOnce(&wembed::CanvasContext) -> R) -> R {
    WCTX.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(wembed::get_context());
        }
        f(c.borrow().as_ref().unwrap())
    })
}

/// The whole A1 UI: a pairing screen, a browse list, and a status footer. Drawn
/// guest-side on wasi:canvas — no video surface yet (that is A2).
fn render() {
    let cv = wctx(|x| x.get_current_buffer());
    cv.clear(0xFF10_1216); // near-black background

    let (sw, sh) = ENGINE.with(|e| e.borrow().surface);
    let (sw, sh) = (sw as f32, sh as f32);
    let pad = 20.0;

    // Header.
    draw_text(&cv, "Jellyfin", pad, pad, 34.0, 700, 0xFF7B_8CFF, sw - 2.0 * pad);

    let phase = ENGINE.with(|e| e.borrow().phase.clone());
    match phase {
        Phase::Boot | Phase::Pairing => {
            let code = ENGINE.with(|e| e.borrow().pairing_code.clone());
            let msg = if let Some(code) = code {
                draw_text(&cv, "Enter this code in Jellyfin →", pad, sh * 0.30, 22.0, 500, 0xFFFF_FFFF, sw - 2.0 * pad);
                draw_text(&cv, &code, pad, sh * 0.30 + 40.0, 72.0, 800, 0xFF7B_FFB0, sw - 2.0 * pad);
                "(user icon → Quick Connect)".to_string()
            } else {
                "Connecting…".to_string()
            };
            draw_text(&cv, &msg, pad, sh * 0.30 + 130.0, 18.0, 400, 0xFFB0_B4BC, sw - 2.0 * pad);
        }
        Phase::LoadingLibrary => {
            draw_text(&cv, "Loading library…", pad, sh * 0.4, 24.0, 500, 0xFFFF_FFFF, sw - 2.0 * pad);
        }
        Phase::Browse | Phase::Resolving | Phase::Ready | Phase::Playing | Phase::Failed => {
            render_list(&cv, sw, sh, pad);
        }
    }

    // Footer: the last log line.
    let last = ENGINE.with(|e| e.borrow().log.last().cloned()).unwrap_or_default();
    draw_rect(&cv, 0.0, sh - 40.0, sw, 40.0, 0xFF1C_2028);
    draw_text(&cv, &last, pad, sh - 32.0, 15.0, 400, 0xFF9AA0_A8u32 & 0xFFFFFFFF, sw - 2.0 * pad);

    drop(cv);
    wctx(|x| x.present());
}

fn render_list(cv: &Canvas, sw: f32, sh: f32, pad: f32) {
    let top = pad + 50.0;
    let row_h = 46.0;
    let visible = (((sh - top - 50.0) / row_h) as usize).max(1);
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        let sel = e.selected.min(e.items.len().saturating_sub(1));
        // Scroll window keeps the selection visible.
        let first = sel.saturating_sub(visible.saturating_sub(1).min(sel));
        // Publish the layout so on-pointer can map a click y → row index.
        e.list_view = (top, row_h, first, visible);
        let e = &*e;
        for (row, it) in e.items.iter().enumerate().skip(first).take(visible) {
            let y = top + (row - first) as f32 * row_h;
            if row == sel {
                draw_rect(cv, pad - 6.0, y - 4.0, sw - 2.0 * pad + 12.0, row_h - 6.0, 0xFF2A_3550);
            }
            let playable = is_playable(it);
            let name_color = if playable { 0xFFFF_FFFF } else { 0xFF70_747C };
            draw_text(cv, &it.name, pad, y, 19.0, 600, name_color, sw - 2.0 * pad - 160.0);
            let cont = if it.container.is_empty() { "?" } else { it.container.as_str() };
            let meta = format!("{} · {} {}/{}", fmt_dur(it.duration_s()), cont, it.video_codec, it.audio_codec);
            draw_text(cv, &meta, sw - pad - 190.0, y + 2.0, 13.0, 400, 0xFF8A_9098, 190.0);
        }
    });
}

fn fmt_dur(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The Playing overlay: transparent clear reveals the host video surface behind
/// the UI (behind-ui), with a title bar + progress bar drawn on top — exactly the
/// wandr.video.player compositing model.
/// Transport-bar geometry (A2). Shared by `render_playing` (draw) and `on_pointer`
/// (hit-test) so buttons line up with clicks. Rects are (x, y, w, h).
struct CtlBar {
    panel: (f32, f32, f32, f32),
    scrub: (f32, f32, f32, f32),
    playpause: (f32, f32, f32, f32),
    stop: (f32, f32, f32, f32),
    mute: (f32, f32, f32, f32),
    vol: (f32, f32, f32, f32),
}

fn control_bar(sw: f32, sh: f32) -> CtlBar {
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

fn hit(x: f32, y: f32, r: (f32, f32, f32, f32)) -> bool {
    x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
}

/// Right-pointing play triangle from vertical rects (font-independent).
fn draw_play(cv: &Canvas, x: f32, y: f32, s: f32, color: u32) {
    let n = 12usize;
    let cw = s / n as f32;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let h = s * (1.0 - t);
        draw_rect(cv, x + i as f32 * cw, y + (s - h) / 2.0, cw + 0.6, h, color);
    }
}

/// Two-bar pause glyph.
fn draw_pause(cv: &Canvas, x: f32, y: f32, s: f32, color: u32) {
    let bw = s * 0.28;
    draw_rect(cv, x, y, bw, s, color);
    draw_rect(cv, x + s - bw, y, bw, s, color);
}

fn render_playing(nanos: u64) {
    let cv = wctx(|x| x.get_current_buffer());
    cv.clear(0x0000_0000); // transparent — the decoded video shows through

    let (sw, sh) = ENGINE.with(|e| e.borrow().surface);
    let (sw, sh) = (sw as f32, sh as f32);
    let (paused, muted, volume, controls_until_ns, scrubbing, scrub_frac, subs_on) = ENGINE.with(|e| {
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

    // ---- transport chrome (A2): title bar + bottom control panel, auto-hiding
    //      after ~3 s; any key/pointer (or being paused) reveals it.
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

        // Scrub: track + buffered-ahead + played + knob (display only in A2; drag
        // → seek comes in Chunk B).
        let buf_pct = if dur_us > 0 {
            ((clock_us as f64 + aud_buf_s as f64 * 1e6) / dur_us as f64).clamp(0.0, 1.0) as f32
        } else { 0.0 };
        // While dragging (B2), the played fill + knob preview the drag position.
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

    // Subtitles (Tier 3): the active cue at the current clock, bottom-center,
    // lifted above the control bar when it's visible. Centering is approximate
    // (no text-measure API): estimate width at ~0.52·fontsize per char.
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

    // Paused overlay (Part B, A1): a centered pill with a font-independent
    // two-bar pause glyph. A2 replaces this with the full clickable transport bar.
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

// ---- exports ---------------------------------------------------------------

struct Component;

impl FrameGuest for Component {
    fn on_frame(nanos: u64) {
        // NOTE: no block_on here — on-frame is a SYNCHRONOUS CM export and cannot
        // block. The MKV open + demux (which block_on the Range reader) run in the
        // async bg-tick; on-frame only submits/decodes/presents from the queues.
        let playing = ENGINE.with(|e| e.borrow().phase == Phase::Playing);
        if playing {
            pump_stream(nanos);
            render_playing(nanos);
        } else {
            render();
        }
    }
    fn on_resize(w: u32, h: u32) {
        let (w, h) = (w.max(1), h.max(1));
        ENGINE.with(|e| e.borrow_mut().surface = (w, h));
        // Live-reconcile the decoder's surface rect (mirrors wandr.video.player).
        STREAM.with(|s| {
            if let Some(p) = s.borrow().as_ref() {
                p.dec.set_rect(video_rect(w, h));
            }
        });
    }
}

/// Move the selection by `delta` rows (clamped). Shared by keys and scroll.
fn nav(e: &mut Engine, delta: i64) {
    let n = e.items.len();
    if n == 0 {
        return;
    }
    e.selected = (e.selected as i64 + delta).clamp(0, n as i64 - 1) as usize;
}

/// Activate the selected item — resolve its DirectPlay URL (A2: start playing).
/// Shared by Enter and click-on-selected.
fn activate(e: &mut Engine) {
    if e.items.is_empty() || e.resolving {
        return;
    }
    // Ignore unplayable (greyed) items — MKV/WebM or 5.1/Opus/Dolby audio the
    // guest can't yet demux/decode. (Can't log here: ENGINE is already borrowed.)
    if !is_playable(&e.items[e.selected]) {
        return;
    }
    e.resolving = true;
    e.pending_resolve = Some(e.selected);
}

impl KeyGuest for Component {
    fn on_key(ev: KeyEvent) {
        if !ev.down {
            return;
        }
        ENGINE.with(|e| {
            let mut e = e.borrow_mut();
            // While Playing: transport controls (Part B). Escape/Backspace/q stop
            // (teardown happens in bg-tick); Space/k toggle pause; ↑/↓ adjust
            // volume (and un-mute on up); m toggles mute. Any other key is consumed.
            if e.phase == Phase::Playing {
                e.controls_bump = true; // any key reveals the transport bar
                if matches!(ev.code.as_str(), "Escape" | "Backspace") || ev.text.eq_ignore_ascii_case("q") {
                    e.stop_requested = true;
                } else if ev.code == "Space" || ev.code == "KeyK" || ev.text == " " {
                    e.paused = !e.paused;
                } else if ev.code == "ArrowUp" {
                    e.muted = false;
                    e.volume = (e.volume + 0.1).min(1.0);
                } else if ev.code == "ArrowDown" {
                    e.volume = (e.volume - 0.1).max(0.0);
                } else if ev.code == "KeyM" || ev.text.eq_ignore_ascii_case("m") {
                    e.muted = !e.muted;
                } else if ev.code == "ArrowRight" || ev.text.eq_ignore_ascii_case("l") {
                    e.seek_request = Some(seek_from_clock(10_000_000)); // +10 s
                } else if ev.code == "ArrowLeft" || ev.text.eq_ignore_ascii_case("j") {
                    e.seek_request = Some(seek_from_clock(-10_000_000)); // −10 s
                } else if ev.code == "Home" {
                    e.seek_request = Some(0); // restart
                } else if ev.code == "KeyS" || ev.text.eq_ignore_ascii_case("s") {
                    // Cycle subtitles: off → track0 → track1 → … → off.
                    let n = e.resolved.as_ref().map(|(_, pb, _)| pb.subtitles.len()).unwrap_or(0);
                    e.sub_sel = match e.sub_sel {
                        None if n > 0 => Some(0),
                        Some(i) if i + 1 < n => Some(i + 1),
                        _ => None,
                    };
                    e.sub_dirty = true;
                }
                return;
            }
            // Resolving/Ready: Escape/Backspace/q aborts back to the list.
            if matches!(e.phase, Phase::Resolving | Phase::Ready) {
                if matches!(ev.code.as_str(), "Escape" | "Backspace")
                    || ev.text.eq_ignore_ascii_case("q")
                {
                    e.stop_requested = true;
                    return;
                }
            }
            if e.phase != Phase::Browse && e.phase != Phase::Ready {
                return;
            }
            // Arrow/Enter carry no `text` — match the W3C `code` first, then fall
            // back to text for the j/k vim keys.
            match ev.code.as_str() {
                "ArrowDown" => nav(&mut e, 1),
                "ArrowUp" => nav(&mut e, -1),
                "PageDown" => nav(&mut e, 10),
                "PageUp" => nav(&mut e, -10),
                "Home" => e.selected = 0,
                "End" => nav(&mut e, i64::MAX),
                "Enter" | "NumpadEnter" | "Space" => activate(&mut e),
                _ => match ev.text.as_str() {
                    "j" => nav(&mut e, 1),
                    "k" => nav(&mut e, -1),
                    _ => {}
                },
            }
        });
    }
}

impl PointerGuest for Component {
    fn on_pointer(ev: PointerEvent) {
        ENGINE.with(|e| {
            let mut e = e.borrow_mut();
            // Playing: transport bar. Any pointer reveals it; primary-press hit-tests
            // the buttons + volume slider; the scrub track is a DRAG (B2) — down
            // starts it, move previews, up commits the seek. A plain click = down+up
            // at one spot, so it still seeks there.
            if e.phase == Phase::Playing {
                e.controls_bump = true;
                let c = control_bar(e.surface.0 as f32, e.surface.1 as f32);
                let scrub_hit = (c.scrub.0, c.scrub.1 - 8.0, c.scrub.2, c.scrub.3 + 16.0);
                let scrub_frac = ((ev.x - c.scrub.0) / c.scrub.2).clamp(0.0, 1.0);
                match ev.kind {
                    PtrKind::Down if matches!(ev.button, Button::Primary) => {
                        if hit(ev.x, ev.y, c.playpause) {
                            e.paused = !e.paused;
                        } else if hit(ev.x, ev.y, c.stop) {
                            e.stop_requested = true;
                        } else if hit(ev.x, ev.y, c.mute) {
                            e.muted = !e.muted;
                        } else if hit(ev.x, ev.y, c.vol) {
                            e.muted = false;
                            e.volume = ((ev.x - c.vol.0) / c.vol.2).clamp(0.0, 1.0);
                        } else if hit(ev.x, ev.y, scrub_hit) {
                            e.scrubbing = true;
                            e.scrub_frac = scrub_frac;
                        }
                    }
                    PtrKind::Move if e.scrubbing => e.scrub_frac = scrub_frac,
                    PtrKind::Up if e.scrubbing => {
                        e.scrubbing = false;
                        let dur = STREAM.with(|s| s.borrow().as_ref().map(|p| p.duration_us).unwrap_or(0));
                        e.seek_request = Some((e.scrub_frac as f64 * dur as f64) as i64);
                    }
                    PtrKind::Cancel | PtrKind::Leave if e.scrubbing => e.scrubbing = false, // abort
                    _ => {}
                }
                return;
            }
            if e.phase != Phase::Browse && e.phase != Phase::Ready {
                return;
            }
            match ev.kind {
                PtrKind::Scroll => {
                    // W3C wheel: positive scroll-dy = content moves down → advance.
                    // Handle both line-unit (small) and pixel-unit (large) deltas.
                    let dy = ev.scroll_dy;
                    if dy.abs() >= 3.0 {
                        e.scroll_accum += dy;
                        while e.scroll_accum >= 40.0 {
                            e.scroll_accum -= 40.0;
                            nav(&mut e, 1);
                        }
                        while e.scroll_accum <= -40.0 {
                            e.scroll_accum += 40.0;
                            nav(&mut e, -1);
                        }
                    } else if dy > 0.0 {
                        nav(&mut e, 1);
                    } else if dy < 0.0 {
                        nav(&mut e, -1);
                    }
                }
                PtrKind::Down if matches!(ev.button, Button::Primary) => {
                    let (top, row_h, first, visible) = e.list_view;
                    if ev.y >= top && row_h > 0.0 {
                        let row = first + ((ev.y - top) / row_h) as usize;
                        if row < first + visible && row < e.items.len() {
                            // First click selects; clicking the already-selected
                            // row plays it (tap-to-select, tap-again-to-play).
                            if row == e.selected {
                                activate(&mut e);
                            } else {
                                e.selected = row;
                            }
                        }
                    }
                }
                _ => {}
            }
        });
    }
}

impl PacingGuest for Component {
    fn next_frame_delay() -> u32 {
        // A1 UI is nearly static; a lazy cadence is fine. A2 will drop this to a
        // frame-rate cadence while Playing (video pump), like wandr.video.player.
        ENGINE.with(|e| match e.borrow().phase {
            Phase::Playing => 16, // frame-rate cadence: pump pulls + presents here
            Phase::Pairing | Phase::LoadingLibrary | Phase::Resolving => 100,
            _ => 200,
        })
    }
}

impl BgGuest for Component {
    async fn bg_tick() -> u32 {
        // Spawn the driver exactly once.
        let spawn_driver = ENGINE.with(|e| {
            let mut e = e.borrow_mut();
            if !e.driver_spawned {
                e.driver_spawned = true;
                true
            } else {
                false
            }
        });
        if spawn_driver {
            reqwest::task::spawn(driver());
        }

        // Pick up a pending resolve request from on-key.
        if let Some(idx) = ENGINE.with(|e| e.borrow_mut().pending_resolve.take()) {
            reqwest::task::spawn(resolve(idx));
        }

        // Stop requested (Escape/Backspace while Playing): tear the stream down
        // — dropping StreamPlayer releases the decoder surface + audio device.
        let stop = ENGINE.with(|e| {
            let mut e = e.borrow_mut();
            if e.stop_requested {
                e.stop_requested = false;
                e.phase = Phase::Browse;
                e.resolving = false;
                true
            } else {
                false
            }
        });
        if stop {
            // B3: report the final position to Jellyfin before tearing down (saves
            // the resume point + watched status). `resolved` still holds this item.
            let final_us = STREAM.with(|s| s.borrow().as_ref().map(|p| p.clock_us).unwrap_or(0));
            spawn_report(Report::Stopped, final_us, false);
            STREAM.with(|s| *s.borrow_mut() = None);
            // Drop the stream's audio still buffered in the shared device, but
            // keep the device OPEN (reopening churns COM on Windows/WASAPI).
            with_audio(|pb| pb.flush());
            log("stopped — back to list");
        }

        // A resolved MKV title opens here (async context) — matroska-demuxer's
        // block_on reader is only legal in an async-lifted export, not on-frame.
        if let Some((url, total, item, surface, is_mkv)) = ENGINE.with(|e| e.borrow_mut().pending_open.take()) {
            if is_mkv {
                open_mkv_sync(url, total, item, surface);
            } else {
                open_mp4_sync(url, total, item, surface);
            }
        }

        // While Playing, DEMUX into the frame queues here (bg-tick) — MKV pulls
        // through a blocking reader, which is only allowed in this async export.
        // MP4 keeps its async prefetch (drive_fetch). Decode/present run in
        // on-frame, which has the host clock.
        let phase = ENGINE.with(|e| e.borrow().phase.clone());
        if phase == Phase::Playing {
            // Proactive async prefetch keeps the reader's cache ahead so the
            // demuxer's block_on fallback rarely fires (no hiccup).
            let handle = STREAM.with(|s| s.borrow().as_ref().and_then(|p| p.prefetch.clone()));
            if let Some(h) = handle {
                httprange::drive_prefetch(&h);
            }
            // Tier 3: subtitle track change → fetch + parse the VTT (or clear).
            if ENGINE.with(|e| std::mem::take(&mut e.borrow_mut().sub_dirty)) {
                let sel = ENGINE.with(|e| {
                    let e = e.borrow();
                    e.sub_sel
                        .and_then(|i| e.resolved.as_ref().and_then(|(item, pb, _)| {
                            pb.subtitles.get(i).map(|st|
                                (item.id.clone(), pb.media_source_id.clone(), st.index, st.label.clone()))
                        }))
                        .zip(e.session.clone())
                });
                SUBTITLES.with(|s| s.borrow_mut().clear());
                match sel {
                    Some(((id, msid, idx, label), session)) => {
                        log(format!("subtitles: {label} …"));
                        if let Some(client) = build_client() {
                            reqwest::task::spawn(fetch_subtitles(
                                client, jellyfin::subtitle_vtt_url(&session, &id, &msid, idx)));
                        }
                    }
                    None => log("subtitles: off"),
                }
            }
            let seek = ENGINE.with(|e| e.borrow_mut().seek_request.take());
            STREAM.with(|s| {
                if let Some(p) = s.borrow_mut().as_mut() {
                    if let Some(target) = seek {
                        do_seek(p, target); // reposition + reset before refilling
                    }
                    fill_queues(p);   // demux → raw video/audio queues
                    decode_audio(p);  // raw audio → decoded PCM (off the on-frame path)
                }
            });
            // B3: report progress every ~10 s of MEDIA time (bg-tick has no wall
            // clock, so throttle on the playback clock, not real time).
            let clk = STREAM.with(|s| s.borrow().as_ref().map(|p| p.clock_us).unwrap_or(0));
            let due = ENGINE.with(|e| {
                let mut e = e.borrow_mut();
                if clk - e.last_report_us >= 10_000_000 { e.last_report_us = clk; true } else { false }
            });
            if due {
                let paused = ENGINE.with(|e| e.borrow().paused);
                spawn_report(Report::Progress, clk, paused);
            }
            return 16;
        }
        match phase {
            Phase::Pairing => 500, // polling is on its own timer; idle here
            Phase::Boot | Phase::LoadingLibrary | Phase::Resolving | Phase::Ready => 100,
            _ => 250,
        }
    }
}

export!(Component);
