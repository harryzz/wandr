//! wandr.audio.player — the player UI (task 108 M1).
//!
//! A wasi:canvas reactor: decodes an embedded FLAC with Symphonia (the
//! guest-decode floor), plays it through wasi:audio, and renders a waveform +
//! transport. The seekbar is driven by `wasi:audio playback.position` (the
//! clock promoted in M1). On the desktop host (no audio backend) a software
//! clock advances the seekbar so the UI is still exercisable; on device the
//! real `position` drives it.
//!
//!   cargo build --target wasm32-wasip2 --release

wit_bindgen::generate!({
    world: "wandr:audio-player/audio-player",
    path: "wit",
    generate_all,
});

use std::cell::RefCell;
use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;

use crate::exports::wasi::input_handlers::frame_handler::Guest as FrameGuest;
use crate::exports::wasi::input_handlers::pointer_handler::{
    Guest as PointerGuest, Kind as PKind, PointerEvent,
};
use crate::wasi::audio::pcm as wpcm;
use crate::wasi::canvas::embedding as wembed;
use crate::wasi::canvas::layout as wlayout;
use crate::wasi::canvas::types as wtypes;

static FLAC: &[u8] = include_bytes!("test.flac");

// ── Palette (ARGB) ────────────────────────────────────────────────────────
const BG: u32 = 0xFF14141E;
const ART_BG: u32 = 0xFF24243A;
const ACCENT: u32 = 0xFF4285F4;
const WAVE_DIM: u32 = 0xFF3A3A52;
const TEXT: u32 = 0xFFFFFFFF;
const SUBTEXT: u32 = 0xFFB0B0C8;
const ICON: u32 = 0xFFFFFFFF;

// ── Decoded track ─────────────────────────────────────────────────────────
const PEAKS: usize = 256;

struct Track {
    sample_rate: u32,
    channels: usize,
    samples: Vec<f32>, // interleaved f32 (the wasi:audio wire format)
    title: String,
    subtitle: String,
    peaks: [f32; PEAKS], // normalized 0..1 waveform overview
    total_frames: u64,
}

#[derive(Default)]
struct State {
    w: f32,
    h: f32,
    track: Option<Track>,
    pb: Option<wpcm::Playback>,
    playing: bool,
    cursor: usize,     // next interleaved-sample index to write
    anchor_dev: u64,   // pb.position() captured at the last anchor (open / seek)
    anchor_track: u64, // the track frame that anchor maps to
    sw_frames: u64,    // software clock (desktop / no-backend fallback)
    last_nanos: u64,
    ended: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static WCTX: RefCell<Option<wembed::CanvasContext>> = const { RefCell::new(None) };
}

fn wctx<R>(f: impl FnOnce(&wembed::CanvasContext) -> R) -> R {
    WCTX.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(wembed::get_context());
        }
        f(c.borrow().as_ref().unwrap())
    })
}

// ── Decode ─────────────────────────────────────────────────────────────────
fn decode() -> Track {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(FLAC.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("flac");
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .expect("probe failed");
    let mut format = probed.format;

    let track = format.default_track().expect("no track").clone();
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(48_000);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

    // Tags (Vorbis comments) — best-effort.
    let (mut title, mut artist, mut album) = (None, None, None);
    if let Some(rev) = format.metadata().current() {
        for tag in rev.tags() {
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) => title = Some(tag.value.to_string()),
                Some(StandardTagKey::Artist) => artist = Some(tag.value.to_string()),
                Some(StandardTagKey::Album) => album = Some(tag.value.to_string()),
                _ => {}
            }
        }
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("decoder failed");

    let mut samples: Vec<f32> = Vec::new();
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = *audio.spec();
                if sbuf.is_none() {
                    sbuf = Some(SampleBuffer::<f32>::new(audio.capacity() as u64, spec));
                }
                let b = sbuf.as_mut().unwrap();
                b.copy_interleaved_ref(audio);
                samples.extend_from_slice(b.samples());
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    let total_frames = (samples.len() / channels.max(1)) as u64;

    // Waveform overview: per-column max amplitude, normalized.
    let mut peaks = [0f32; PEAKS];
    let frames = total_frames as usize;
    if frames > 0 {
        for (i, peak) in peaks.iter_mut().enumerate() {
            let s = i * frames / PEAKS;
            let e = (((i + 1) * frames / PEAKS).max(s + 1)).min(frames);
            let mut mx = 0f32;
            for f in s..e {
                for c in 0..channels {
                    let v = samples[f * channels + c].abs();
                    if v > mx {
                        mx = v;
                    }
                }
            }
            *peak = mx;
        }
        let gmax = peaks.iter().copied().fold(0f32, f32::max).max(1e-6);
        for p in peaks.iter_mut() {
            *p = (*p / gmax).clamp(0.0, 1.0);
        }
    }

    let subtitle = match (artist, album) {
        (Some(a), Some(al)) => format!("{a} — {al}"),
        (Some(a), None) => a,
        (None, Some(al)) => al,
        (None, None) => format!(
            "FLAC · {} kHz · {}",
            sample_rate / 1000,
            if channels >= 2 { "stereo" } else { "mono" }
        ),
    };

    Track {
        sample_rate,
        channels,
        samples,
        title: title.unwrap_or_else(|| "Untitled".to_string()),
        subtitle,
        peaks,
        total_frames,
    }
}

fn stream_config(t: &Track) -> wpcm::StreamConfig {
    wpcm::StreamConfig {
        sample_rate: t.sample_rate,
        channel_layout: if t.channels >= 2 {
            wpcm::ChannelLayout::Stereo
        } else {
            wpcm::ChannelLayout::Mono
        },
        format: wpcm::Format::PcmF32,
        class: wpcm::StreamClass::Media,
    }
}

// ── Draw helpers ────────────────────────────────────────────────────────────
fn paint(color: u32) -> wtypes::Paint<'static> {
    wtypes::Paint {
        style: wtypes::PaintStyle::Fill,
        color,
        alpha: 255,
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

fn rect(x: f32, y: f32, w: f32, h: f32) -> wtypes::Rect {
    wtypes::Rect { x, y, width: w, height: h }
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> wtypes::RoundedRect {
    let c = wtypes::Point { x: r, y: r };
    wtypes::RoundedRect {
        rect: rect(x, y, w, h),
        top_left: c,
        top_right: c,
        bottom_right: c,
        bottom_left: c,
    }
}

struct Para {
    p: wlayout::Paragraph,
    width: f32,
    height: f32,
}

fn para(text: &str, size: f32, weight: u32, color: u32) -> Para {
    let style = wlayout::TextStyle {
        family: "sans-serif".into(),
        size,
        weight,
        italic: false,
        color,
        letter_spacing: 0.0,
        line_height: 0.0,
        baseline_shift: 0.0,
        decoration: None,
        shadows: Vec::new(),
        background: None,
    };
    let b = wlayout::ParagraphBuilder::new(&style);
    b.add_text(text);
    let p = wlayout::ParagraphBuilder::build(b);
    p.layout(1.0e6);
    let width = p.max_intrinsic_width();
    let height = p.height();
    Para { p, width, height }
}

/// Centered text; returns the box height.
fn text_centered(cv: &wembed::Canvas, text: &str, w: f32, top: f32, size: f32, weight: u32, color: u32) -> f32 {
    let pa = para(text, size, weight, color);
    pa.p.paint(cv, wtypes::Point { x: (w - pa.width) * 0.5, y: top });
    pa.height
}

fn fmt_time(frames: u64, sr: u32) -> String {
    let secs = frames / sr.max(1) as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

// ── Layout (derived from surface dims — no hardcoded geometry) ──────────────
struct Layout {
    art: wtypes::Rect,
    title_top: f32,
    sub_top: f32,
    wave: wtypes::Rect,
    time_top: f32,
    btn_cx: f32,
    btn_cy: f32,
    btn_r: f32,
}

fn layout(w: f32, h: f32) -> Layout {
    let m = w * 0.07;
    let art_side = (w.min(h) * 0.34).min(w - 2.0 * m);
    let art_x = (w - art_side) * 0.5;
    let art_y = h * 0.07;
    let title_top = art_y + art_side + h * 0.035;
    let sub_top = title_top + h * 0.05;
    let wave_h = h * 0.16;
    let wave_y = sub_top + h * 0.10;
    let time_top = wave_y + wave_h + h * 0.015;
    Layout {
        art: rect(art_x, art_y, art_side, art_side),
        title_top,
        sub_top,
        wave: rect(m, wave_y, w - 2.0 * m, wave_h),
        time_top,
        btn_cx: w * 0.5,
        btn_cy: h * 0.86,
        btn_r: w.min(h) * 0.085,
    }
}

// ── Audio ────────────────────────────────────────────────────────────────
/// Top the device ring up to full from the decode buffer (backpressure: stop
/// when the ring rejects more). Cheap per-frame; keeps the ring ahead of any
/// frame cadence.
fn pump(s: &mut State) {
    let (len, ch) = match &s.track {
        Some(t) => (t.samples.len(), t.channels),
        None => return,
    };
    if s.pb.is_none() {
        return;
    }
    let chunk = 4_800 * ch;
    loop {
        if s.cursor >= len {
            break;
        }
        let end = (s.cursor + chunk).min(len);
        let accepted = {
            let t = s.track.as_ref().unwrap();
            let pb = s.pb.as_ref().unwrap();
            pb.write(&t.samples[s.cursor..end]) as usize
        };
        s.cursor += accepted * ch;
        if accepted == 0 {
            break; // ring full
        }
    }
}

fn position_frames(s: &State) -> u64 {
    match &s.pb {
        // Device clock is monotonic frames-since-open; map it to the track via
        // the anchor captured at the last open/seek.
        Some(pb) => s.anchor_track + pb.position().saturating_sub(s.anchor_dev),
        None => s.sw_frames,
    }
}

/// Seek via `playback.flush` (no stream re-create): drop the buffered
/// (old-position) audio, re-anchor the device clock to the target, and
/// re-point the decode cursor. `flush` keeps `position` continuous, so the
/// anchor math stays exact.
fn seek(s: &mut State, target_frame: u64) {
    let ch = match &s.track {
        Some(t) => t.channels,
        None => return,
    };
    s.cursor = target_frame as usize * ch;
    s.sw_frames = target_frame;
    s.ended = false;
    if s.pb.is_some() {
        let was_playing = s.playing;
        // flush() pauses + drops the buffered (old-position) audio host-side.
        let pos = {
            let pb = s.pb.as_ref().unwrap();
            pb.flush();
            pb.position()
        };
        s.anchor_dev = pos;
        s.anchor_track = target_frame;
        // Prime the ring from the new position BEFORE resuming, so the device
        // doesn't restart into an empty ring (instant underrun).
        pump(s);
        if was_playing {
            if let Some(pb) = &s.pb {
                let _ = pb.start();
            }
        }
    }
}

fn toggle(s: &mut State) {
    if s.ended {
        // Track was closed at end — reset to the top; the open arm below
        // creates a fresh track (no seek-after-end target was set).
        s.ended = false;
        s.sw_frames = 0;
        s.cursor = 0;
    }
    if s.playing {
        if let Some(pb) = &s.pb {
            let _ = pb.pause();
        }
        s.playing = false;
    } else {
        match &s.pb {
            Some(pb) => {
                let _ = pb.start();
            }
            None => {
                if let Some(t) = &s.track {
                    if let Ok(pb) = wpcm::Playback::open(stream_config(t)) {
                        let _ = pb.start();
                        // Anchor the device clock to the current playhead
                        // (carries any seek made before the first play).
                        s.anchor_dev = pb.position();
                        s.anchor_track = s.sw_frames;
                        s.pb = Some(pb);
                    }
                }
            }
        }
        s.playing = true;
    }
}

// ── Reactor ──────────────────────────────────────────────────────────────
struct Player;

impl FrameGuest for Player {
    fn on_frame(nanos: u64) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            let cv = wctx(|x| x.get_current_buffer());
            if s.w == 0.0 {
                s.w = cv.width();
                s.h = cv.height();
            }
            if s.track.is_none() {
                s.track = Some(decode());
            }

            // Software clock (desktop / no backend): advance only while playing
            // and when no real stream is driving `position`.
            let dt = if s.last_nanos == 0 { 0 } else { nanos.saturating_sub(s.last_nanos) };
            s.last_nanos = nanos;
            if s.playing && s.pb.is_none() {
                let sr = s.track.as_ref().unwrap().sample_rate as u64;
                s.sw_frames = s.sw_frames.saturating_add(dt.saturating_mul(sr) / 1_000_000_000);
            }

            if s.playing {
                pump(&mut s);
            }

            // End detection.
            let total = s.track.as_ref().unwrap().total_frames;
            let pos = position_frames(&s).min(total);
            let drained = s.pb.as_ref().map(|p| p.buffered_frames() == 0).unwrap_or(true);
            let cursor_done = s.cursor >= s.track.as_ref().unwrap().samples.len();
            if s.playing && pos >= total.saturating_sub(1) && cursor_done && drained {
                // CLOSE the track at end. Leaving it started with an empty ring
                // underruns, and AudioFlinger removes a sustained-underrun track
                // ("BUFFER TIMEOUT: remove track ... due to underrun") — a
                // removed track can't be revived by flush/start. So drop it; a
                // fresh track is opened on replay/seek-after-end.
                s.pb = None; // drop = close
                s.sw_frames = total; // freeze the display at the end
                s.playing = false;
                s.ended = true;
            }

            let (w, h) = (s.w, s.h);
            let lay = layout(w, h);
            let frac = if total > 0 { (pos as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };

            // ── paint ──
            cv.draw_rect(rect(0.0, 0.0, w, h), &paint(BG));

            // Album-art placeholder: a "vinyl" (no embedded art in the test file).
            let a = lay.art;
            cv.draw_rounded_rect(rrect(a.x, a.y, a.width, a.height, a.width * 0.08), &paint(ART_BG));
            let cx = a.x + a.width * 0.5;
            let cy = a.y + a.height * 0.5;
            let disc = a.width * 0.34;
            cv.draw_oval(rect(cx - disc, cy - disc, disc * 2.0, disc * 2.0), &paint(ACCENT));
            let hole = a.width * 0.07;
            cv.draw_oval(rect(cx - hole, cy - hole, hole * 2.0, hole * 2.0), &paint(ART_BG));

            // Title + subtitle (from tags / format).
            let (title, subtitle, sr) = {
                let t = s.track.as_ref().unwrap();
                (t.title.clone(), t.subtitle.clone(), t.sample_rate)
            };
            text_centered(&cv, &title, w, lay.title_top, h * 0.038, 600, TEXT);
            text_centered(&cv, &subtitle, w, lay.sub_top, h * 0.024, 400, SUBTEXT);

            // Waveform overview (played = accent, rest = dim) + playhead.
            let wv = lay.wave;
            let peaks = s.track.as_ref().unwrap().peaks;
            let bar_w = wv.width / PEAKS as f32;
            let mid = wv.y + wv.height * 0.5;
            for (i, p) in peaks.iter().enumerate() {
                let bh = (p * wv.height).max(wv.height * 0.02);
                let bx = wv.x + i as f32 * bar_w;
                let col = if (i as f32 + 0.5) / PEAKS as f32 <= frac { ACCENT } else { WAVE_DIM };
                cv.draw_rect(rect(bx, mid - bh * 0.5, bar_w * 0.7, bh), &paint(col));
            }
            let ph = wv.x + frac * wv.width;
            cv.draw_rect(rect(ph - h * 0.002, wv.y, h * 0.004, wv.height), &paint(TEXT));

            // Times.
            let elapsed = para(&fmt_time(pos, sr), h * 0.022, 400, SUBTEXT);
            elapsed.p.paint(&cv, wtypes::Point { x: wv.x, y: lay.time_top });
            let totp = para(&fmt_time(total, sr), h * 0.022, 400, SUBTEXT);
            totp.p.paint(&cv, wtypes::Point { x: wv.x + wv.width - totp.width, y: lay.time_top });

            // Play / pause button.
            cv.draw_oval(
                rect(lay.btn_cx - lay.btn_r, lay.btn_cy - lay.btn_r, lay.btn_r * 2.0, lay.btn_r * 2.0),
                &paint(ACCENT),
            );
            let r = lay.btn_r;
            if s.playing {
                let bw = r * 0.26;
                let bh = r * 0.9;
                cv.draw_rect(rect(lay.btn_cx - r * 0.42, lay.btn_cy - bh * 0.5, bw, bh), &paint(ICON));
                cv.draw_rect(rect(lay.btn_cx + r * 0.16, lay.btn_cy - bh * 0.5, bw, bh), &paint(ICON));
            } else {
                let x0 = lay.btn_cx - r * 0.32;
                let x1 = lay.btn_cx + r * 0.46;
                let y0 = lay.btn_cy - r * 0.5;
                let y1 = lay.btn_cy + r * 0.5;
                let path = format!("M {x0} {y0} L {x1} {} L {x0} {y1} Z", lay.btn_cy);
                cv.draw_path(&path, wtypes::FillRule::Nonzero, &paint(ICON));
            }

            drop(cv);
            wctx(|x| x.present());
        });
    }

    fn on_resize(w: u32, h: u32) {
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            s.w = w as f32;
            s.h = h as f32;
        });
    }
}

impl PointerGuest for Player {
    fn on_pointer(ev: PointerEvent) {
        if !matches!(ev.kind, PKind::Down) {
            return;
        }
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            if s.track.is_none() || s.w == 0.0 {
                return;
            }
            let lay = layout(s.w, s.h);
            let (x, y) = (ev.x, ev.y);

            // Play/pause button (circular hit-test).
            let dx = x - lay.btn_cx;
            let dy = y - lay.btn_cy;
            if dx * dx + dy * dy <= lay.btn_r * lay.btn_r {
                toggle(&mut s);
                return;
            }

            // Waveform → seek (generous vertical hit band).
            let wv = lay.wave;
            let band = wv.height;
            if x >= wv.x && x <= wv.x + wv.width && y >= wv.y - band && y <= wv.y + wv.height + band {
                let frac = ((x - wv.x) / wv.width).clamp(0.0, 1.0);
                let total = s.track.as_ref().unwrap().total_frames;
                seek(&mut s, (frac as f64 * total as f64) as u64);
            }
        });
    }
}

export!(Player);
