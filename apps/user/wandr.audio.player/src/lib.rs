//! wandr.audio.player — Slint UI, streaming multi-track player (task 108 M3).
//!
//! Scans `/music` (host-preopened, read-only) for albums → tracks, decodes the
//! current track INCREMENTALLY (Symphonia, low memory) in the
//! `wandr:background/background` bg-tick, resampling to the 48 kHz backend with a
//! guest-side linear resampler, and writes PCM to `wasi:audio`. The engine +
//! now-playing publishing run in bg-tick (every role) so playback + lockscreen
//! control survive backgrounding. UI is Slint via `crates/slint-wandr`: a Slint
//! ListView playlist + progress bar with a draggable thumb; cover art is the
//! album's `albumart.{jpg,png}` decoded guest-side → Slint Image.
//!
//!   cargo build --target wasm32-wasip2 --release
//!   cp target/wasm32-wasip2/release/wandr_audio_player.wasm components/ui.wasm

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::bindings::wasi::audio::pcm as wpcm;
use crate::bindings::wasi::media_session::session as wsession;

const OUT_RATE: u32 = 48_000;
const MUSIC_DIR: &str = "/music";

slint::slint! {
    import { ListView } from "std-widgets.slint";

    struct TrackRow { title: string, album: string, current: bool }

    export component MainWindow inherits Window {
        background: #141422;
        in property <string> song-title: "—";
        in property <string> subtitle: "";
        in property <string> elapsed: "0:00";
        in property <string> right-time: "0:00";
        in property <float> progress: 0.0;
        in property <bool> playing: false;
        in property <image> cover;
        in property <bool> has-cover: false;
        in property <bool> shuffle: false;
        in property <bool> repeat: false;
        in property <[TrackRow]> tracks: [];
        callback toggle();
        callback prev-track();
        callback next-track();
        callback seek(float);
        callback toggle-shuffle();
        callback toggle-repeat();
        callback toggle-time();
        callback select(int);

        property <length> art-size: min(root.width, root.height) * 0.30;
        property <length> btn: min(root.width, root.height) * 0.13;
        property <color> on-col: #4285f4;
        property <color> off-col: #8a8aa0;

        VerticalLayout {
            padding: root.width * 0.06;
            spacing: root.height * 0.012;

            HorizontalLayout {
                alignment: center;
                Rectangle {
                    width: art-size; height: art-size;
                    border-radius: art-size * 0.08;
                    clip: true;
                    background: #24243a;
                    if root.has-cover : Image {
                        width: 100%; height: 100%;
                        source: root.cover;
                        image-fit: ImageFit.cover;
                    }
                    if !root.has-cover : Rectangle {
                        width: art-size * 0.66; height: art-size * 0.66;
                        border-radius: self.width / 2;
                        background: #4285f4;
                        Rectangle {
                            width: art-size * 0.14; height: art-size * 0.14;
                            border-radius: self.width / 2;
                            background: #24243a;
                        }
                    }
                }
            }

            Text {
                text: root.song-title; color: white; font-size: 19px; font-weight: 700;
                horizontal-alignment: center; overflow: elide;
            }
            Text {
                text: root.subtitle; color: #b0b0c8; font-size: 13px;
                horizontal-alignment: center; overflow: elide;
            }

            // Progress bar with a draggable seek thumb.
            prog := Rectangle {
                height: 16px;
                property <bool> dragging: false;
                property <float> drag-frac: 0.0;
                property <float> shown: dragging ? drag-frac : root.progress;
                Rectangle {
                    width: 100%; height: 4px; y: (parent.height - self.height) / 2;
                    border-radius: 2px; background: #3a3a52;
                }
                Rectangle {
                    width: parent.width * prog.shown; height: 4px;
                    y: (parent.height - self.height) / 2;
                    border-radius: 2px; background: #4285f4;
                }
                Rectangle {
                    width: 14px; height: 14px; border-radius: 7px; background: white;
                    x: prog.shown * (parent.width - self.width);
                    y: (parent.height - self.height) / 2;
                }
                TouchArea {
                    moved => {
                        prog.drag-frac = clamp(self.mouse-x / self.width, 0.0, 1.0);
                        prog.dragging = true;
                    }
                    pointer-event(ev) => {
                        if (ev.kind == PointerEventKind.down) {
                            prog.drag-frac = clamp(self.mouse-x / self.width, 0.0, 1.0);
                            prog.dragging = true;
                        }
                        if (ev.kind == PointerEventKind.up) {
                            root.seek(prog.drag-frac);
                            prog.dragging = false;
                        }
                    }
                }
            }

            HorizontalLayout {
                Text { text: root.elapsed; color: #b0b0c8; font-size: 12px; }
                Rectangle { }
                TouchArea {
                    width: rt.preferred-width; height: rt.preferred-height;
                    clicked => { root.toggle-time(); }
                    rt := Text { text: root.right-time; color: #b0b0c8; font-size: 12px; }
                }
            }

            // Transport: shuffle · prev · play/pause · next · repeat.
            HorizontalLayout {
                alignment: center;
                spacing: root.width * 0.045;

                Rectangle {
                    width: btn * 0.62; height: btn * 0.62;
                    Path {
                        width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                        commands: "M 14 34 L 42 34 L 86 66 M 74 58 L 88 66 L 76 73 M 14 66 L 42 66 L 86 34 M 76 27 L 88 34 L 74 42";
                        stroke: root.shuffle ? on-col : off-col; stroke-width: 7px; fill: transparent;
                    }
                    TouchArea { clicked => { root.toggle-shuffle(); } }
                }
                Rectangle {
                    width: btn * 0.8; height: btn * 0.8;
                    Path {
                        width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                        commands: "M 64 28 L 40 50 L 64 72 Z M 32 28 L 38 28 L 38 72 L 32 72 Z";
                        fill: white;
                    }
                    TouchArea { clicked => { root.prev-track(); } }
                }
                Rectangle {
                    width: btn; height: btn; border-radius: btn / 2; background: #4285f4;
                    if !root.playing : Path {
                        width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                        commands: "M 36 24 L 76 50 L 36 76 Z";
                        fill: white;
                    }
                    if root.playing : Path {
                        width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                        commands: "M 34 26 L 45 26 L 45 74 L 34 74 Z M 55 26 L 66 26 L 66 74 L 55 74 Z";
                        fill: white;
                    }
                    TouchArea { clicked => { root.toggle(); } }
                }
                Rectangle {
                    width: btn * 0.8; height: btn * 0.8;
                    Path {
                        width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                        commands: "M 36 28 L 60 50 L 36 72 Z M 62 28 L 68 28 L 68 72 L 62 72 Z";
                        fill: white;
                    }
                    TouchArea { clicked => { root.next-track(); } }
                }
                Rectangle {
                    width: btn * 0.62; height: btn * 0.62;
                    Path {
                        width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                        commands: "M 34 38 L 60 38 A 14 14 0 0 1 74 52 L 74 58 M 66 53 L 74 62 L 82 53 M 66 62 L 40 62 A 14 14 0 0 1 26 48 L 26 42 M 18 47 L 26 38 L 34 47";
                        stroke: root.repeat ? on-col : off-col; stroke-width: 7px; fill: transparent;
                    }
                    TouchArea { clicked => { root.toggle-repeat(); } }
                }
            }

            // Playlist.
            ListView {
                vertical-stretch: 1;
                for t[i] in root.tracks : Rectangle {
                    height: 46px;
                    background: t.current ? #20203a : transparent;
                    HorizontalLayout {
                        padding-left: 10px; padding-right: 10px;
                        VerticalLayout {
                            alignment: center;
                            Text {
                                text: t.title; font-size: 14px; overflow: elide;
                                color: t.current ? #4285f4 : white;
                            }
                            Text { text: t.album; font-size: 11px; color: #8a8aa0; overflow: elide; }
                        }
                    }
                    TouchArea { clicked => { root.select(i); } }
                }
            }
        }
    }
}

// ── Library + streaming track ────────────────────────────────────────────────
#[derive(Clone)]
struct LibTrack {
    path: String,
    title: String, // derived from filename (cheap)
    album: String,
    art_path: Option<String>,
}

struct Loaded {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    channels: usize,
    resampler: Option<LinearResampler>,
    pending: Vec<f32>, // resampled 48k interleaved, not yet written
    pending_pos: usize,
    eof: bool,
    total_frames: u64, // at 48k (0 = unknown)
    title: String,
    subtitle: String,
    art: Option<(Vec<u8>, u32, u32)>,
}

#[derive(Default)]
struct State {
    library: Vec<LibTrack>,
    order: Vec<usize>, // play order (shuffle-aware) → library index
    order_pos: usize,
    scanned: bool,
    loaded: Option<Loaded>,
    pb: Option<wpcm::Playback>,
    playing: bool,
    anchor_dev: u64,
    anchor_track: u64,
    sw_frames: u64,
    ended: bool,
    published: bool,
    pub_playing: bool,
    last_pub_sec: i64,
    shuffle: bool,
    repeat: bool,
    show_remaining: bool,
    // UI dirty flags
    list_dirty: bool,
    meta_dirty: bool,
    // album-art cache (decode once per album)
    art_cache: Option<(String, Vec<u8>, u32, u32)>,
    rng: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static UI: RefCell<Option<MainWindow>> = const { RefCell::new(None) };
}

// ── Linear streaming resampler (src → 48k) ───────────────────────────────────
struct LinearResampler {
    step: f64, // source frames advanced per output frame
    ch: usize,
    pos: f64,
    last: Vec<f32>,
}
impl LinearResampler {
    fn new(src: u32, ch: usize) -> Self {
        Self { step: src as f64 / OUT_RATE as f64, ch, pos: 0.0, last: vec![0.0; ch] }
    }
    fn reset(&mut self) {
        self.pos = 0.0;
        for x in &mut self.last {
            *x = 0.0;
        }
    }
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let ch = self.ch;
        let n = input.len() / ch;
        if n == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(((n as f64 / self.step) as usize + 2) * ch);
        while self.pos < n as f64 {
            let i = self.pos.floor() as usize; // 0..n-1
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

// ── Library scan ─────────────────────────────────────────────────────────────
fn pretty_title(fname: &str) -> String {
    let stem = fname.rsplit_once('.').map(|(a, _)| a).unwrap_or(fname);
    let trimmed = stem
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(['_', ' ', '-', '.']);
    let s = trimmed.replace('_', " ");
    if s.trim().is_empty() { stem.to_string() } else { s }
}

fn is_audio(p: &std::path::Path) -> bool {
    p.extension()
        .map(|x| {
            let x = x.to_string_lossy().to_lowercase();
            x == "mp3" || x == "flac" || x == "wav" || x == "ogg"
        })
        .unwrap_or(false)
}

fn scan_library() -> Vec<LibTrack> {
    let mut out = Vec::new();
    let Ok(albums) = std::fs::read_dir(MUSIC_DIR) else {
        return out;
    };
    let mut album_dirs: Vec<std::path::PathBuf> = albums
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| !p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(true))
        .collect();
    album_dirs.sort();
    for apath in album_dirs {
        let album = apath.file_name().unwrap().to_string_lossy().to_string();
        let art_path = ["albumart.jpg", "albumart.png", "cover.jpg", "cover.png", "folder.jpg"]
            .iter()
            .map(|f| apath.join(f))
            .find(|p| p.exists())
            .map(|p| p.to_string_lossy().to_string());
        let Ok(files) = std::fs::read_dir(&apath) else {
            continue;
        };
        let mut tracks: Vec<std::path::PathBuf> =
            files.flatten().map(|e| e.path()).filter(|p| is_audio(p)).collect();
        tracks.sort();
        for p in tracks {
            let fname = p.file_name().unwrap().to_string_lossy().to_string();
            out.push(LibTrack {
                path: p.to_string_lossy().to_string(),
                title: pretty_title(&fname),
                album: album.clone(),
                art_path: art_path.clone(),
            });
        }
    }
    out
}

fn ext_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, e)| e).unwrap_or("")
}

fn load_art(s: &mut State, art_path: &Option<String>) -> Option<(Vec<u8>, u32, u32)> {
    let path = art_path.as_ref()?;
    if let Some((cached_path, rgba, w, h)) = &s.art_cache {
        if cached_path == path {
            return Some((rgba.clone(), *w, *h));
        }
    }
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let raw = rgba.into_raw();
    s.art_cache = Some((path.clone(), raw.clone(), w, h));
    Some((raw, w, h))
}

// ── Load a library track for streaming (no full decode) ──────────────────────
fn load(s: &mut State, lib_index: usize) {
    s.pb = None; // close any open track (channels may differ); play() reopens
    s.playing = false;
    let Some(entry) = s.library.get(lib_index).cloned() else {
        return;
    };
    let file = match std::fs::File::open(&entry.path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext_of(&entry.path));
    let probed = match symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions { enable_gapless: true, ..Default::default() },
        &MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(_) => return,
    };
    let mut format = probed.format;
    let track = match format.default_track() {
        Some(t) => t.clone(),
        None => return,
    };
    let track_id = track.id;
    let src_rate = track.codec_params.sample_rate.unwrap_or(OUT_RATE);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);
    let n_frames = track.codec_params.n_frames;
    let decoder = match symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
    {
        Ok(d) => d,
        Err(_) => return,
    };

    // Tags (real title/artist/album override the filename-derived title).
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
    let title = title.unwrap_or_else(|| entry.title.clone());
    let subtitle = match (artist, album) {
        (Some(a), Some(al)) => format!("{a} — {al}"),
        (Some(a), None) => a,
        (None, _) => entry.album.clone(),
    };

    let total_frames = n_frames
        .map(|nf| (nf as u128 * OUT_RATE as u128 / src_rate.max(1) as u128) as u64)
        .unwrap_or(0);
    let art = load_art(s, &entry.art_path);
    let resampler = if src_rate != OUT_RATE { Some(LinearResampler::new(src_rate, channels)) } else { None };

    s.loaded = Some(Loaded {
        format,
        decoder,
        track_id,
        channels,
        resampler,
        pending: Vec::new(),
        pending_pos: 0,
        eof: false,
        total_frames,
        title,
        subtitle,
        art,
    });
    s.anchor_dev = 0;
    s.anchor_track = 0;
    s.sw_frames = 0;
    s.ended = false;
    s.list_dirty = true;
    s.meta_dirty = true;
    publish_metadata(s);
    publish_state(s);
    publish_position(s);
    s.published = true;
    s.pub_playing = s.playing;
    s.last_pub_sec = -1;
}

// ── Streaming decode + ring feed ─────────────────────────────────────────────
fn decode_more(s: &mut State) -> bool {
    let Some(l) = s.loaded.as_mut() else {
        return false;
    };
    loop {
        let packet = match l.format.next_packet() {
            Ok(p) => p,
            Err(_) => {
                l.eof = true;
                return false;
            }
        };
        if packet.track_id() != l.track_id {
            continue;
        }
        match l.decoder.decode(&packet) {
            Ok(audio) => {
                let spec = *audio.spec();
                let mut sb = SampleBuffer::<f32>::new(audio.capacity() as u64, spec);
                sb.copy_interleaved_ref(audio);
                let out = match l.resampler.as_mut() {
                    Some(r) => r.process(sb.samples()),
                    None => sb.samples().to_vec(),
                };
                l.pending = out;
                l.pending_pos = 0;
                return true;
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => {
                l.eof = true;
                return false;
            }
        }
    }
}

fn pump(s: &mut State) {
    if s.pb.is_none() || s.loaded.is_none() {
        return;
    }
    loop {
        let has_pending = {
            let l = s.loaded.as_ref().unwrap();
            l.pending_pos < l.pending.len()
        };
        if has_pending {
            let ch = s.loaded.as_ref().unwrap().channels.max(1);
            let accepted = {
                let l = s.loaded.as_ref().unwrap();
                let pb = s.pb.as_ref().unwrap();
                pb.write(&l.pending[l.pending_pos..]) as usize
            };
            let l = s.loaded.as_mut().unwrap();
            l.pending_pos += accepted * ch;
            if l.pending_pos >= l.pending.len() {
                l.pending.clear();
                l.pending_pos = 0;
            }
            if accepted == 0 {
                break; // ring full
            }
        } else {
            if s.loaded.as_ref().unwrap().eof {
                break;
            }
            if !decode_more(s) {
                break;
            }
        }
    }
}

fn position_frames(s: &State) -> u64 {
    match &s.pb {
        Some(pb) => s.anchor_track + pb.position().saturating_sub(s.anchor_dev),
        None => s.sw_frames,
    }
}

fn play(s: &mut State) {
    if s.loaded.is_none() {
        return;
    }
    if s.ended {
        s.ended = false;
    }
    match &s.pb {
        Some(pb) => {
            let _ = pb.start();
        }
        None => {
            let ch = s.loaded.as_ref().unwrap().channels;
            let cfg = wpcm::StreamConfig {
                sample_rate: OUT_RATE,
                channel_layout: if ch >= 2 { wpcm::ChannelLayout::Stereo } else { wpcm::ChannelLayout::Mono },
                format: wpcm::Format::PcmF32,
                class: wpcm::StreamClass::Media,
            };
            if let Ok(pb) = wpcm::Playback::open(cfg) {
                let _ = pb.start();
                s.anchor_dev = pb.position();
                s.anchor_track = s.sw_frames;
                s.pb = Some(pb);
            }
        }
    }
    s.playing = true;
}

fn pause(s: &mut State) {
    if let Some(pb) = &s.pb {
        let _ = pb.pause();
    }
    s.playing = false;
}

fn seek_to(s: &mut State, target_48k: u64) {
    let target = {
        let Some(l) = s.loaded.as_mut() else {
            return;
        };
        let target = if l.total_frames > 0 { target_48k.min(l.total_frames) } else { target_48k };
        let secs = target as f64 / OUT_RATE as f64;
        let _ = l.format.seek(
            SeekMode::Accurate,
            SeekTo::Time { time: Time::new(secs.trunc() as u64, secs.fract()), track_id: Some(l.track_id) },
        );
        l.decoder.reset();
        l.pending.clear();
        l.pending_pos = 0;
        l.eof = false;
        if let Some(r) = l.resampler.as_mut() {
            r.reset();
        }
        target
    };
    s.sw_frames = target;
    s.ended = false;
    let was = s.playing;
    if let Some(pb) = s.pb.as_ref() {
        pb.flush();
        s.anchor_dev = pb.position();
        s.anchor_track = target;
    }
    pump(s);
    if was {
        if let Some(pb) = s.pb.as_ref() {
            let _ = pb.start();
        }
    }
}

fn cur_lib_index(s: &State) -> Option<usize> {
    s.order.get(s.order_pos).copied()
}

fn go_to(s: &mut State, order_pos: usize, autoplay: bool) {
    if order_pos >= s.order.len() {
        return;
    }
    s.order_pos = order_pos;
    let lib = s.order[order_pos];
    load(s, lib);
    if autoplay {
        play(s);
    }
    after_change_publish(s);
}

fn on_track_end(s: &mut State) {
    if s.repeat && s.order_pos + 1 >= s.order.len() {
        // repeat the queue: wrap to the start.
        go_to(s, 0, true);
    } else if s.order_pos + 1 < s.order.len() {
        go_to(s, s.order_pos + 1, true);
    } else {
        // end of queue, no repeat: stop at the end.
        s.pb = None;
        s.playing = false;
        s.ended = true;
        if let Some(l) = &s.loaded {
            s.sw_frames = l.total_frames;
        }
    }
}

fn rebuild_order(s: &mut State, keep_lib: Option<usize>) {
    let n = s.library.len();
    if s.shuffle {
        let mut v: Vec<usize> = (0..n).collect();
        let mut i = n;
        while i > 1 {
            i -= 1;
            s.rng = s.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (s.rng >> 33) as usize % (i + 1);
            v.swap(i, j);
        }
        s.order = v;
    } else {
        s.order = (0..n).collect();
    }
    if let Some(lib) = keep_lib {
        s.order_pos = s.order.iter().position(|&k| k == lib).unwrap_or(0);
    }
    s.list_dirty = true;
}

// ── media-session publishing ────────────────────────────────────────────────
fn publish_metadata(s: &State) {
    let Some(l) = &s.loaded else { return };
    let (artist, album) = l.subtitle.split_once(" — ").unwrap_or((l.subtitle.as_str(), ""));
    wsession::set_metadata(&wsession::Metadata {
        title: l.title.clone(),
        artist: artist.to_string(),
        album: album.to_string(),
        artwork: None,
    });
}
fn publish_state(s: &State) {
    let st = if s.playing {
        wsession::PlaybackState::Playing
    } else if s.ended {
        wsession::PlaybackState::None
    } else {
        wsession::PlaybackState::Paused
    };
    wsession::set_playback_state(st);
}
fn publish_position(s: &State) {
    let Some(l) = &s.loaded else { return };
    let total = l.total_frames;
    let pos = position_frames(s).min(if total > 0 { total } else { u64::MAX });
    wsession::set_position(wsession::PositionState {
        duration_s: total as f64 / OUT_RATE as f64,
        playback_rate: if s.playing { 1.0 } else { 0.0 },
        position_s: pos as f64 / OUT_RATE as f64,
    });
}
fn after_change_publish(s: &mut State) {
    publish_state(s);
    s.pub_playing = s.playing;
    publish_position(s);
    s.last_pub_sec = -1;
}

// ── Engine step (bg-tick, every role) ────────────────────────────────────────
fn engine_step(s: &mut State) -> u32 {
    if !s.scanned {
        s.scanned = true;
        s.rng = 0x9E3779B97F4A7C15;
        s.library = scan_library();
        s.order = (0..s.library.len()).collect();
        s.list_dirty = true;
        if !s.library.is_empty() {
            go_to(s, 0, false); // load first track, paused
        }
    }
    if s.loaded.is_none() {
        return 1000;
    }

    if s.playing {
        pump(s);
    }

    let (eof, total) = {
        let l = s.loaded.as_ref().unwrap();
        (l.eof && l.pending_pos >= l.pending.len(), l.total_frames)
    };
    let ring_empty = s.pb.as_ref().map(|p| p.buffered_frames() == 0).unwrap_or(true);
    if s.playing && eof && ring_empty {
        on_track_end(s);
    }

    if s.pub_playing != s.playing {
        publish_state(s);
        s.pub_playing = s.playing;
    }
    let pos = position_frames(s).min(if total > 0 { total } else { u64::MAX });
    let cur_sec = (pos / OUT_RATE as u64) as i64;
    if cur_sec != s.last_pub_sec {
        publish_position(s);
        s.last_pub_sec = cur_sec;
    }

    if s.playing { 33 } else { 500 }
}

// ── UI bridge ───────────────────────────────────────────────────────────────
fn fmt_time(frames: u64) -> String {
    let secs = frames / OUT_RATE as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn push_ui() {
    UI.with(|u| {
        let b = u.borrow();
        let Some(ui) = b.as_ref() else { return };
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            let (title, subtitle, total) = match &s.loaded {
                Some(l) => (l.title.clone(), l.subtitle.clone(), l.total_frames),
                None => ("No music in /music".to_string(), String::new(), 0),
            };
            let pos = position_frames(&s).min(if total > 0 { total } else { u64::MAX });
            let progress = if total > 0 { (pos as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
            ui.set_song_title(title.into());
            ui.set_subtitle(subtitle.into());
            ui.set_elapsed(fmt_time(pos).into());
            ui.set_right_time(
                if s.show_remaining {
                    format!("-{}", fmt_time(total.saturating_sub(pos)))
                } else {
                    fmt_time(total)
                }
                .into(),
            );
            ui.set_progress(progress);
            ui.set_playing(s.playing);
            ui.set_shuffle(s.shuffle);
            ui.set_repeat(s.repeat);

            if s.meta_dirty {
                s.meta_dirty = false;
                match s.loaded.as_ref().and_then(|l| l.art.clone()) {
                    Some((rgba, w, h)) => {
                        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
                        buf.make_mut_bytes().copy_from_slice(&rgba);
                        ui.set_cover(Image::from_rgba8(buf));
                        ui.set_has_cover(true);
                    }
                    None => ui.set_has_cover(false),
                }
            }

            if s.list_dirty {
                s.list_dirty = false;
                let cur = cur_lib_index(&s);
                let rows: Vec<TrackRow> = s
                    .library
                    .iter()
                    .enumerate()
                    .map(|(i, t)| TrackRow {
                        title: t.title.as_str().into(),
                        album: t.album.as_str().into(),
                        current: Some(i) == cur,
                    })
                    .collect();
                ui.set_tracks(ModelRc::from(Rc::new(VecModel::from(rows))));
            }
        });
    });
}

// ── Transport commands (Slint callbacks + media-session on-action) ───────────
fn cmd_toggle() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        if s.playing {
            pause(&mut s);
        } else {
            play(&mut s);
        }
        after_change_publish(&mut s);
    });
    push_ui();
}
fn cmd_play() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        play(&mut s);
        after_change_publish(&mut s);
    });
    push_ui();
}
fn cmd_pause() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        pause(&mut s);
        after_change_publish(&mut s);
    });
    push_ui();
}
fn cmd_stop() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        pause(&mut s);
        seek_to(&mut s, 0);
        after_change_publish(&mut s);
    });
    push_ui();
}
fn cmd_seek_frac(frac: f32) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        let total = s.loaded.as_ref().map(|l| l.total_frames).unwrap_or(0);
        if total > 0 {
            seek_to(&mut s, (frac.clamp(0.0, 1.0) as f64 * total as f64) as u64);
            after_change_publish(&mut s);
        }
    });
    push_ui();
}
fn cmd_seek_secs(secs: f64) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        seek_to(&mut s, (secs.max(0.0) * OUT_RATE as f64) as u64);
        after_change_publish(&mut s);
    });
    push_ui();
}
fn cmd_seek_rel(delta: f64) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        let cur = position_frames(&s) as f64;
        let total = s.loaded.as_ref().map(|l| l.total_frames).unwrap_or(0);
        let mut target = (cur + delta * OUT_RATE as f64).max(0.0) as u64;
        if total > 0 {
            target = target.min(total);
        }
        seek_to(&mut s, target);
        after_change_publish(&mut s);
    });
    push_ui();
}
fn cmd_next() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        if s.order.is_empty() {
            return;
        }
        let np = (s.order_pos + 1) % s.order.len();
        go_to(&mut s, np, true);
    });
    push_ui();
}
fn cmd_prev() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        if s.order.is_empty() {
            return;
        }
        // >3s in → restart current; else previous track.
        if position_frames(&s) > 3 * OUT_RATE as u64 {
            seek_to(&mut s, 0);
            after_change_publish(&mut s);
        } else {
            let pp = if s.order_pos == 0 { s.order.len() - 1 } else { s.order_pos - 1 };
            go_to(&mut s, pp, true);
        }
    });
    push_ui();
}
fn cmd_select(lib_index: usize) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        if let Some(pos) = s.order.iter().position(|&k| k == lib_index) {
            go_to(&mut s, pos, true);
        }
    });
    push_ui();
}
fn cmd_toggle_shuffle() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        s.shuffle = !s.shuffle;
        let cur = cur_lib_index(&s);
        rebuild_order(&mut s, cur);
    });
    push_ui();
}
fn cmd_toggle_repeat() {
    STATE.with(|st| st.borrow_mut().repeat ^= true);
    push_ui();
}
fn cmd_toggle_time() {
    STATE.with(|st| st.borrow_mut().show_remaining ^= true);
    push_ui();
}

fn engine_tick() -> u32 {
    let delay = STATE.with(|st| engine_step(&mut st.borrow_mut()));
    push_ui();
    delay
}

// ── WIT: the audio-extras world (alongside slint_wandr::launch!) ─────────────
mod bindings {
    slint_wandr::__wit_bindgen::generate!({
        path: "wit",
        world: "audio-extras",
        generate_all,
        runtime_path: "::slint_wandr::__wit_bindgen::rt",
    });

    struct Extras;

    impl exports::wasi::media_session::session_handler::Guest for Extras {
        fn on_action(details: exports::wasi::media_session::session_handler::ActionDetails) {
            use exports::wasi::media_session::session_handler::Action as A;
            match details.action {
                A::Play => crate::cmd_play(),
                A::Pause => crate::cmd_pause(),
                A::Stop => crate::cmd_stop(),
                A::SeekTo => {
                    if let Some(t) = details.seek_time_s {
                        crate::cmd_seek_secs(t);
                    }
                }
                A::SeekForward => crate::cmd_seek_rel(details.seek_time_s.unwrap_or(10.0)),
                A::SeekBackward => crate::cmd_seek_rel(-details.seek_time_s.unwrap_or(10.0)),
                A::PreviousTrack => crate::cmd_prev(),
                A::NextTrack => crate::cmd_next(),
            }
        }
    }

    impl exports::wandr::background::background::Guest for Extras {
        fn bg_tick() -> u32 {
            crate::engine_tick()
        }
    }

    export!(Extras);
}

// ── Slint launch ─────────────────────────────────────────────────────────────
slint_wandr::launch!(|| {
    let ui = MainWindow::new().expect("audio-player: create MainWindow");
    ui.on_toggle(cmd_toggle);
    ui.on_prev_track(cmd_prev);
    ui.on_next_track(cmd_next);
    ui.on_seek(cmd_seek_frac);
    ui.on_toggle_shuffle(cmd_toggle_shuffle);
    ui.on_toggle_repeat(cmd_toggle_repeat);
    ui.on_toggle_time(cmd_toggle_time);
    ui.on_select(|i| cmd_select(i as usize));
    UI.with(|u| *u.borrow_mut() = Some(ui.clone_strong()));
    push_ui();
    ui.show().expect("audio-player: show");
    ui
});
