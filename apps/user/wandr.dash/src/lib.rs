//! wandr.dash — task 119 Part B (B1). A standalone OPEN DASH/CMAF streaming
//! client. It reuses the EXACT playback engine `wandr.jellyfin` uses — extracted
//! into the `wandr-media-engine` crate — and adds only the DASH source layer:
//! manifest (.mpd) parse + adaptive CMAF/fMP4 segment fetch feeding the engine's
//! `Demux::Fmp4`. This is the second, open real-world consumer that justifies the
//! `wandr:video` playback contract (no server to run, no Google fortress).
//!
//! Same two-lane shape as jellyfin:
//!   * bg-tick (async): fetch the manifest, pick a video+audio Representation,
//!     byte-range-fetch init+media segments, feed the engine.
//!   * on-frame (sync): the engine pumps/presents video + writes audio; this app
//!     draws a small status/loading overlay (and, while Playing, the engine draws
//!     the transport bar).
//!
//! INCREMENT 3 (this file): scaffold — fetch the MPD over wasi:tls and report it
//! on screen, proving the new app builds + renders + fetches through the crate.
//! INCREMENT 4 wires dash-mpd parse + `Demux::Fmp4` segment feeding into playback.
#![allow(clippy::too_many_arguments)]

// EXPORTS-ONLY world (video/audio/canvas imports live in wandr-media-engine's own
// bindgen). Exactly one cabi_realloc (this export half). No generate_all.
wit_bindgen::generate!({
    world: "wandr:dash/dash-app",
    path: "wit",
});

// The shared playback engine: demux/audio/clock/present + the transport overlay,
// and the video/audio/canvas IMPORTS bindgen. This app depends ONLY on it (+ HTTP).
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

/// The default DASH manifest — Unified Streaming's "Tears of Steel" (clear/no-DRM,
/// CMAF/fMP4, multi-bitrate, separate H.264 video + AAC audio: exactly the
/// adaptive path). Verified live 2026-08-01 (see tasks/119-…-real-client-proof.md).
/// The SAME asset also serves HLS (`…/.m3u8`) for B3.
const DEFAULT_MPD: &str =
    "https://demo.unified-streaming.com/k8s/features/stable/video/tears-of-steel/tears-of-steel.ism/.mpd";

/// Where the client is in the fetch-manifest → resolve → play lifecycle. The
/// render loop reads this to decide what to draw; the async engine advances it.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Nothing started; the first bg-tick spawns the driver.
    Boot,
    /// Fetching + parsing the manifest / opening the first segments.
    Loading,
    /// Segments feeding the engine; on-frame pumps + presents.
    Playing,
    /// Stopped or failed — terminal; the last log line says why.
    Ended,
}

struct Engine {
    phase: Phase,
    /// On-screen status ring (also mirrored to stdout for headless checks).
    log: Vec<String>,
    /// Spawn the async driver exactly once.
    driver_spawned: bool,
    /// The manifest URL to play (default overridable later via /state).
    mpd_url: String,
}

impl Engine {
    const fn new() -> Self {
        Engine {
            phase: Phase::Boot,
            log: Vec::new(),
            driver_spawned: false,
            mpd_url: String::new(),
        }
    }
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::new());
}

/// Log to both the on-screen ring (last 12) and stdout (headless proof).
fn log(msg: impl Into<String>) {
    let msg = msg.into();
    println!("dash: {msg}");
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

fn build_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("wandr-dash/0.1 ( https://github.com/harryzz/wandr )")
        .build()
        .ok()
}

/// Download-then-play first cut: fetch a BOUNDED prefix of segments so playback
/// starts quickly and memory stays bounded (a full movie is streamed in a later
/// pass). ~40 × ~4 s ≈ 160 s of A/V — enough to prove sustained, synced playback.
const SEG_LIMIT: usize = 30;

/// A resolved Representation: the absolute init-segment URL + ordered media URLs.
struct RepPlan {
    init: String,
    segs: Vec<String>,
}

/// $Template$ substitution (DASH SegmentTemplate identifiers we use).
fn subst(tmpl: &str, rep_id: &str, bw: u64, time: Option<u64>) -> String {
    let mut s = tmpl
        .replace("$RepresentationID$", rep_id)
        .replace("$Bandwidth$", &bw.to_string());
    if let Some(t) = time {
        s = s.replace("$Time$", &t.to_string());
    }
    s
}

/// Enumerate the `$Time$` values from a SegmentTimeline (cumulative `t`, honoring
/// `d` and the `r` repeat count), capped at `limit`.
fn timeline_times(tl: &dash_mpd::SegmentTimeline, limit: usize) -> Vec<u64> {
    let mut out = Vec::new();
    let mut cur = 0u64;
    for s in &tl.segments {
        if let Some(t) = s.t {
            cur = t;
        }
        let reps = s.r.unwrap_or(0).max(0) as usize; // r = ADDITIONAL repeats
        for _ in 0..=reps {
            out.push(cur);
            cur += s.d;
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

/// Resolve one Representation's init + media segment URLs against `base`.
fn resolve_rep(
    base: &url::Url,
    tmpl: &dash_mpd::SegmentTemplate,
    rep_id: &str,
    bw: u64,
    limit: usize,
) -> Option<RepPlan> {
    let init_t = tmpl.initialization.as_ref()?;
    let init = base.join(&subst(init_t, rep_id, bw, None)).ok()?.to_string();
    let media_t = tmpl.media.as_ref()?;
    let times = tmpl
        .SegmentTimeline
        .as_ref()
        .map(|tl| timeline_times(tl, limit))
        .unwrap_or_default();
    let segs = times
        .iter()
        .filter_map(|&t| base.join(&subst(media_t, rep_id, bw, Some(t))).ok().map(|u| u.to_string()))
        .collect();
    Some(RepPlan { init, segs })
}

/// Pick an AdaptationSet of `kind` ("video"/"audio") and its LOWEST-bandwidth
/// Representation (small download for the first cut). Returns the SegmentTemplate
/// (adaptation-level, falling back to rep-level), the rep id, and its bandwidth.
fn pick<'a>(
    period: &'a dash_mpd::Period,
    kind: &str,
) -> Option<(&'a dash_mpd::SegmentTemplate, String, u64)> {
    let a = period.adaptations.iter().find(|a| {
        a.contentType.as_deref() == Some(kind)
            || a.mimeType.as_deref().map(|m| m.starts_with(kind)).unwrap_or(false)
    })?;
    let rep = a.representations.iter().min_by_key(|r| r.bandwidth.unwrap_or(u64::MAX))?;
    let tmpl = a.SegmentTemplate.as_ref().or(rep.SegmentTemplate.as_ref())?;
    Some((tmpl, rep.id.clone().unwrap_or_default(), rep.bandwidth.unwrap_or(0)))
}

async fn fetch_bytes(client: &reqwest::Client, url_str: &str) -> Option<Vec<u8>> {
    let u = url::Url::parse(url_str).ok()?;
    let resp = client.get(u).send().await.ok()?;
    Some(resp.bytes().await.ok()?.to_vec())
}

/// Download init + all planned media segments, concatenated into one fragmented-MP4
/// byte stream (the shape `oxideav-mp4` demuxes). Stops on the first fetch error.
async fn download(client: &reqwest::Client, plan: &RepPlan, what: &str) -> Option<Vec<u8>> {
    let mut buf = fetch_bytes(client, &plan.init).await?;
    for (i, s) in plan.segs.iter().enumerate() {
        match fetch_bytes(client, s).await {
            Some(b) => buf.extend_from_slice(&b),
            None => {
                log(format!("{what}: seg {i} fetch failed — stopping at {} segs", i));
                break;
            }
        }
        if i % 10 == 9 {
            log(format!("{what}: {}/{} segs, {} KB", i + 1, plan.segs.len(), buf.len() / 1024));
        }
    }
    Some(buf)
}

/// bg-tick driver (spawned once): fetch + parse the MPD, resolve a low video +
/// audio Representation, download a bounded prefix of segments, and hand the two
/// concatenated fragmented-MP4 buffers to the engine (`Demux::Fmp4`).
async fn driver() {
    let url = ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        if e.mpd_url.is_empty() {
            e.mpd_url = DEFAULT_MPD.to_string();
        }
        e.mpd_url.clone()
    });
    set_phase(Phase::Loading);
    log("fetching manifest…");

    let Some(client) = build_client() else {
        log("FAILED: build client");
        return set_phase(Phase::Ended);
    };
    let Ok(mpd_url) = url::Url::parse(&url) else {
        log("FAILED: bad mpd url");
        return set_phase(Phase::Ended);
    };
    let Some(text) = fetch_bytes(&client, mpd_url.as_str()).await else {
        log("FAILED: mpd fetch");
        return set_phase(Phase::Ended);
    };
    let text = String::from_utf8_lossy(&text).into_owned();
    let mpd = match dash_mpd::parse(&text) {
        Ok(m) => m,
        Err(e) => {
            log(format!("FAILED: mpd parse: {e}"));
            return set_phase(Phase::Ended);
        }
    };
    let Some(period) = mpd.periods.first() else {
        log("FAILED: no period");
        return set_phase(Phase::Ended);
    };

    // BaseURL resolution: MPD-level then Period-level, relative to the manifest URL.
    let mut base = mpd_url.clone();
    for b in &mpd.base_url {
        if let Ok(j) = base.join(&b.base) { base = j; }
    }
    for b in &period.BaseURL {
        if let Ok(j) = base.join(&b.base) { base = j; }
    }

    // Video is required; audio is best-effort.
    let Some((v_tmpl, v_id, v_bw)) = pick(period, "video") else {
        log("FAILED: no video adaptation");
        return set_phase(Phase::Ended);
    };
    let Some(vplan) = resolve_rep(&base, v_tmpl, &v_id, v_bw, SEG_LIMIT) else {
        log("FAILED: video SegmentTemplate/Timeline missing");
        return set_phase(Phase::Ended);
    };
    let aplan = pick(period, "audio")
        .and_then(|(t, id, bw)| resolve_rep(&base, t, &id, bw, SEG_LIMIT));

    log(format!("video rep {v_id} ({} segs); audio {}", vplan.segs.len(),
        aplan.as_ref().map(|p| format!("{} segs", p.segs.len())).unwrap_or_else(|| "none".into())));

    let Some(vbuf) = download(&client, &vplan, "video").await else {
        log("FAILED: video download");
        return set_phase(Phase::Ended);
    };
    let abuf = match &aplan {
        Some(ap) => download(&client, ap, "audio").await,
        None => None,
    };

    let dur_us = mpd.mediaPresentationDuration.map(|d| d.as_micros() as i64).unwrap_or(0);
    let surface = engine::CONTROLS.with(|c| c.borrow().surface);
    log(format!("opening: video {} KB, audio {} KB", vbuf.len() / 1024, abuf.as_ref().map(|b| b.len() / 1024).unwrap_or(0)));
    match engine::open_fmp4(vbuf, abuf, "Tears of Steel".to_string(), dur_us, surface) {
        Ok(()) => {
            // Reset controls for a fresh stream (mirrors jellyfin's on_stream_started).
            engine::CONTROLS.with(|c| {
                let mut c = c.borrow_mut();
                c.paused = false;
                c.seek_request = None;
            });
            set_phase(Phase::Playing);
        }
        Err(e) => {
            log(format!("FAILED: open_fmp4: {e}"));
            set_phase(Phase::Ended);
        }
    }
}

// ---- render (status / loading overlay) -------------------------------------

fn render() {
    let cv: Canvas = engine::wctx(|x| x.get_current_buffer());
    cv.clear(0xFF0B_0D10); // near-black background

    let (sw, sh) = engine::CONTROLS.with(|c| c.borrow().surface);
    let (sw, sh) = (sw as f32, sh as f32);
    let pad = 20.0;

    engine::draw_text(&cv, "wandr.dash — open DASH/CMAF client", pad, 24.0, 22.0, 700, 0xFFFF_FFFF, sw - 2.0 * pad);
    let (phase, url) = ENGINE.with(|e| { let e = e.borrow(); (e.phase.clone(), e.mpd_url.clone()) });
    let sub = match phase {
        Phase::Boot => "starting…".to_string(),
        Phase::Loading => "loading manifest…".to_string(),
        Phase::Playing => "playing".to_string(),
        Phase::Ended => "ended".to_string(),
    };
    engine::draw_text(&cv, &sub, pad, 56.0, 16.0, 500, 0xFF8A_9098, sw - 2.0 * pad);
    if !url.is_empty() {
        engine::draw_text(&cv, &url, pad, 80.0, 12.0, 400, 0xFF60_6870, sw - 2.0 * pad);
    }

    // Status ring.
    let lines = ENGINE.with(|e| e.borrow().log.clone());
    let mut y = 116.0;
    for ln in lines.iter() {
        engine::draw_text(&cv, ln, pad, y, 13.0, 400, 0xFFB0_B4BC, sw - 2.0 * pad);
        y += 20.0;
        if y > sh - 24.0 {
            break;
        }
    }

    drop(cv);
    engine::wctx(|x| x.present());
}

// ---- exports ---------------------------------------------------------------

struct Component;

impl FrameGuest for Component {
    fn on_frame(nanos: u64) {
        // on-frame is a SYNCHRONOUS CM export (no block_on): the fetch/demux run in
        // the async bg-tick; on-frame only submits/decodes/presents from the queues.
        let playing = ENGINE.with(|e| e.borrow().phase == Phase::Playing);
        if playing {
            engine::pump_stream(nanos);
            engine::render_playing(nanos);
        } else {
            render();
        }
    }
    fn on_resize(w: u32, h: u32) {
        // The engine owns the surface size + decoder-rect reconcile.
        engine::set_surface(w, h);
    }
}

impl KeyGuest for Component {
    fn on_key(ev: KeyEvent) {
        if !ev.down {
            return;
        }
        let playing = ENGINE.with(|e| e.borrow().phase == Phase::Playing);
        if !playing {
            return;
        }
        // Transport (same bindings as jellyfin): Esc/Backspace/q stop; Space/k
        // pause; ↑/↓ volume; m mute; ←/→ (j/l) seek ∓10 s; Home restart. All
        // intents land on the engine's CONTROLS, which the pump reads.
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
                c.seek_request = Some(engine::seek_from_clock(10_000_000));
            } else if ev.code == "ArrowLeft" || ev.text.eq_ignore_ascii_case("j") {
                c.seek_request = Some(engine::seek_from_clock(-10_000_000));
            } else if ev.code == "Home" {
                c.seek_request = Some(0);
            } else if ev.code == "KeyA" || ev.text.eq_ignore_ascii_case("a") {
                if n_audio > 1 {
                    c.audio_pref = (c.audio_pref + 1) % n_audio;
                    c.audio_switch = true;
                }
            }
        });
    }
}

impl PointerGuest for Component {
    fn on_pointer(ev: PointerEvent) {
        let playing = ENGINE.with(|e| e.borrow().phase == Phase::Playing);
        if !playing {
            return;
        }
        // Transport bar (geometry from the engine): any pointer reveals it;
        // primary-press hit-tests the buttons + volume slider; the scrub track is a
        // DRAG — down starts, move previews, up commits the seek.
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
        ENGINE.with(|e| match e.borrow().phase {
            Phase::Playing => 16,  // frame-rate cadence: pump pulls + presents
            Phase::Loading => 100,
            _ => 200,
        })
    }
}

impl BgGuest for Component {
    async fn bg_tick() -> u32 {
        // Spawn the driver exactly once.
        let spawn = ENGINE.with(|e| {
            let mut e = e.borrow_mut();
            if !e.driver_spawned { e.driver_spawned = true; true } else { false }
        });
        if spawn {
            reqwest::task::spawn(driver());
        }

        // Stop requested (Esc/q or the stop button): tear the stream down and end.
        if engine::CONTROLS.with(|c| std::mem::take(&mut c.borrow_mut().stop_requested)) {
            engine::STREAM.with(|s| *s.borrow_mut() = None);
            let _ = engine::with_audio(|pb| pb.flush());
            set_phase(Phase::Ended);
        }

        // While a stream is active, drive the engine off the RT path: seek (blocking
        // I/O), demux fill, audio decode, prefetch. (Dormant until INCREMENT 4
        // installs a Demux::Fmp4 stream; the plumbing is identical to jellyfin's.)
        let active = engine::STREAM.with(|s| s.borrow().is_some());
        if active {
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

        // bg-tick cadence (ms): brisk while loading/playing, lazy otherwise.
        ENGINE.with(|e| match e.borrow().phase {
            Phase::Playing => 8,
            Phase::Loading => 30,
            _ => 120,
        })
    }
}

export!(Component);
