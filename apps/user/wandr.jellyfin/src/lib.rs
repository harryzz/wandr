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

// EXPORTS-ONLY world (the video/audio/canvas imports moved to wandr-media-engine's
// own bindgen). No `generate_all` — the deleted deps are gone; this generates only
// the export interfaces (frame/key/pointer handlers, frame-pacing, background).
wit_bindgen::generate!({
    world: "wandr:jellyfin/jellyfin-app",
    path: "wit",
});

mod jellyfin;
use jellyfin::{Item, Playback, Session};

// The extracted playback engine: owns the demux/audio/clock/present pipeline and
// the video/audio/canvas IMPORTS bindgen. The app drives it and draws its browse
// UI against the engine's re-exported canvas bindings.
use wandr_media_engine as engine;
use engine::canvas::draw::Canvas;

use crate::exports::wandr::background::background::Guest as BgGuest;
use crate::exports::wandr::ui_shell::frame_pacing::Guest as PacingGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::key_handler::{Guest as KeyGuest, KeyEvent};
use crate::exports::wasi::input_handlers::pointer_handler::{
    Button, Guest as PointerGuest, Kind as PtrKind, PointerEvent,
};

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

/// Browse tabs.
#[derive(Clone, Copy, PartialEq)]
enum Tab { Movies, Shows }

struct Engine {
    phase: Phase,
    session: Option<Session>,
    /// The Quick Connect code to type into the Jellyfin web UI (Pairing phase).
    pairing_code: Option<String>,
    /// The CURRENT playable list: movies (Movies tab) OR the open series' episodes.
    items: Vec<Item>,
    selected: usize,
    /// Browse tabs (little-polish): Movies = flat list; Shows = Series → Episodes.
    tab: Tab,
    movies: Vec<Item>,
    series: Vec<Item>,            // Shows level 0: Series rows
    seasons: Vec<Item>,           // Shows level 1: Seasons of the open series
    series_open: Option<usize>,   // index into `series` (None = series list)
    season_open: Option<usize>,   // index into `seasons` (None = seasons list; episodes in `items`)
    /// Pending async drill fetches (bg-tick): (series idx, series id) → Seasons;
    /// (season idx, series id, season id) → Episodes.
    pending_seasons: Option<(usize, String)>,
    pending_episodes: Option<(usize, String, String)>,
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
    /// Geometry of the browse list as last rendered — so on-pointer can hit-test
    /// a click to a row. (top-y, row-height, first-visible-index, visible-count).
    list_view: (f32, f32, usize, usize),
    /// Accumulated scroll delta (surface units) until it crosses one row.
    scroll_accum: f32,
    /// A resolved title awaiting a SYNCHRONOUS open in bg-tick — both container
    /// demuxers (mp4 crate / matroska-demuxer) pull through a block_on Range
    /// reader, which is only legal in the async bg-tick, not on-frame.
    /// (url, total_len, item, surface, is_mkv).
    pending_open: Option<(String, u64, Item, (u32, u32), bool)>,
    /// B3: media-time (µs) of the last progress report to Jellyfin — throttles
    /// reporting to ~10 s intervals; reset to 0 when a new stream starts.
    last_report_us: i64,
    // All TRANSPORT state — paused/muted/volume, the control-bar reveal timer,
    // seek/scrub, subtitle + audio-track selection, the surface size, and the stop
    // flag — now lives in `wandr_media_engine::CONTROLS`. Input handlers write it
    // there; the engine's pump + overlay read it. This app struct keeps only the
    // browse/session state the engine never sees.
}

impl Engine {
    const fn new() -> Self {
        Engine {
            phase: Phase::Boot,
            session: None,
            pairing_code: None,
            items: Vec::new(),
            selected: 0,
            tab: Tab::Movies,
            movies: Vec::new(),
            series: Vec::new(),
            seasons: Vec::new(),
            series_open: None,
            season_open: None,
            pending_seasons: None,
            pending_episodes: None,
            pending_resolve: None,
            resolved: None,
            driver_spawned: false,
            resolving: false,
            log: Vec::new(),
            list_view: (0.0, 46.0, 0, 0),
            scroll_accum: 0.0,
            pending_open: None,
            last_report_us: 0,
        }
    }
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::new());
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
    if it.is_series { return true; } // Series rows drill into episodes, not greyed
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
            size, run_time_ticks: 0, image_tag: None, resume_ticks: 0, is_series: false,
        };
        let surface = engine::CONTROLS.with(|c| c.borrow().surface);
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
    let movies = match jellyfin::browse_movies(&client, &session, 500).await {
        Ok(mut m) => { m.sort_by_key(|i| !is_playable(i)); m } // playable first
        Err(e) => { log(format!("browse movies failed: {e}")); set_phase(Phase::Failed); return; }
    };
    let series = jellyfin::browse_series(&client, &session, 500).await.unwrap_or_else(|e| {
        log(format!("browse series failed: {e}")); Vec::new()
    });
    log(format!("library: {} movies · {} series", movies.len(), series.len()));
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        e.items = movies.clone(); // default tab = Movies
        e.movies = movies;
        e.series = series;
        e.tab = Tab::Movies;
        e.series_open = None;
        e.selected = 0;
    });
    set_phase(Phase::Browse);
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
                let surface = engine::CONTROLS.with(|c| c.borrow().surface);
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

/// Shows drill-in level 1: fetch a series' Seasons.
async fn open_seasons(idx: usize, series_id: String) {
    let (client, session) = ENGINE.with(|e| { let e = e.borrow(); (build_client(), e.session.clone()) });
    let (Some(client), Some(session)) = (client, session) else { return };
    match jellyfin::browse_seasons(&client, &session, &series_id).await {
        Ok(seasons) => {
            log(format!("series: {} seasons", seasons.len()));
            ENGINE.with(|e| {
                let mut e = e.borrow_mut();
                e.seasons = seasons;
                e.series_open = Some(idx);
                e.season_open = None;
                e.selected = 0;
            });
        }
        Err(e) => log(format!("seasons: {e}")),
    }
}

/// Shows drill-in level 2: fetch a season's Episodes (kept in order).
async fn open_episodes(idx: usize, series_id: String, season_id: String) {
    let (client, session) = ENGINE.with(|e| { let e = e.borrow(); (build_client(), e.session.clone()) });
    let (Some(client), Some(session)) = (client, session) else { return };
    match jellyfin::browse_episodes(&client, &session, &series_id, &season_id).await {
        Ok(eps) => {
            log(format!("season: {} episodes", eps.len()));
            ENGINE.with(|e| {
                let mut e = e.borrow_mut();
                e.items = eps;
                e.season_open = Some(idx);
                e.selected = 0;
            });
        }
        Err(e) => log(format!("episodes: {e}")),
    }
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

/// Fetch + parse a subtitle track into SUBTITLES (spawned from bg-tick).
async fn fetch_subtitles(client: reqwest::Client, vtt_url: String) {
    match jellyfin::fetch_vtt(&client, vtt_url).await {
        Ok(text) => {
            let cues = engine::parse_vtt(&text);
            let n = cues.len();
            engine::SUBTITLES.with(|s| *s.borrow_mut() = cues);
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
    engine::CONTROLS.with(|c| {
        let mut c = c.borrow_mut();
        c.audio_pref = 0; // fresh stream → first audio track
        c.audio_switch = false;
    });
    if let Some(item) = ENGINE.with(|e| e.borrow().resolved.clone().map(|(i, _, _)| i)) {
        let dur_us = (item.duration_s() * 1e6) as i64;
        let resume_us = item.resume_ticks / 10; // 100 ns ticks → µs
        if resume_us > 5_000_000 && (dur_us == 0 || resume_us < dur_us * 9 / 10) {
            engine::CONTROLS.with(|c| c.borrow_mut().seek_request = Some(resume_us));
            log(format!("resume: seeking to {:.0}s", resume_us as f64 / 1e6));
        }
    }
    spawn_report(Report::Playing, 0, false);
}

/// The whole A1 UI: a pairing screen, a browse list, and a status footer. Drawn
/// guest-side on wasi:canvas — no video surface yet (that is A2).
fn render() {
    let cv = engine::wctx(|x| x.get_current_buffer());
    cv.clear(0xFF10_1216); // near-black background

    let (sw, sh) = engine::CONTROLS.with(|c| c.borrow().surface);
    let (sw, sh) = (sw as f32, sh as f32);
    let pad = 20.0;

    // Header.
    engine::draw_text(&cv, "Jellyfin", pad, pad, 34.0, 700, 0xFF7B_8CFF, sw - 2.0 * pad);

    let phase = ENGINE.with(|e| e.borrow().phase.clone());
    match phase {
        Phase::Boot | Phase::Pairing => {
            let code = ENGINE.with(|e| e.borrow().pairing_code.clone());
            let msg = if let Some(code) = code {
                engine::draw_text(&cv, "Enter this code in Jellyfin →", pad, sh * 0.30, 22.0, 500, 0xFFFF_FFFF, sw - 2.0 * pad);
                engine::draw_text(&cv, &code, pad, sh * 0.30 + 40.0, 72.0, 800, 0xFF7B_FFB0, sw - 2.0 * pad);
                "(user icon → Quick Connect)".to_string()
            } else {
                "Connecting…".to_string()
            };
            engine::draw_text(&cv, &msg, pad, sh * 0.30 + 130.0, 18.0, 400, 0xFFB0_B4BC, sw - 2.0 * pad);
        }
        Phase::LoadingLibrary => {
            engine::draw_text(&cv, "Loading library…", pad, sh * 0.4, 24.0, 500, 0xFFFF_FFFF, sw - 2.0 * pad);
        }
        Phase::Browse | Phase::Resolving | Phase::Ready | Phase::Playing | Phase::Failed => {
            render_list(&cv, sw, sh, pad);
        }
    }

    // Footer: the last log line.
    let last = ENGINE.with(|e| e.borrow().log.last().cloned()).unwrap_or_default();
    engine::draw_rect(&cv, 0.0, sh - 40.0, sw, 40.0, 0xFF1C_2028);
    engine::draw_text(&cv, &last, pad, sh - 32.0, 15.0, 400, 0xFF9AA0_A8u32 & 0xFFFFFFFF, sw - 2.0 * pad);

    drop(cv);
    engine::wctx(|x| x.present());
}

/// Tab-bar hit-test (shared by render + on_pointer): the Tab under (x,y), if any.
/// Tabs sit BELOW the "Jellyfin" header (which occupies ~pad..pad+40).
fn tab_hit(x: f32, y: f32, pad: f32) -> Option<Tab> {
    if y < pad + 44.0 || y > pad + 76.0 { return None; }
    if x >= pad && x < pad + 92.0 { Some(Tab::Movies) }
    else if x >= pad + 100.0 && x < pad + 192.0 { Some(Tab::Shows) }
    else { None }
}

fn render_list(cv: &Canvas, sw: f32, sh: f32, pad: f32) {
    // Tabs (below the header) + a breadcrumb when drilled into a series.
    ENGINE.with(|e| {
        let e = e.borrow();
        let ty = pad + 46.0;
        for (i, (tab, label)) in [(Tab::Movies, "Movies"), (Tab::Shows, "Shows")].iter().enumerate() {
            let x = pad + i as f32 * 100.0;
            let active = e.tab == *tab;
            engine::draw_rect(cv, x, ty, 92.0, 28.0, if active { 0xFF3A_4560 } else { 0xFF20_242C });
            engine::draw_text(cv, label, x + 14.0, ty + 4.0, 18.0, if active { 700 } else { 500 },
                if active { 0xFFFF_FFFF } else { 0xFF9A_9EA6 }, 80.0);
        }
        if let Some(crumb) = shows_crumb(&e) {
            engine::draw_text(cv, &crumb, pad + 210.0, ty + 6.0, 15.0, 500, 0xFF8A_9098, sw - pad - 210.0);
        }
    });

    let top = pad + 88.0;
    let row_h = 46.0;
    let rows = (((sh - top - 50.0) / row_h) as usize).max(1);
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        let n = visible(&e).len();
        let sel = e.selected.min(n.saturating_sub(1));
        // Scroll window keeps the selection visible.
        let first = sel.saturating_sub(rows.saturating_sub(1).min(sel));
        // Publish the layout so on-pointer can map a click y → row index.
        e.list_view = (top, row_h, first, rows);
        let e = &*e;
        let folder_word = if e.series_open.is_none() { "seasons" } else { "episodes" };
        for (row, it) in visible(e).iter().enumerate().skip(first).take(rows) {
            let y = top + (row - first) as f32 * row_h;
            if row == sel {
                engine::draw_rect(cv, pad - 6.0, y - 4.0, sw - 2.0 * pad + 12.0, row_h - 6.0, 0xFF2A_3550);
            }
            let name_color = if is_playable(it) { 0xFFFF_FFFF } else { 0xFF70_747C };
            engine::draw_text(cv, &it.name, pad, y, 19.0, 600, name_color, sw - 2.0 * pad - 160.0);
            let meta = if it.is_series {
                format!("{} {folder_word}  >", it.run_time_ticks)
            } else {
                let cont = if it.container.is_empty() { "?" } else { it.container.as_str() };
                format!("{} · {} {}/{}", engine::fmt_dur(it.duration_s()), cont, it.video_codec, it.audio_codec)
            };
            engine::draw_text(cv, &meta, sw - pad - 190.0, y + 2.0, 13.0, 400, 0xFF8A_9098, 190.0);
        }
    });
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
            engine::pump_stream(nanos);
            engine::render_playing(nanos);
        } else {
            render();
        }
    }
    fn on_resize(w: u32, h: u32) {
        // Record the surface size on the engine's CONTROLS + live-reconcile the
        // decoder rect (mirrors wandr.video.player). The engine owns both now.
        engine::set_surface(w, h);
    }
}

/// The currently VISIBLE rows: Series (Shows/L0), Seasons (Shows/L1), else the
/// playable `items` (Movies, or a season's Episodes).
fn visible(e: &Engine) -> &[Item] {
    match (e.tab, e.series_open, e.season_open) {
        (Tab::Shows, None, _) => &e.series,
        (Tab::Shows, Some(_), None) => &e.seasons,
        _ => &e.items,
    }
}

/// Shows-tab breadcrumb ("series > season"), or None.
fn shows_crumb(e: &Engine) -> Option<String> {
    if e.tab != Tab::Shows { return None; }
    let si = e.series_open?;
    let sname = e.series.get(si).map(|s| s.name.as_str()).unwrap_or("");
    Some(match e.season_open {
        Some(sei) => format!("< Esc   ·   {sname}  >  {}", e.seasons.get(sei).map(|s| s.name.as_str()).unwrap_or("")),
        None => format!("< Esc   ·   {sname}"),
    })
}

/// Move the selection by `delta` rows (clamped). Shared by keys and scroll.
fn nav(e: &mut Engine, delta: i64) {
    let n = visible(e).len();
    if n == 0 {
        return;
    }
    e.selected = (e.selected as i64 + delta).clamp(0, n as i64 - 1) as usize;
}

/// Switch browse tab, resetting to its top.
fn set_tab(e: &mut Engine, tab: Tab) {
    e.tab = tab;
    e.series_open = None;
    e.season_open = None;
    e.selected = 0;
    if tab == Tab::Movies {
        e.items = e.movies.clone();
    }
}

/// Go up one Shows level (Episodes → Seasons → Series). True if it moved.
fn back_up(e: &mut Engine) -> bool {
    if e.tab != Tab::Shows {
        return false;
    }
    if e.season_open.is_some() {
        e.season_open = None;
        e.selected = 0;
        true
    } else if e.series_open.is_some() {
        e.series_open = None;
        e.selected = 0;
        true
    } else {
        false
    }
}

/// Activate the selected row — Series → its Seasons, Season → its Episodes, a
/// playable item → resolve + play. Shared by Enter and click-on-selected.
fn activate(e: &mut Engine) {
    if e.resolving {
        return;
    }
    let sel = e.selected;
    match (e.tab, e.series_open, e.season_open) {
        (Tab::Shows, None, _) => {
            if let Some(sr) = e.series.get(sel) {
                e.pending_seasons = Some((sel, sr.id.clone()));
            }
        }
        (Tab::Shows, Some(si), None) => {
            let series_id = e.series.get(si).map(|s| s.id.clone()).unwrap_or_default();
            if let Some(se) = e.seasons.get(sel) {
                e.pending_episodes = Some((sel, series_id, se.id.clone()));
            }
        }
        _ => {
            if sel < e.items.len() && is_playable(&e.items[sel]) {
                e.resolving = true;
                e.pending_resolve = Some(sel);
            }
        }
    }
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
                // Transport intents live on engine::CONTROLS now (the pump reads them).
                // Subtitle count comes from the app's resolved item; audio-track count
                // from the active stream.
                let n_subs = e.resolved.as_ref().map(|(_, pb, _)| pb.subtitles.len()).unwrap_or(0);
                let n_audio = engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.audio_track_count()).unwrap_or(0));
                engine::CONTROLS.with(|c| {
                    let mut c = c.borrow_mut();
                    c.controls_bump = true; // any key reveals the transport bar
                    if matches!(ev.code.as_str(), "Escape" | "Backspace") || ev.text.eq_ignore_ascii_case("q") {
                        c.stop_requested = true;
                    } else if ev.code == "Space" || ev.code == "KeyK" || ev.text == " " {
                        c.paused = !c.paused;
                    } else if ev.code == "ArrowUp" {
                        c.muted = false;
                        c.volume = (c.volume + 0.1).min(1.0);
                    } else if ev.code == "ArrowDown" {
                        c.volume = (c.volume - 0.1).max(0.0);
                    } else if ev.code == "KeyM" || ev.text.eq_ignore_ascii_case("m") {
                        c.muted = !c.muted;
                    } else if ev.code == "ArrowRight" || ev.text.eq_ignore_ascii_case("l") {
                        c.seek_request = Some(engine::seek_from_clock(10_000_000)); // +10 s
                    } else if ev.code == "ArrowLeft" || ev.text.eq_ignore_ascii_case("j") {
                        c.seek_request = Some(engine::seek_from_clock(-10_000_000)); // −10 s
                    } else if ev.code == "Home" {
                        c.seek_request = Some(0); // restart
                    } else if ev.code == "KeyS" || ev.text.eq_ignore_ascii_case("s") {
                        // Cycle subtitles: off → track0 → track1 → … → off.
                        c.sub_sel = match c.sub_sel {
                            None if n_subs > 0 => Some(0),
                            Some(i) if i + 1 < n_subs => Some(i + 1),
                            _ => None,
                        };
                        c.sub_dirty = true;
                    } else if ev.code == "KeyA" || ev.text.eq_ignore_ascii_case("a") {
                        // Cycle audio tracks (MKV, in-place); count from the active stream.
                        if n_audio > 1 {
                            c.audio_pref = (c.audio_pref + 1) % n_audio;
                            c.audio_switch = true;
                        }
                    }
                });
                return;
            }
            // Resolving/Ready: Escape/Backspace/q aborts back to the list.
            if matches!(e.phase, Phase::Resolving | Phase::Ready) {
                if matches!(ev.code.as_str(), "Escape" | "Backspace")
                    || ev.text.eq_ignore_ascii_case("q")
                {
                    engine::CONTROLS.with(|c| c.borrow_mut().stop_requested = true);
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
                "Tab" => {
                    let t = if e.tab == Tab::Movies { Tab::Shows } else { Tab::Movies };
                    set_tab(&mut e, t);
                }
                "Escape" | "Backspace" => { back_up(&mut e); } // up one Shows level
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
                // Transport bar geometry from the engine; intents written to CONTROLS.
                let (sw, sh) = engine::CONTROLS.with(|c| c.borrow().surface);
                let bar = engine::control_bar(sw as f32, sh as f32);
                let scrub_hit = (bar.scrub.0, bar.scrub.1 - 8.0, bar.scrub.2, bar.scrub.3 + 16.0);
                let scrub_frac = ((ev.x - bar.scrub.0) / bar.scrub.2).clamp(0.0, 1.0);
                engine::CONTROLS.with(|c| {
                    let mut c = c.borrow_mut();
                    c.controls_bump = true;
                    match ev.kind {
                        PtrKind::Down if matches!(ev.button, Button::Primary) => {
                            if engine::hit(ev.x, ev.y, bar.playpause) {
                                c.paused = !c.paused;
                            } else if engine::hit(ev.x, ev.y, bar.stop) {
                                c.stop_requested = true;
                            } else if engine::hit(ev.x, ev.y, bar.mute) {
                                c.muted = !c.muted;
                            } else if engine::hit(ev.x, ev.y, bar.vol) {
                                c.muted = false;
                                c.volume = ((ev.x - bar.vol.0) / bar.vol.2).clamp(0.0, 1.0);
                            } else if engine::hit(ev.x, ev.y, scrub_hit) {
                                c.scrubbing = true;
                                c.scrub_frac = scrub_frac;
                            }
                        }
                        PtrKind::Move if c.scrubbing => c.scrub_frac = scrub_frac,
                        PtrKind::Up if c.scrubbing => {
                            c.scrubbing = false;
                            let dur = engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.duration_us()).unwrap_or(0));
                            c.seek_request = Some((c.scrub_frac as f64 * dur as f64) as i64);
                        }
                        PtrKind::Cancel | PtrKind::Leave if c.scrubbing => c.scrubbing = false, // abort
                        _ => {}
                    }
                });
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
                    if let Some(t) = tab_hit(ev.x, ev.y, 20.0) {
                        set_tab(&mut e, t);
                    } else {
                        let (top, row_h, first, vis) = e.list_view;
                        let n = visible(&e).len();
                        if ev.y >= top && row_h > 0.0 {
                            let row = first + ((ev.y - top) / row_h) as usize;
                            if row < first + vis && row < n {
                                // First click selects; clicking the already-selected
                                // row activates it (drill a series / play an item).
                                if row == e.selected {
                                    activate(&mut e);
                                } else {
                                    e.selected = row;
                                }
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
        // Drill into the Shows hierarchy (async fetches): Series → Seasons → Episodes.
        if let Some((idx, sid)) = ENGINE.with(|e| e.borrow_mut().pending_seasons.take()) {
            reqwest::task::spawn(open_seasons(idx, sid));
        }
        if let Some((idx, series_id, season_id)) = ENGINE.with(|e| e.borrow_mut().pending_episodes.take()) {
            reqwest::task::spawn(open_episodes(idx, series_id, season_id));
        }

        // Stop requested (Escape/Backspace while Playing): tear the stream down
        // — dropping StreamPlayer releases the decoder surface + audio device. The
        // stop intent lives on engine::CONTROLS; the phase/resolving is app state.
        let stop = engine::CONTROLS.with(|c| {
            let mut c = c.borrow_mut();
            if c.stop_requested { c.stop_requested = false; true } else { false }
        });
        if stop {
            ENGINE.with(|e| {
                let mut e = e.borrow_mut();
                e.phase = Phase::Browse;
                e.resolving = false;
            });
            // B3: report the final position to Jellyfin before tearing down (saves
            // the resume point + watched status). `resolved` still holds this item.
            let final_us = engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.clock_us()).unwrap_or(0));
            spawn_report(Report::Stopped, final_us, false);
            engine::STREAM.with(|s| *s.borrow_mut() = None);
            // Drop the stream's audio still buffered in the shared device, but
            // keep the device OPEN (reopening churns COM on Windows/WASAPI).
            engine::with_audio(|pb| pb.flush());
            log("stopped — back to list");
        }

        // A resolved title opens here (async context) — the container demuxers'
        // block_on reader is only legal in an async-lifted export, not on-frame.
        // The engine's open fns take title + duration (µs); on success the app sets
        // Playing + reports/resumes; on failure it logs and returns to the list.
        if let Some((url, total, item, surface, is_mkv)) = ENGINE.with(|e| e.borrow_mut().pending_open.take()) {
            let title = item.name.clone();
            let dur_us = (item.duration_s() * 1_000_000.0) as i64;
            let opened = if is_mkv {
                engine::open_mkv_sync(url, total, title, dur_us, surface)
            } else {
                engine::open_mp4_sync(url, total, title, dur_us, surface)
            };
            match opened {
                Ok(()) => { set_phase(Phase::Playing); on_stream_started(); }
                Err(msg) => { log(format!("stream: open failed — {msg}")); set_phase(Phase::Browse); }
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
            let handle = engine::STREAM.with(|s| s.borrow().as_ref().and_then(|p| p.prefetch_handle()));
            if let Some(h) = handle {
                engine::drive_prefetch(&h);
            }
            // Tier 3: subtitle track change → fetch + parse the VTT (or clear).
            if engine::CONTROLS.with(|c| std::mem::take(&mut c.borrow_mut().sub_dirty)) {
                let sub_sel = engine::CONTROLS.with(|c| c.borrow().sub_sel);
                let sel = ENGINE.with(|e| {
                    let e = e.borrow();
                    sub_sel
                        .and_then(|i| e.resolved.as_ref().and_then(|(item, pb, _)| {
                            pb.subtitles.get(i).map(|st|
                                (item.id.clone(), pb.media_source_id.clone(), st.index, st.label.clone()))
                        }))
                        .zip(e.session.clone())
                });
                engine::SUBTITLES.with(|s| s.borrow_mut().clear());
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
            // C2: audio-track switch (in-place, MKV) — re-route + rebuild the decoder,
            // then re-seek to the current position so the single sequential MKV cursor
            // re-reads the switched track from here (its packets for the already-
            // buffered region were skipped as we read ahead). do_seek does the queue/
            // ring/clock reset; the video re-anchors at the nearest keyframe.
            if engine::CONTROLS.with(|c| std::mem::take(&mut c.borrow_mut().audio_switch)) {
                let pref = engine::CONTROLS.with(|c| c.borrow().audio_pref);
                let cur = engine::STREAM.with(|s| {
                    let mut g = s.borrow_mut();
                    g.as_mut().map(|p| { engine::switch_audio(p, pref); p.clock_us() })
                });
                if let Some(cur) = cur {
                    engine::CONTROLS.with(|c| c.borrow_mut().seek_request = Some(cur));
                }
            }
            let seek = engine::CONTROLS.with(|c| c.borrow_mut().seek_request.take());
            engine::STREAM.with(|s| {
                if let Some(p) = s.borrow_mut().as_mut() {
                    if let Some(target) = seek {
                        engine::do_seek(p, target); // reposition + reset before refilling
                    }
                    engine::fill_queues(p);   // demux → raw video/audio queues
                    engine::decode_audio(p);  // raw audio → decoded PCM (off the on-frame path)
                }
            });
            // B3: report progress every ~10 s of MEDIA time (bg-tick has no wall
            // clock, so throttle on the playback clock, not real time).
            let clk = engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.clock_us()).unwrap_or(0));
            let due = ENGINE.with(|e| {
                let mut e = e.borrow_mut();
                if clk - e.last_report_us >= 10_000_000 { e.last_report_us = clk; true } else { false }
            });
            if due {
                let paused = engine::CONTROLS.with(|c| c.borrow().paused);
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
