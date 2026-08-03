//! wandr.flac.test — the MINIMAL call site for the engine's `Demux::Audio` path.
//!
//! It fetches a raw FLAC (or MP3 / Ogg-Vorbis / WAV — symphonia handles all four the
//! same way) over HTTPS byte-range and plays it through the SHIPPED engine unchanged:
//! probe the URL for its size, `engine::open_audio_sync`, then let the engine demux +
//! decode + drive `wasi:audio` + the transport overlay. Same reactor shape as
//! wandr.dash, minus everything DASH-specific. This exists to prove the audio-only
//! source end-to-end before the Navidrome/Subsonic client is built on top.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    world: "wandr:flac-test/flac-app",
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

use std::cell::RefCell;

/// The media URL. Defaults to a battle-tested public MP3 (SoundHelix) so the wiring is
/// provable with zero setup — MP3 goes through the SAME `Demux::Audio` symphonia path
/// as FLAC/Ogg/WAV. SWAP this for a real FLAC to test lossless: your Navidrome/Subsonic
/// `…/rest/stream?id=…&format=flac&…`, or any Range-serving `.flac` (e.g. one you host).
const DEFAULT_URL: &str = "https://www.soundhelix.com/examples/mp3/SoundHelix-Song-1.mp3";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Boot,
    Loading,
    Playing,
    Ended,
}

struct App {
    phase: Phase,
    driver_spawned: bool,
    /// Set by the async driver once the size is known; consumed by bg-tick, where the
    /// `block_on` audio open is legal (an async-lifted export, never on-frame).
    pending_open: Option<(String, u64)>,
    last: String,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        phase: Phase::Boot,
        driver_spawned: false,
        pending_open: None,
        last: String::new(),
    });
}

fn set_phase(p: Phase) {
    APP.with(|a| a.borrow_mut().phase = p);
}

/// Log to the host AND keep the last line for the on-screen status.
fn note(msg: impl Into<String>) {
    let m = msg.into();
    APP.with(|a| a.borrow_mut().last = m.clone());
    engine::log(m);
}

fn build_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("wandr-flac-test/0.1 ( https://github.com/harryzz/wandr )")
        .build()
        .ok()
}

/// Async: probe the URL for its total length, then hand off to bg-tick for the open.
async fn driver() {
    set_phase(Phase::Loading);
    let Some(client) = build_client() else {
        note("no HTTP client");
        return set_phase(Phase::Ended);
    };
    let url = DEFAULT_URL.to_string();
    // A 1-byte range GET → Content-Range/Content-Length gives the total size.
    let total_len = match engine::net::fetch_range(&client, &url, 0, Some(0)).await {
        Ok(r) if r.total_len > 0 => r.total_len,
        Ok(_) => {
            note("probe: server did not report a length");
            return set_phase(Phase::Ended);
        }
        Err(e) => {
            note(format!("probe failed: {e}"));
            return set_phase(Phase::Ended);
        }
    };
    note(format!("opening — {total_len} bytes"));
    APP.with(|a| a.borrow_mut().pending_open = Some((url, total_len)));
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
        if !ev.down || APP.with(|a| a.borrow().phase != Phase::Playing) {
            return;
        }
        // Same transport bindings as jellyfin/dash: Esc/q stop; Space pause; ↑/↓ vol;
        // m mute; ←/→ (j/l) seek ∓10 s; Home restart. Intents land on engine CONTROLS.
        engine::CONTROLS.with(|c| {
            let mut c = c.borrow_mut();
            c.controls_bump = true;
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
                c.seek_request = Some(engine::seek_from_clock(10_000_000));
            } else if ev.code == "ArrowLeft" || ev.text.eq_ignore_ascii_case("j") {
                c.seek_request = Some(engine::seek_from_clock(-10_000_000));
            } else if ev.code == "Home" {
                c.seek_request = Some(0);
            }
        });
    }
}

impl PointerGuest for Component {
    fn on_pointer(ev: PointerEvent) {
        if APP.with(|a| a.borrow().phase != Phase::Playing) {
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
            Phase::Loading => 100,
            _ => 200,
        })
    }
}

impl BgGuest for Component {
    async fn bg_tick() -> u32 {
        // Spawn the driver exactly once.
        if APP.with(|a| {
            let mut a = a.borrow_mut();
            if !a.driver_spawned { a.driver_spawned = true; true } else { false }
        }) {
            reqwest::task::spawn(driver());
        }

        // Stop (Esc/q or the stop button): tear down + end.
        if engine::CONTROLS.with(|c| std::mem::take(&mut c.borrow_mut().stop_requested)) {
            engine::STREAM.with(|s| *s.borrow_mut() = None);
            let _ = engine::with_audio(|pb| pb.flush());
            set_phase(Phase::Ended);
        }

        // The block_on audio open runs HERE (async-lifted export), not on-frame.
        if let Some((url, total)) = APP.with(|a| a.borrow_mut().pending_open.take()) {
            let surface = engine::CONTROLS.with(|c| c.borrow().surface);
            match engine::open_audio_sync(url, total, "FLAC test".to_string(), 0, surface) {
                Ok(()) => {
                    note("playing");
                    set_phase(Phase::Playing);
                }
                Err(e) => {
                    note(format!("open failed: {e}"));
                    set_phase(Phase::Ended);
                }
            }
        }

        // Drive the active stream off the RT path: seek (blocking I/O), demux, decode.
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
            Phase::Loading => 30,
            _ => 120,
        })
    }
}

/// Minimal status overlay (drawn until Playing; the engine draws the transport bar
/// while Playing). Just enough to see boot/loading/error state on screen.
fn render() {
    let cv: Canvas = engine::wctx(|x| x.get_current_buffer());
    let (sw, _sh) = engine::CONTROLS.with(|c| c.borrow().surface);
    let sw = sw as f32;
    let pad = 20.0;
    engine::draw_text(&cv, "wandr.flac.test — Demux::Audio call site", pad, 28.0, 22.0, 700, 0xFFFF_FFFF, sw - 2.0 * pad);
    let phase = APP.with(|a| format!("{:?}", a.borrow().phase));
    engine::draw_text(&cv, &phase, pad, 60.0, 16.0, 500, 0xFF8A_9098, sw - 2.0 * pad);
    engine::draw_text(&cv, DEFAULT_URL, pad, 86.0, 12.0, 400, 0xFF60_6870, sw - 2.0 * pad);
    let last = APP.with(|a| a.borrow().last.clone());
    if !last.is_empty() {
        engine::draw_text(&cv, &last, pad, 116.0, 13.0, 400, 0xFFB0_B4BC, sw - 2.0 * pad);
    }
    engine::wctx(|x| x.present());
}

export!(Component);
