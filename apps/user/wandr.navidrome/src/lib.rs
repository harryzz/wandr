//! wandr.navidrome — a Subsonic/OpenSubsonic (Navidrome) music client.
//!
//! Talks the Subsonic REST API via the `opensubsonic` crate (a vendored fork wired to
//! our `wasi:tls` transport — see crates/opensubsonic-rs) and plays the returned audio
//! through the SHIPPED engine audio path (`engine::open_audio_sync` → Demux::Audio →
//! symphonia demux+decode → `wasi:audio` + the master clock + seek). First cut:
//! `get_random_songs` → a scrollable list; Enter plays the selected song via
//! `stream?…&format=raw`; the engine transport drives playback; Esc returns to the list.
//!
//! CONFIG is read from `/state/navidrome/config.json` (server/user/pass) — never baked
//! in. Token auth (md5(pass+salt)) is done by the `opensubsonic` crate.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    world: "wandr:navidrome/navidrome-app",
    path: "wit",
});

use wandr_media_engine as engine;
use engine::canvas::draw::Canvas;

use crate::exports::wandr::background::background::Guest as BgGuest;
use crate::exports::wandr::ui_shell::frame_pacing::Guest as PacingGuest;
use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::key_handler::{Guest as KeyGuest, KeyEvent};
use crate::exports::wasi::input_handlers::pointer_handler::{
    Button, Guest as PointerGuest, Kind as PtrKind, PointerEvent,
};

use opensubsonic::data::Child;
use opensubsonic::{Auth, Client};
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

// ---- app state -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Loading,
    Browse,
    Opening,
    Playing,
    Error,
}

struct App {
    phase: Phase,
    driver_spawned: bool,
    /// The typed Subsonic client (built once in the driver); stream URLs are derived from
    /// it synchronously. `None` until connected.
    client: Option<Client>,
    songs: Vec<Child>,
    sel: usize,
    pending_play: Option<Child>,
    /// Async open handoff (url, total_len, title, duration_us) → bg-tick open_audio_sync.
    pending_open: Option<(String, u64, String, i64)>,
    now: String,
    last: String,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        phase: Phase::Loading,
        driver_spawned: false,
        client: None,
        songs: Vec::new(),
        sel: 0,
        pending_play: None,
        pending_open: None,
        now: String::new(),
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
/// A `wandr-reqwest` client for the byte-range size probe (the engine streams the media
/// itself; `opensubsonic` only builds the authed URLs + the API calls).
fn probe_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("wandr-navidrome/0.1 ( https://github.com/harryzz/wandr )")
        .build()
        .ok()
}

/// Async: read config, build the client, authenticate, load songs to browse.
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
    match client.get_random_songs(Some(200), None, None, None, None).await {
        Ok(songs) if !songs.is_empty() => {
            note(format!("{} songs", songs.len()));
            APP.with(|a| {
                let mut a = a.borrow_mut();
                a.songs = songs;
                a.sel = 0;
                a.client = Some(client);
            });
            set_phase(Phase::Browse);
        }
        Ok(_) => {
            note("no songs returned");
            set_phase(Phase::Error);
        }
        Err(e) => {
            note(format!("library: {e}"));
            set_phase(Phase::Error);
        }
    }
}

/// Async: build the stream URL from the client, probe its size, hand off to bg-tick.
async fn play(song: Child) {
    set_phase(Phase::Opening);
    // stream_url is synchronous (URL + auth only) — pull it from the stored client.
    let url = match APP.with(|a| a.borrow().client.as_ref().and_then(|c| c.stream_url(&song.id, None, Some("raw")).ok())) {
        Some(u) => u.to_string(),
        None => {
            note("no stream url");
            return set_phase(Phase::Browse);
        }
    };
    let Some(hc) = probe_client() else {
        note("no HTTP client");
        return set_phase(Phase::Browse);
    };
    let total_len = match engine::net::fetch_range(&hc, &url, 0, Some(0)).await {
        Ok(r) if r.total_len > 0 => r.total_len,
        Ok(_) => {
            note("stream: server did not report a length");
            return set_phase(Phase::Browse);
        }
        Err(e) => {
            note(format!("stream probe: {e}"));
            return set_phase(Phase::Browse);
        }
    };
    let title = format!(
        "{} — {}",
        song.artist.clone().unwrap_or_default(),
        song.title
    );
    // Subsonic's scanned duration is authoritative → exact transport total.
    let dur_us = song.duration.unwrap_or(0).max(0) * 1_000_000;
    APP.with(|a| {
        let mut a = a.borrow_mut();
        a.now = title.clone();
        a.pending_open = Some((url, total_len, title, dur_us));
    });
}

// ---- exports ---------------------------------------------------------------

struct Component;

impl FrameGuest for Component {
    fn on_frame(nanos: u64) {
        if APP.with(|a| a.borrow().phase == Phase::Playing) {
            engine::pump_stream(nanos);
            engine::render_playing(nanos);
        } else {
            render();
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
        let phase = APP.with(|a| a.borrow().phase);
        match phase {
            Phase::Browse => APP.with(|a| {
                let mut a = a.borrow_mut();
                let n = a.songs.len();
                if n == 0 {
                    return;
                }
                match ev.code.as_str() {
                    "ArrowDown" | "KeyJ" => a.sel = (a.sel + 1) % n,
                    "ArrowUp" | "KeyK" => a.sel = (a.sel + n - 1) % n,
                    "PageDown" => a.sel = (a.sel + 10).min(n - 1),
                    "PageUp" => a.sel = a.sel.saturating_sub(10),
                    "Home" => a.sel = 0,
                    "End" => a.sel = n - 1,
                    "Enter" | "Space" => {
                        let song = a.songs[a.sel].clone();
                        a.pending_play = Some(song);
                    }
                    _ => {}
                }
            }),
            Phase::Playing => {
                engine::CONTROLS.with(|c| {
                    let mut c = c.borrow_mut();
                    c.controls_bump = true;
                    if matches!(ev.code.as_str(), "Escape" | "Backspace") || ev.text.eq_ignore_ascii_case("q") {
                        c.stop_requested = true;
                    } else if ev.code == "Space" || ev.code == "KeyK" {
                        c.paused = !c.paused;
                    } else if ev.code == "ArrowUp" {
                        c.muted = false;
                        c.volume = (c.volume + 0.1).min(1.0);
                    } else if ev.code == "ArrowDown" {
                        c.volume = (c.volume - 0.1).max(0.0);
                    } else if ev.code == "KeyM" || ev.text.eq_ignore_ascii_case("m") {
                        c.muted = !c.muted;
                    } else if ev.code == "ArrowRight" || ev.text.eq_ignore_ascii_case("l") {
                        c.seek_request = Some(engine::seek_from_clock(10_000_000));
                    } else if ev.code == "ArrowLeft" || ev.text.eq_ignore_ascii_case("j") {
                        c.seek_request = Some(engine::seek_from_clock(-10_000_000));
                    } else if ev.code == "Home" {
                        c.seek_request = Some(0);
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
        if phase == Phase::Browse {
            if matches!(ev.kind, PtrKind::Down) && matches!(ev.button, Button::Primary) {
                let (_, sh) = engine::CONTROLS.with(|c| c.borrow().surface);
                let row = ((ev.y - LIST_TOP) / ROW_H).floor();
                if row >= 0.0 && ev.y < sh as f32 {
                    APP.with(|a| {
                        let mut a = a.borrow_mut();
                        let top = list_top_index(a.sel, a.songs.len(), sh as f32);
                        let idx = top + row as usize;
                        if idx < a.songs.len() {
                            if idx == a.sel {
                                let song = a.songs[idx].clone();
                                a.pending_play = Some(song);
                            } else {
                                a.sel = idx;
                            }
                        }
                    });
                }
            }
            return;
        }
        if phase != Phase::Playing {
            return;
        }
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
            Phase::Playing => 16,
            Phase::Loading | Phase::Opening => 100,
            _ => 200,
        })
    }
}

impl BgGuest for Component {
    async fn bg_tick() -> u32 {
        if APP.with(|a| {
            let mut a = a.borrow_mut();
            if !a.driver_spawned { a.driver_spawned = true; true } else { false }
        }) {
            reqwest::task::spawn(driver());
        }

        if let Some(song) = APP.with(|a| a.borrow_mut().pending_play.take()) {
            reqwest::task::spawn(play(song));
        }

        if engine::CONTROLS.with(|c| std::mem::take(&mut c.borrow_mut().stop_requested)) {
            engine::STREAM.with(|s| *s.borrow_mut() = None);
            let _ = engine::with_audio(|pb| pb.flush());
            APP.with(|a| a.borrow_mut().now.clear());
            set_phase(Phase::Browse);
        }

        if let Some((u, total, title, dur)) = APP.with(|a| a.borrow_mut().pending_open.take()) {
            let surface = engine::CONTROLS.with(|c| c.borrow().surface);
            match engine::open_audio_sync(u, total, title, dur, surface) {
                Ok(()) => set_phase(Phase::Playing),
                Err(e) => {
                    note(format!("open failed: {e}"));
                    set_phase(Phase::Browse);
                }
            }
        }

        // Track finished playing → tear down and return to the library.
        if APP.with(|a| a.borrow().phase == Phase::Playing)
            && engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.is_ended()).unwrap_or(false))
        {
            engine::STREAM.with(|s| *s.borrow_mut() = None);
            let _ = engine::with_audio(|pb| pb.flush());
            APP.with(|a| a.borrow_mut().now.clear());
            set_phase(Phase::Browse);
        }

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
            Phase::Playing => 8,
            Phase::Loading | Phase::Opening => 30,
            _ => 120,
        })
    }
}

// ---- browse UI -------------------------------------------------------------

const LIST_TOP: f32 = 96.0;
const ROW_H: f32 = 28.0;

fn list_top_index(sel: usize, n: usize, sh: f32) -> usize {
    let rows = (((sh - LIST_TOP) / ROW_H).floor() as usize).max(1);
    if n <= rows {
        0
    } else {
        sel.saturating_sub(rows / 2).min(n - rows)
    }
}

fn render() {
    let cv: Canvas = engine::wctx(|x| x.get_current_buffer());
    let (sw, sh) = engine::CONTROLS.with(|c| c.borrow().surface);
    let (sw, sh) = (sw as f32, sh as f32);
    let pad = 20.0;
    engine::draw_text(&cv, "Navidrome — random songs", pad, 30.0, 22.0, 700, 0xFFFF_FFFF, sw - 2.0 * pad);

    let (phase, last, sel, n) = APP.with(|a| {
        let a = a.borrow();
        (a.phase, a.last.clone(), a.sel, a.songs.len())
    });
    let status = match phase {
        Phase::Loading => "connecting…".to_string(),
        Phase::Opening => "opening stream…".to_string(),
        Phase::Error => format!("error: {last}"),
        _ => format!("{n} songs   ↑/↓ select · Enter play   ({last})"),
    };
    engine::draw_text(&cv, &status, pad, 60.0, 14.0, 500, 0xFF8A_9098, sw - 2.0 * pad);

    if phase == Phase::Browse {
        let rows = (((sh - LIST_TOP) / ROW_H).floor() as usize).max(1);
        let top = list_top_index(sel, n, sh);
        APP.with(|a| {
            let a = a.borrow();
            for i in 0..rows.min(n.saturating_sub(top)) {
                let idx = top + i;
                let s = &a.songs[idx];
                let y = LIST_TOP + i as f32 * ROW_H + 18.0;
                let (color, weight) = if idx == sel { (0xFFFF_FFFF, 700) } else { (0xFFB0_B4BC, 400) };
                if idx == sel {
                    engine::draw_text(&cv, "▶", pad, y, 15.0, 700, 0xFF4A_C0FF, 20.0);
                }
                let d = s.duration.unwrap_or(0).max(0);
                let dur = format!("{}:{:02}", d / 60, d % 60);
                let line = format!(
                    "{}   {} · {}   [{}] {}",
                    s.title,
                    s.artist.clone().unwrap_or_default(),
                    s.album.clone().unwrap_or_default(),
                    s.suffix.clone().unwrap_or_default(),
                    dur
                );
                engine::draw_text(&cv, &line, pad + 22.0, y, 14.0, weight, color, sw - 2.0 * pad - 22.0);
            }
        });
    }
    engine::wctx(|x| x.present());
}

export!(Component);
