//! wandr.navidrome — a Subsonic/OpenSubsonic (Navidrome) music client.
//!
//! Talks the Subsonic REST API via the `opensubsonic` crate (a vendored fork wired to
//! our `wasi:tls` transport — see crates/opensubsonic-rs) and plays the returned audio
//! through the SHIPPED engine audio path (`engine::open_audio_sync` → Demux::Audio →
//! symphonia demux+decode → `wasi:audio` + the master clock + seek).
//!
//! UI is a navigation STACK of list screens:
//!   Menu → { Albums · Artists · Playlists · Random · Search }
//!   Albums → Album (tracklist)      Artists → Artist (albums) → Album
//!   Playlists → Playlist (tracks)   Search → mixed results
//! Enter on a SONG builds a play QUEUE from that screen's songs and starts playback;
//! Esc pops the stack. The now-playing screen shows COVER ART (host-decoded via the
//! canvas `graphics` factory) behind the shared transport chrome, with next/prev over
//! the queue and auto-advance at end-of-track.
//!
//! CONFIG is read from `/state/navidrome/config.json` (server/user/pass) — never baked
//! in. All network I/O is async (spawned onto the reqwest executor); the sync
//! on_frame/on_key/on_pointer paths only mutate app state and hand work to `bg_tick`.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    world: "wandr:navidrome/navidrome-app",
    path: "wit",
});

use wandr_media_engine as engine;
use engine::canvas::draw::Canvas;
use engine::canvas::types::Image;

use crate::exports::wandr::background::background::Guest as BgGuest;
use crate::exports::wandr::ui_shell::frame_pacing::Guest as PacingGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::key_handler::{Guest as KeyGuest, KeyEvent};
use crate::exports::wasi::input_handlers::pointer_handler::{
    Button, Guest as PointerGuest, Kind as PtrKind, PointerEvent,
};

use opensubsonic::data::Child;
use opensubsonic::{AlbumListType, Auth, Client};
use serde::Deserialize;
use std::cell::RefCell;

// ---- config (read from the per-app /state preopen, NOT baked in) -----------
const STATE_DIR: &str = "/state/navidrome";
const CONFIG_PATH: &str = "/state/navidrome/config.json";
const CONFIG_TEMPLATE: &str = r#"{
  "server": "https://music.example.com",
  "user": "youruser",
  "pass": "yourpassword"
}
"#;

#[derive(Default, Clone, Deserialize)]
struct Config {
    #[serde(default)]
    server: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    pass: String,
}

/// Load config from `/state/navidrome/config.json`; on first run write the template and
/// ask the user to edit it (server + password live in /state, never in the binary/git).
fn load_config() -> Result<Config, String> {
    let text = match std::fs::read_to_string(CONFIG_PATH) {
        Ok(t) => t,
        Err(_) => {
            let _ = std::fs::create_dir_all(STATE_DIR);
            let _ = std::fs::write(CONFIG_PATH, CONFIG_TEMPLATE);
            return Err(format!("edit {CONFIG_PATH} (server/user/pass), then relaunch"));
        }
    };
    let mut cfg: Config = serde_json::from_str(&text).map_err(|e| format!("{CONFIG_PATH}: {e}"))?;
    cfg.server = cfg.server.trim_end_matches('/').to_string();
    if cfg.server.is_empty() || cfg.user.is_empty() {
        return Err(format!("{CONFIG_PATH}: need \"server\" and \"user\""));
    }
    Ok(cfg)
}

// ---- navigation model ------------------------------------------------------

/// A top-level browse destination (the Menu entries; also what a fetch targets).
#[derive(Clone, Copy, PartialEq)]
enum Dest {
    Albums,
    Artists,
    Playlists,
    Random,
    Search,
}

impl Dest {
    fn label(self) -> &'static str {
        match self {
            Dest::Albums => "Albums",
            Dest::Artists => "Artists",
            Dest::Playlists => "Playlists",
            Dest::Random => "Random songs",
            Dest::Search => "Search…",
        }
    }
}

/// One row in a list screen. A `Song` is playable (Enter → queue+play); the rest drill
/// into another fetch.
#[derive(Clone)]
enum Row {
    Song(Child),
    Album { id: String, name: String, sub: String },
    Artist { id: String, name: String },
    Playlist { id: String, name: String, count: i64 },
    Menu(Dest),
}

/// A pushed list screen. `cover` is the header artwork id (album/playlist), if any.
struct Screen {
    title: String,
    rows: Vec<Row>,
    sel: usize,
    cover: Option<String>,
}

fn menu_screen() -> Screen {
    Screen {
        title: "Navidrome".into(),
        sel: 0,
        cover: None,
        rows: vec![
            Row::Menu(Dest::Albums),
            Row::Menu(Dest::Artists),
            Row::Menu(Dest::Playlists),
            Row::Menu(Dest::Random),
            Row::Menu(Dest::Search),
        ],
    }
}

/// A deferred network fetch, queued by the sync input path and run in `bg_tick`.
#[derive(Clone)]
enum Nav {
    Dest(Dest),
    Album(String),
    Artist(String),
    Playlist(String),
    Search(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Loading,
    Ready,
    Opening,
    Playing,
    Error,
}

// ---- app state -------------------------------------------------------------

struct App {
    phase: Phase,
    driver_spawned: bool,
    /// The typed Subsonic client (built once in the driver). `None` until connected.
    client: Option<Client>,
    /// Navigation stack; `stack.last()` is the visible screen. Empty until connected.
    stack: Vec<Screen>,
    pending_nav: Option<Nav>,
    busy: bool,
    /// Search input mode + buffer.
    searching: bool,
    query: String,
    // playback queue -----------------------------------------------------
    queue: Vec<Child>,
    qidx: usize,
    /// Queue index to (re)open — initial play, next/prev, and auto-advance all set this.
    pending_play: Option<usize>,
    /// Async open handoff (url, total_len, title, duration_us) → bg-tick open_audio_sync.
    pending_open: Option<(String, u64, String, i64)>,
    // cover art (single decoded slot; host-side Skia decode) --------------
    cover: Option<Image>,
    /// Which cover-art id is decoded into `cover` (or attempted, on failure).
    cover_id: Option<String>,
    /// A cover fetch is in flight (avoids duplicate spawns).
    cover_fetching: bool,
    /// Fetched bytes awaiting sync decode in bg_tick.
    pending_cover: Option<(String, Vec<u8>)>,
    last: String,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        phase: Phase::Loading,
        driver_spawned: false,
        client: None,
        stack: Vec::new(),
        pending_nav: None,
        busy: false,
        searching: false,
        query: String::new(),
        queue: Vec::new(),
        qidx: 0,
        pending_play: None,
        pending_open: None,
        cover: None,
        cover_id: None,
        cover_fetching: false,
        pending_cover: None,
        last: String::new(),
    });
}

fn set_phase(p: Phase) {
    APP.with(|a| a.borrow_mut().phase = p);
}
fn note(msg: impl Into<String>) {
    let m = msg.into();
    APP.with(|a| a.borrow_mut().last = m.clone());
    engine::log(m);
}
fn client() -> Option<Client> {
    APP.with(|a| a.borrow().client.clone())
}
/// Push a freshly-fetched screen and return to browsing.
fn push_screen(s: Screen) {
    APP.with(|a| {
        let mut a = a.borrow_mut();
        a.stack.push(s);
        a.busy = false;
        a.phase = Phase::Ready;
    });
}
fn fail(msg: impl Into<String>) {
    note(msg);
    APP.with(|a| {
        let mut a = a.borrow_mut();
        a.busy = false;
        // Stay in whatever browse context we came from; surface the error in status.
        if a.stack.is_empty() {
            a.phase = Phase::Error;
        } else {
            a.phase = Phase::Ready;
        }
    });
}

/// A `wandr-reqwest` client for the byte-range size probe (the engine streams the media
/// itself; `opensubsonic` only builds the authed URLs + the API calls).
fn probe_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("wandr-navidrome/0.1 ( https://github.com/harryzz/wandr )")
        .build()
        .ok()
}

// ---- async driver + fetches ------------------------------------------------

/// Async: read config, build the client, authenticate, show the top-level menu.
async fn driver() {
    let cfg = match load_config() {
        Ok(c) => c,
        Err(e) => {
            note(e);
            return set_phase(Phase::Error);
        }
    };
    // Plaintext `u`/`p` over HTTPS: this server rejects token auth (Subsonic error 41).
    let client = match Client::new(&cfg.server, Auth::plain(&cfg.user, &cfg.pass)) {
        Ok(c) => c.with_client_name("wandr"),
        Err(e) => {
            note(format!("client: {e}"));
            return set_phase(Phase::Error);
        }
    };
    if let Err(e) = client.ping().await {
        note(format!("login failed: {e}"));
        return set_phase(Phase::Error);
    }
    note(format!("connected to {}", cfg.server));
    APP.with(|a| {
        let mut a = a.borrow_mut();
        a.client = Some(client);
        a.stack = vec![menu_screen()];
        a.phase = Phase::Ready;
    });
}

fn album_row(a: opensubsonic::data::AlbumId3) -> Row {
    let mut sub = a.artist.clone().unwrap_or_default();
    if let Some(y) = a.year {
        if !sub.is_empty() {
            sub.push_str("  ·  ");
        }
        sub.push_str(&y.to_string());
    }
    Row::Album { id: a.id, name: a.name, sub }
}

/// Async: run the fetch behind a `Nav` and push the resulting screen.
async fn fetch(nav: Nav) {
    let Some(c) = client() else {
        return fail("not connected");
    };
    match nav {
        Nav::Dest(Dest::Albums) => {
            match c
                .get_album_list2(AlbumListType::AlphabeticalByName, Some(200), None, None, None, None, None)
                .await
            {
                Ok(albums) => push_screen(Screen {
                    title: "Albums".into(),
                    sel: 0,
                    cover: None,
                    rows: albums.into_iter().map(album_row).collect(),
                }),
                Err(e) => fail(format!("albums: {e}")),
            }
        }
        Nav::Dest(Dest::Artists) => match c.get_artists(None).await {
            Ok(idx) => {
                let rows: Vec<Row> = idx
                    .index
                    .into_iter()
                    .flat_map(|i| i.artist)
                    .map(|ar| Row::Artist { id: ar.id, name: ar.name })
                    .collect();
                push_screen(Screen { title: "Artists".into(), sel: 0, cover: None, rows });
            }
            Err(e) => fail(format!("artists: {e}")),
        },
        Nav::Dest(Dest::Playlists) => match c.get_playlists(None).await {
            Ok(pls) => {
                let rows: Vec<Row> = pls
                    .into_iter()
                    .map(|p| Row::Playlist {
                        id: p.id,
                        name: p.name,
                        count: p.song_count.unwrap_or(0),
                    })
                    .collect();
                push_screen(Screen { title: "Playlists".into(), sel: 0, cover: None, rows });
            }
            Err(e) => fail(format!("playlists: {e}")),
        },
        Nav::Dest(Dest::Random) => {
            match c.get_random_songs(Some(200), None, None, None, None).await {
                Ok(songs) => push_screen(Screen {
                    title: "Random songs".into(),
                    sel: 0,
                    cover: None,
                    rows: songs.into_iter().map(Row::Song).collect(),
                }),
                Err(e) => fail(format!("random: {e}")),
            }
        }
        Nav::Dest(Dest::Search) => {} // opened via the search overlay, not a fetch
        Nav::Album(id) => match c.get_album(&id).await {
            Ok(al) => push_screen(Screen {
                title: al.name,
                sel: 0,
                cover: al.cover_art,
                rows: al.song.into_iter().map(Row::Song).collect(),
            }),
            Err(e) => fail(format!("album: {e}")),
        },
        Nav::Artist(id) => match c.get_artist(&id).await {
            Ok(ar) => push_screen(Screen {
                title: ar.name,
                sel: 0,
                cover: ar.cover_art,
                rows: ar.album.into_iter().map(album_row).collect(),
            }),
            Err(e) => fail(format!("artist: {e}")),
        },
        Nav::Playlist(id) => match c.get_playlist(&id).await {
            Ok(pl) => push_screen(Screen {
                title: pl.name,
                sel: 0,
                cover: pl.cover_art,
                rows: pl.entry.into_iter().map(Row::Song).collect(),
            }),
            Err(e) => fail(format!("playlist: {e}")),
        },
        Nav::Search(q) => {
            match c
                .search3(&q, Some(20), None, Some(20), None, Some(80), None, None)
                .await
            {
                Ok(r) => {
                    let mut rows: Vec<Row> = Vec::new();
                    rows.extend(r.artist.into_iter().map(|a| Row::Artist { id: a.id, name: a.name }));
                    rows.extend(r.album.into_iter().map(album_row));
                    rows.extend(r.song.into_iter().map(Row::Song));
                    if rows.is_empty() {
                        note(format!("no results for \"{q}\""));
                    }
                    push_screen(Screen { title: format!("Search: {q}"), sel: 0, cover: None, rows });
                }
                Err(e) => fail(format!("search: {e}")),
            }
        }
    }
}

/// Async: build the stream URL for `queue[idx]`, probe its size, hand off to bg-tick.
async fn play(idx: usize) {
    let Some((song, c)) = APP.with(|a| {
        let a = a.borrow();
        a.queue.get(idx).cloned().zip(a.client.clone())
    }) else {
        return set_phase(Phase::Ready);
    };
    // stream_url is synchronous (URL + auth only); `format=raw` = no server transcode.
    let url = match c.stream_url(&song.id, None, Some("raw")) {
        Ok(u) => u.to_string(),
        Err(e) => {
            note(format!("stream url: {e}"));
            return set_phase(Phase::Ready);
        }
    };
    let Some(hc) = probe_client() else {
        note("no HTTP client");
        return set_phase(Phase::Ready);
    };
    let total_len = match engine::net::fetch_range(&hc, &url, 0, Some(0)).await {
        Ok(r) if r.total_len > 0 => r.total_len,
        Ok(_) => {
            note("stream: server did not report a length");
            return set_phase(Phase::Ready);
        }
        Err(e) => {
            note(format!("stream probe: {e}"));
            return set_phase(Phase::Ready);
        }
    };
    let title = format!("{} — {}", song.artist.clone().unwrap_or_default(), song.title);
    // Subsonic's scanned duration is authoritative → exact transport total.
    let dur_us = song.duration.unwrap_or(0).max(0) * 1_000_000;
    APP.with(|a| a.borrow_mut().pending_open = Some((url, total_len, title, dur_us)));
}

/// Async: fetch cover-art bytes (decoded synchronously in bg_tick).
async fn fetch_cover(id: String) {
    let Some(c) = client() else { return };
    match c.get_cover_art(&id, Some(400)).await {
        Ok(bytes) => APP.with(|a| a.borrow_mut().pending_cover = Some((id, bytes.to_vec()))),
        Err(_) => APP.with(|a| {
            // Mark attempted so we don't refetch the same missing art every tick.
            let mut a = a.borrow_mut();
            a.cover = None;
            a.cover_id = Some(id);
            a.cover_fetching = false;
        }),
    }
}

/// The cover-art id the current view wants shown (playing song, or a detail header).
fn desired_cover() -> Option<String> {
    APP.with(|a| {
        let a = a.borrow();
        match a.phase {
            Phase::Opening | Phase::Playing => {
                a.queue.get(a.qidx).and_then(|c| c.cover_art.clone())
            }
            Phase::Ready => a.stack.last().and_then(|s| s.cover.clone()),
            _ => None,
        }
    })
}

/// Tear down the active stream (return to a clean state before opening the next track).
fn teardown_stream() {
    engine::STREAM.with(|s| *s.borrow_mut() = None);
    let _ = engine::with_audio(|pb| pb.flush());
}

// ---- playback control ------------------------------------------------------

/// Start playing `queue` from `idx` (idx clamped to range). Used by Enter-on-song.
fn start_queue(queue: Vec<Child>, idx: usize) {
    if queue.is_empty() {
        return;
    }
    APP.with(|a| {
        let mut a = a.borrow_mut();
        a.qidx = idx.min(queue.len() - 1);
        a.queue = queue;
        a.pending_play = Some(a.qidx);
    });
}

/// Skip within the queue by `delta` (±1). No wrap: clamps at the ends.
fn skip(delta: i64) {
    APP.with(|a| {
        let mut a = a.borrow_mut();
        if a.queue.is_empty() {
            return;
        }
        let n = a.queue.len() as i64;
        let next = (a.qidx as i64 + delta).clamp(0, n - 1);
        if next != a.qidx as i64 {
            a.qidx = next as usize;
            a.pending_play = Some(a.qidx);
        }
    });
}

/// Collect the current screen's songs into a queue and start at the selected song.
fn activate_selected() {
    let nav = APP.with(|a| {
        let mut a = a.borrow_mut();
        let Some(scr) = a.stack.last() else { return None };
        let Some(row) = scr.rows.get(scr.sel) else { return None };
        match row.clone() {
            Row::Menu(Dest::Search) => {
                a.searching = true;
                a.query.clear();
                None
            }
            Row::Menu(d) => Some(Nav::Dest(d)),
            Row::Album { id, .. } => Some(Nav::Album(id)),
            Row::Artist { id, .. } => Some(Nav::Artist(id)),
            Row::Playlist { id, .. } => Some(Nav::Playlist(id)),
            Row::Song(_) => {
                // Build a queue from every Song row on this screen; start at `sel`.
                let songs: Vec<Child> = scr
                    .rows
                    .iter()
                    .filter_map(|r| if let Row::Song(c) = r { Some(c.clone()) } else { None })
                    .collect();
                let start = scr.rows[..=scr.sel]
                    .iter()
                    .filter(|r| matches!(r, Row::Song(_)))
                    .count()
                    .saturating_sub(1);
                drop(a); // release before start_queue re-borrows
                start_queue(songs, start);
                return None;
            }
        }
    });
    if let Some(n) = nav {
        APP.with(|a| a.borrow_mut().pending_nav = Some(n));
    }
}

// ---- exports ---------------------------------------------------------------

struct Component;

impl FrameGuest for Component {
    fn on_frame(nanos: u64) {
        match APP.with(|a| a.borrow().phase) {
            Phase::Playing => {
                engine::pump_stream(nanos);
                render_playing_screen(nanos);
            }
            Phase::Opening => render_playing_screen(nanos),
            _ => render_browse(),
        }
    }
    fn on_resize(w: u32, h: u32) {
        engine::set_surface(w, h);
    }
}

impl KeyGuest for Component {
    fn on_key(ev: KeyEvent) {
        if !ev.down {
            return;
        }
        let (phase, searching) = APP.with(|a| {
            let a = a.borrow();
            (a.phase, a.searching)
        });

        // Search-input capture takes priority while active.
        if searching {
            APP.with(|a| {
                let mut a = a.borrow_mut();
                match ev.code.as_str() {
                    "Escape" => a.searching = false,
                    "Backspace" => {
                        a.query.pop();
                    }
                    "Enter" => {
                        a.searching = false;
                        if !a.query.trim().is_empty() {
                            a.pending_nav = Some(Nav::Search(a.query.trim().to_string()));
                        }
                    }
                    _ => {
                        // Accept a single printable character.
                        let t = ev.text.clone();
                        if t.chars().count() == 1 && !t.chars().next().unwrap().is_control() {
                            a.query.push_str(&t);
                        }
                    }
                }
            });
            return;
        }

        match phase {
            Phase::Ready => APP.with(|a| {
                let n = a.borrow().stack.last().map(|s| s.rows.len()).unwrap_or(0);
                match ev.code.as_str() {
                    "ArrowDown" | "KeyJ" if n > 0 => {
                        let mut a = a.borrow_mut();
                        if let Some(s) = a.stack.last_mut() {
                            s.sel = (s.sel + 1) % n;
                        }
                    }
                    "ArrowUp" | "KeyK" if n > 0 => {
                        let mut a = a.borrow_mut();
                        if let Some(s) = a.stack.last_mut() {
                            s.sel = (s.sel + n - 1) % n;
                        }
                    }
                    "PageDown" if n > 0 => {
                        let mut a = a.borrow_mut();
                        if let Some(s) = a.stack.last_mut() {
                            s.sel = (s.sel + 10).min(n - 1);
                        }
                    }
                    "PageUp" => {
                        let mut a = a.borrow_mut();
                        if let Some(s) = a.stack.last_mut() {
                            s.sel = s.sel.saturating_sub(10);
                        }
                    }
                    "Home" => {
                        let mut a = a.borrow_mut();
                        if let Some(s) = a.stack.last_mut() {
                            s.sel = 0;
                        }
                    }
                    "End" if n > 0 => {
                        let mut a = a.borrow_mut();
                        if let Some(s) = a.stack.last_mut() {
                            s.sel = n - 1;
                        }
                    }
                    "Enter" | "Space" => {
                        drop(a.borrow());
                        activate_selected();
                    }
                    "Escape" | "Backspace" => {
                        let mut a = a.borrow_mut();
                        if a.stack.len() > 1 {
                            a.stack.pop();
                        }
                    }
                    "Slash" | "KeyF" => {
                        let mut a = a.borrow_mut();
                        a.searching = true;
                        a.query.clear();
                    }
                    _ => {}
                }
            }),
            Phase::Playing | Phase::Opening => {
                let code = ev.code.clone();
                let text = ev.text.clone();
                engine::CONTROLS.with(|c| {
                    let mut c = c.borrow_mut();
                    c.controls_bump = true;
                    match code.as_str() {
                        "Escape" | "Backspace" => c.stop_requested = true,
                        "Space" | "KeyK" => c.paused = !c.paused,
                        "ArrowUp" => {
                            c.muted = false;
                            c.volume = (c.volume + 0.1).min(1.0);
                        }
                        "ArrowDown" => c.volume = (c.volume - 0.1).max(0.0),
                        "KeyM" => c.muted = !c.muted,
                        "ArrowRight" | "KeyL" => c.seek_request = Some(engine::seek_from_clock(10_000_000)),
                        "ArrowLeft" | "KeyH" => c.seek_request = Some(engine::seek_from_clock(-10_000_000)),
                        "Home" => c.seek_request = Some(0),
                        "KeyN" => {
                            drop(c);
                            skip(1);
                        }
                        "KeyB" | "KeyP" => {
                            drop(c);
                            skip(-1);
                        }
                        _ => {
                            if text.eq_ignore_ascii_case("q") {
                                c.stop_requested = true;
                            }
                        }
                    }
                });
            }
            _ => {}
        }
    }
}

impl PointerGuest for Component {
    fn on_pointer(ev: PointerEvent) {
        let phase = APP.with(|a| a.borrow().phase);
        if phase == Phase::Ready {
            if matches!(ev.kind, PtrKind::Down) && matches!(ev.button, Button::Primary) {
                let (_, sh) = engine::CONTROLS.with(|c| c.borrow().surface);
                let row = ((ev.y - LIST_TOP) / ROW_H).floor();
                if row >= 0.0 && ev.y < sh as f32 {
                    let activate = APP.with(|a| {
                        let mut a = a.borrow_mut();
                        let Some(s) = a.stack.last_mut() else { return false };
                        let n = s.rows.len();
                        let top = list_top_index(s.sel, n, sh as f32);
                        let idx = top + row as usize;
                        if idx < n {
                            if idx == s.sel {
                                return true; // second tap on the selected row → activate
                            }
                            s.sel = idx;
                        }
                        false
                    });
                    if activate {
                        activate_selected();
                    }
                }
            }
            return;
        }
        if phase != Phase::Playing && phase != Phase::Opening {
            return;
        }
        // Transport pointer handling (matches engine::render_transport's control_bar layout).
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
                PtrKind::Cancel | PtrKind::Leave if c.scrubbing => c.scrubbing = false,
                _ => {}
            }
        });
    }
}

impl PacingGuest for Component {
    fn next_frame_delay() -> u32 {
        APP.with(|a| match a.borrow().phase {
            Phase::Playing | Phase::Opening => 16,
            Phase::Loading => 100,
            _ => 200,
        })
    }
}

impl BgGuest for Component {
    async fn bg_tick() -> u32 {
        // Spawn the connect driver once.
        if APP.with(|a| {
            let mut a = a.borrow_mut();
            if !a.driver_spawned {
                a.driver_spawned = true;
                true
            } else {
                false
            }
        }) {
            reqwest::task::spawn(driver());
        }

        // Queued navigation fetch.
        if let Some(nav) = APP.with(|a| a.borrow_mut().pending_nav.take()) {
            APP.with(|a| a.borrow_mut().busy = true);
            reqwest::task::spawn(fetch(nav));
        }

        // Queued (re)open of a queue track — initial play, next/prev, auto-advance.
        if let Some(idx) = APP.with(|a| a.borrow_mut().pending_play.take()) {
            teardown_stream();
            set_phase(Phase::Opening);
            reqwest::task::spawn(play(idx));
        }

        // Stop requested from the transport → back to browsing.
        if engine::CONTROLS.with(|c| std::mem::take(&mut c.borrow_mut().stop_requested)) {
            teardown_stream();
            set_phase(Phase::Ready);
        }

        // The probe completed → open the audio stream and start playing.
        if let Some((u, total, title, dur)) = APP.with(|a| a.borrow_mut().pending_open.take()) {
            let surface = engine::CONTROLS.with(|c| c.borrow().surface);
            match engine::open_audio_sync(u, total, title, dur, surface) {
                Ok(()) => set_phase(Phase::Playing),
                Err(e) => {
                    note(format!("open failed: {e}"));
                    set_phase(Phase::Ready);
                }
            }
        }

        // End-of-track → advance the queue, or return to the library at the end.
        if APP.with(|a| a.borrow().phase == Phase::Playing)
            && engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.is_ended()).unwrap_or(false))
        {
            let advanced = APP.with(|a| {
                let mut a = a.borrow_mut();
                if a.qidx + 1 < a.queue.len() {
                    a.qidx += 1;
                    a.pending_play = Some(a.qidx);
                    true
                } else {
                    false
                }
            });
            if !advanced {
                teardown_stream();
                set_phase(Phase::Ready);
            }
        }

        // Cover-art reconcile: fetch what the current view wants, decode when it arrives.
        reconcile_cover();

        // Drive the open stream (seek / demux-fill / decode / prefetch).
        if engine::STREAM.with(|s| s.borrow().is_some()) {
            let seek = engine::CONTROLS.with(|c| c.borrow_mut().seek_request.take());
            engine::STREAM.with(|s| {
                if let Some(p) = s.borrow_mut().as_mut() {
                    if let Some(t) = seek {
                        engine::do_seek(p, t);
                    }
                    engine::fill_queues(p);
                    engine::decode_audio(p);
                }
            });
            if let Some(h) = engine::STREAM.with(|s| s.borrow().as_ref().and_then(|p| p.prefetch_handle())) {
                engine::drive_prefetch(&h);
            }
        }

        APP.with(|a| match a.borrow().phase {
            Phase::Playing | Phase::Opening => 8,
            Phase::Loading => 30,
            _ => 120,
        })
    }
}

/// Fetch the wanted cover art (once) and decode arrived bytes into the image slot.
fn reconcile_cover() {
    let want = desired_cover();
    let (loaded, fetching) = APP.with(|a| {
        let a = a.borrow();
        (a.cover_id.clone(), a.cover_fetching)
    });
    if want != loaded && !fetching {
        match want {
            Some(id) => {
                APP.with(|a| a.borrow_mut().cover_fetching = true);
                reqwest::task::spawn(fetch_cover(id));
            }
            None => APP.with(|a| {
                let mut a = a.borrow_mut();
                a.cover = None;
                a.cover_id = None;
            }),
        }
    }
    if let Some((id, bytes)) = APP.with(|a| a.borrow_mut().pending_cover.take()) {
        let img = engine::decode_image(&bytes);
        APP.with(|a| {
            let mut a = a.borrow_mut();
            a.cover = img;
            a.cover_id = Some(id);
            a.cover_fetching = false;
        });
    }
}

// ---- browse UI -------------------------------------------------------------

const LIST_TOP: f32 = 96.0;
const ROW_H: f32 = 30.0;

fn list_top_index(sel: usize, n: usize, sh: f32) -> usize {
    let rows = (((sh - LIST_TOP) / ROW_H).floor() as usize).max(1);
    if n <= rows {
        0
    } else {
        sel.saturating_sub(rows / 2).min(n - rows)
    }
}

fn row_glyph(r: &Row) -> (&'static str, u32) {
    match r {
        Row::Song(_) => ("\u{266A}", 0xFF7B_FFB0), // ♪
        Row::Album { .. } => ("\u{25A4}", 0xFF4A_C0FF),
        Row::Artist { .. } => ("\u{25CF}", 0xFFE0_A860),
        Row::Playlist { .. } => ("\u{2261}", 0xFFC0_88FF),
        Row::Menu(_) => ("\u{25B8}", 0xFF8A_9098),
    }
}

fn row_text(r: &Row) -> String {
    match r {
        Row::Menu(d) => d.label().to_string(),
        Row::Artist { name, .. } => name.clone(),
        Row::Playlist { name, count, .. } => format!("{name}   ({count} songs)"),
        Row::Album { name, sub, .. } => {
            if sub.is_empty() {
                name.clone()
            } else {
                format!("{name}   —   {sub}")
            }
        }
        Row::Song(s) => {
            let d = s.duration.unwrap_or(0).max(0);
            format!(
                "{}   {} · {}   {}:{:02}",
                s.title,
                s.artist.clone().unwrap_or_default(),
                s.album.clone().unwrap_or_default(),
                d / 60,
                d % 60
            )
        }
    }
}

fn render_browse() {
    let cv: Canvas = engine::wctx(|x| x.get_current_buffer());
    cv.clear(0xFF0B_0D10);
    let (sw, sh) = engine::CONTROLS.with(|c| c.borrow().surface);
    let (sw, sh) = (sw as f32, sh as f32);
    let pad = 20.0;

    let (phase, last, searching, query) = APP.with(|a| {
        let a = a.borrow();
        (a.phase, a.last.clone(), a.searching, a.query.clone())
    });

    // Header: breadcrumb title (current screen) + a small cover thumbnail if loaded.
    let title = APP.with(|a| {
        a.borrow()
            .stack
            .last()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "Navidrome".to_string())
    });
    engine::draw_text(&cv, &title, pad, 26.0, 22.0, 700, 0xFFFF_FFFF, sw - 2.0 * pad - 72.0);
    APP.with(|a| {
        let a = a.borrow();
        let header_cover = a.stack.last().and_then(|s| s.cover.as_ref());
        if let (Some(img), Some(cid)) = (a.cover.as_ref(), header_cover) {
            if a.cover_id.as_ref() == Some(cid) {
                engine::draw_image_cover(&cv, img, sw - pad - 56.0, 12.0, 56.0, 56.0);
            }
        }
    });

    let status = match phase {
        Phase::Loading => "connecting…".to_string(),
        Phase::Error => format!("error: {last}"),
        _ => {
            let n = APP.with(|a| a.borrow().stack.last().map(|s| s.rows.len()).unwrap_or(0));
            let busy = APP.with(|a| a.borrow().busy);
            if busy {
                "loading…".to_string()
            } else {
                format!("{n} items   ·   ↑/↓ Enter · Esc back · / search   ({last})")
            }
        }
    };
    engine::draw_text(&cv, &status, pad, 58.0, 13.0, 500, 0xFF8A_9098, sw - 2.0 * pad);

    // The windowed list.
    if phase == Phase::Ready {
        let rows_visible = (((sh - LIST_TOP) / ROW_H).floor() as usize).max(1);
        APP.with(|a| {
            let a = a.borrow();
            let Some(scr) = a.stack.last() else { return };
            let n = scr.rows.len();
            let top = list_top_index(scr.sel, n, sh);
            for i in 0..rows_visible.min(n.saturating_sub(top)) {
                let idx = top + i;
                let r = &scr.rows[idx];
                let y = LIST_TOP + i as f32 * ROW_H;
                let selected = idx == scr.sel;
                if selected {
                    engine::draw_rect(&cv, pad - 6.0, y - 2.0, sw - 2.0 * pad + 12.0, ROW_H, 0x2240_80C0);
                }
                let (glyph, gcol) = row_glyph(r);
                engine::draw_text(&cv, glyph, pad, y + 16.0, 15.0, 700, gcol, 22.0);
                let (color, weight) = if selected { (0xFFFF_FFFF, 700) } else { (0xFFB0_B4BC, 400) };
                engine::draw_text(
                    &cv,
                    &row_text(r),
                    pad + 24.0,
                    y + 16.0,
                    14.0,
                    weight,
                    color,
                    sw - 2.0 * pad - 24.0,
                );
            }
        });
    }

    // Search overlay.
    if searching {
        let bw = sw - 2.0 * pad;
        let by = sh * 0.33;
        engine::draw_rect(&cv, pad, by, bw, 56.0, 0xF01A_1E24);
        engine::draw_rect(&cv, pad, by, bw, 2.0, 0xFF4A_C0FF);
        engine::draw_text(&cv, "Search", pad + 12.0, by + 8.0, 12.0, 600, 0xFF8A_9098, bw - 24.0);
        let shown = if query.is_empty() { "…".to_string() } else { format!("{query}\u{2588}") };
        engine::draw_text(&cv, &shown, pad + 12.0, by + 26.0, 18.0, 600, 0xFFFF_FFFF, bw - 24.0);
    }

    engine::wctx(|x| x.present());
}

// ---- now-playing UI (cover art behind the shared transport chrome) ---------

fn render_playing_screen(nanos: u64) {
    let cv: Canvas = engine::wctx(|x| x.get_current_buffer());
    cv.clear(0xFF0B_0D10);
    let (sw, sh) = engine::CONTROLS.with(|c| c.borrow().surface);
    let (sw, sh) = (sw as f32, sh as f32);

    // Cover art: a large centered square in the upper area (placeholder if none yet).
    let side = (sw.min(sh - 260.0)).clamp(80.0, sw - 48.0);
    let cx = (sw - side) / 2.0;
    let cy = 64.0;
    APP.with(|a| {
        let a = a.borrow();
        match a.cover.as_ref() {
            Some(img) => engine::draw_image_cover(&cv, img, cx, cy, side, side),
            None => {
                engine::draw_rect(&cv, cx, cy, side, side, 0xFF1A_1E24);
                engine::draw_text(&cv, "\u{266A}", cx + side / 2.0 - 16.0, cy + side / 2.0 - 24.0, 48.0, 700, 0xFF3A_4048, side);
            }
        }
        // Metadata + queue position under the cover.
        if let Some(song) = a.queue.get(a.qidx) {
            let ty = cy + side + 22.0;
            engine::draw_text(&cv, &song.title, 24.0, ty, 22.0, 700, 0xFFFF_FFFF, sw - 110.0);
            let meta = format!(
                "{}   ·   {}",
                song.artist.clone().unwrap_or_default(),
                song.album.clone().unwrap_or_default()
            );
            engine::draw_text(&cv, &meta, 24.0, ty + 30.0, 15.0, 500, 0xFF9A_A0A8, sw - 48.0);
            let qpos = format!("{} / {}", a.qidx + 1, a.queue.len());
            engine::draw_text(&cv, &qpos, sw - 96.0, ty, 14.0, 500, 0xFF6A_7078, 84.0);
        }
        if a.phase == Phase::Opening {
            engine::draw_text(&cv, "opening…", 24.0, cy + side + 82.0, 13.0, 500, 0xFF8A_9098, sw - 48.0);
        }
    });

    // The shared transport chrome (scrub / play-pause / stop / mute / vol) — drawn over
    // our background; its layout matches control_bar so pointer hits line up. Adds a
    // hint for the queue keys.
    engine::render_transport(nanos, &cv);
    engine::draw_text(&cv, "n/b: next/prev", 12.0, sh - 96.0, 12.0, 500, 0xFF6A_7078, 160.0);

    engine::wctx(|x| x.present());
}

export!(Component);
