//! wandr.audio.player — Slint streaming music player with a library browser
//! (task 108 M3). Scans `/music` (host read-only preopen) reading tags, browses
//! by Albums/Artists/Genres/Songs (drill-down), and streams the current track
//! incrementally (Symphonia, low memory) in the `wandr:background/background`
//! bg-tick with a guest-side resampler to the 48 kHz backend. Two views — a
//! Library browser and a Now-Playing screen — with a mini now-playing bar.
//! Engine + media-session publishing run in bg-tick (every role).
//!
//!   cargo build --target wasm32-wasip2 --release
//!   cp target/wasm32-wasip2/release/wandr_audio_player.wasm components/ui.wasm

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

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
/// Last.fm API key (free, from last.fm/api). Empty → the Last.fm source is
/// skipped. Set this to enable album.getInfo lookups.
const LASTFM_API_KEY: &str = "adc9909f24e4992ebe93ccb14dedf65d";
/// Cover-art cache (writable /state preopen). Per album: <key>.img (raw bytes).
const CACHE_DIR: &str = "/state/meta";

// tab ids
const TAB_ALBUMS: i32 = 0;
const TAB_ARTISTS: i32 = 1;
const TAB_GENRES: i32 = 2;
const TAB_SONGS: i32 = 3;

slint::slint! {
    import { ListView } from "std-widgets.slint";

    struct Row { primary: string, secondary: string, current: bool }

    export component MainWindow inherits Window {
        background: #141422;
        in property <int> view: 0;        // 0 = library, 1 = now-playing
        // library
        in property <int> tab: 0;
        in property <bool> in-drill: false;
        in property <string> crumb: "";
        in property <[Row]> rows: [];
        // now-playing / mini-bar
        in property <string> np-title: "—";
        in property <string> np-sub: "";
        in property <string> elapsed: "0:00";
        in property <string> right-time: "0:00";
        in property <float> progress: 0.0;
        in property <bool> playing: false;
        in property <image> cover;
        in property <bool> has-cover: false;
        in property <bool> shuffle: false;
        in property <bool> repeat: false;
        callback set-tab(int);
        callback row-tap(int);
        callback go-back();
        callback open-np();
        callback close-np();
        callback toggle();
        callback prev-track();
        callback next-track();
        callback seek(float);
        callback toggle-shuffle();
        callback toggle-repeat();
        callback toggle-time();

        property <color> on-col: #4285f4;
        property <color> off-col: #8a8aa0;

        // ── Library view ──────────────────────────────────────────────────
        if (root.view == 0) : Rectangle {
            width: 100%; height: 100%;
            VerticalLayout {
                padding: 10px;
                spacing: 6px;

                // Tabs
                HorizontalLayout {
                    spacing: 6px;
                    for t[i] in [ "Albums", "Artists", "Genres", "Songs" ] : Rectangle {
                        height: 34px;
                        border-radius: 8px;
                        background: root.tab == i ? #20203a : transparent;
                        Text {
                            text: t; font-size: 13px;
                            horizontal-alignment: center; vertical-alignment: center;
                            color: root.tab == i ? on-col : #c8c8d8;
                        }
                        TouchArea { clicked => { root.set-tab(i); } }
                    }
                }

                // Breadcrumb / back (when drilled in)
                if (root.in-drill) : Rectangle {
                    height: 30px;
                    HorizontalLayout {
                        spacing: 8px;
                        Rectangle {
                            width: 26px; height: 26px;
                            Path {
                                width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                                commands: "M 60 24 L 34 50 L 60 76"; stroke: white; stroke-width: 9px; fill: transparent;
                            }
                            TouchArea { clicked => { root.go-back(); } }
                        }
                        Text {
                            text: root.crumb; color: white; font-size: 15px; font-weight: 700;
                            vertical-alignment: center; overflow: elide;
                        }
                    }
                }

                // Current level (groups or tracks)
                ListView {
                    vertical-stretch: 1;
                    for r[i] in root.rows : Rectangle {
                        height: 50px;
                        background: r.current ? #20203a : transparent;
                        HorizontalLayout {
                            padding-left: 10px; padding-right: 10px;
                            VerticalLayout {
                                alignment: center;
                                Text {
                                    text: r.primary; font-size: 15px; overflow: elide;
                                    color: r.current ? on-col : white;
                                }
                                Text { text: r.secondary; font-size: 11px; color: #8a8aa0; overflow: elide; }
                            }
                        }
                        TouchArea { clicked => { root.row-tap(i); } }
                    }
                }

                // Mini now-playing bar
                Rectangle {
                    height: 58px; border-radius: 10px; background: #1c1c2e;
                    TouchArea { clicked => { root.open-np(); } }
                    HorizontalLayout {
                        padding: 10px; spacing: 10px;
                        Rectangle {
                            width: 40px; height: 40px; border-radius: 6px; clip: true; background: #24243a;
                            if root.has-cover : Image { width: 100%; height: 100%; source: root.cover; image-fit: ImageFit.cover; }
                        }
                        VerticalLayout {
                            alignment: center; horizontal-stretch: 1;
                            Text { text: root.np-title; color: white; font-size: 14px; overflow: elide; }
                            Text { text: root.np-sub; color: #8a8aa0; font-size: 11px; overflow: elide; }
                        }
                        Rectangle {
                            width: 42px; height: 42px; border-radius: 21px; background: #4285f4;
                            if !root.playing : Path {
                                width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                                commands: "M 38 26 L 74 50 L 38 74 Z"; fill: white;
                            }
                            if root.playing : Path {
                                width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                                commands: "M 35 28 L 45 28 L 45 72 L 35 72 Z M 55 28 L 65 28 L 65 72 L 55 72 Z"; fill: white;
                            }
                            TouchArea { clicked => { root.toggle(); } }
                        }
                    }
                }
            }
        }

        // ── Now-Playing view ──────────────────────────────────────────────
        if (root.view == 1) : Rectangle {
            width: 100%; height: 100%;
            property <length> art-size: min(root.width, root.height) * 0.42;
            property <length> btn: min(root.width, root.height) * 0.15;
            VerticalLayout {
                padding: root.width * 0.06;
                spacing: root.height * 0.018;
                alignment: center;

                // close (down) button
                HorizontalLayout {
                    Rectangle {
                        width: 30px; height: 30px;
                        Path {
                            width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 24 40 L 50 66 L 76 40"; stroke: white; stroke-width: 9px; fill: transparent;
                        }
                        TouchArea { clicked => { root.close-np(); } }
                    }
                    Rectangle { }
                }

                HorizontalLayout {
                    alignment: center;
                    Rectangle {
                        width: art-size; height: art-size; border-radius: art-size * 0.08;
                        clip: true; background: #24243a;
                        if root.has-cover : Image { width: 100%; height: 100%; source: root.cover; image-fit: ImageFit.cover; }
                        if !root.has-cover : Rectangle {
                            width: art-size * 0.66; height: art-size * 0.66; border-radius: self.width / 2; background: #4285f4;
                            Rectangle { width: art-size * 0.14; height: art-size * 0.14; border-radius: self.width / 2; background: #24243a; }
                        }
                    }
                }

                Text { text: root.np-title; color: white; font-size: 20px; font-weight: 700; horizontal-alignment: center; overflow: elide; }
                Text { text: root.np-sub; color: #b0b0c8; font-size: 13px; horizontal-alignment: center; overflow: elide; }

                prog := Rectangle {
                    height: 16px;
                    property <bool> dragging: false;
                    property <float> drag-frac: 0.0;
                    property <float> shown: dragging ? drag-frac : root.progress;
                    Rectangle { width: 100%; height: 4px; y: (parent.height - self.height)/2; border-radius: 2px; background: #3a3a52; }
                    Rectangle { width: parent.width * prog.shown; height: 4px; y: (parent.height - self.height)/2; border-radius: 2px; background: #4285f4; }
                    Rectangle { width: 14px; height: 14px; border-radius: 7px; background: white; x: prog.shown * (parent.width - self.width); y: (parent.height - self.height)/2; }
                    TouchArea {
                        moved => { prog.drag-frac = clamp(self.mouse-x / self.width, 0.0, 1.0); prog.dragging = true; }
                        pointer-event(ev) => {
                            if (ev.kind == PointerEventKind.down) { prog.drag-frac = clamp(self.mouse-x / self.width, 0.0, 1.0); prog.dragging = true; }
                            if (ev.kind == PointerEventKind.up) { root.seek(prog.drag-frac); prog.dragging = false; }
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

                HorizontalLayout {
                    alignment: center; spacing: root.width * 0.045;
                    Rectangle {
                        width: btn * 0.62; height: btn * 0.62;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 14 34 L 42 34 L 86 66 M 74 58 L 88 66 L 76 73 M 14 66 L 42 66 L 86 34 M 76 27 L 88 34 L 74 42";
                            stroke: root.shuffle ? on-col : off-col; stroke-width: 7px; fill: transparent; }
                        TouchArea { clicked => { root.toggle-shuffle(); } }
                    }
                    Rectangle {
                        width: btn * 0.8; height: btn * 0.8;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 64 28 L 40 50 L 64 72 Z M 32 28 L 38 28 L 38 72 L 32 72 Z"; fill: white; }
                        TouchArea { clicked => { root.prev-track(); } }
                    }
                    Rectangle {
                        width: btn; height: btn; border-radius: btn / 2; background: #4285f4;
                        if !root.playing : Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100; commands: "M 36 24 L 76 50 L 36 76 Z"; fill: white; }
                        if root.playing : Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100; commands: "M 34 26 L 45 26 L 45 74 L 34 74 Z M 55 26 L 66 26 L 66 74 L 55 74 Z"; fill: white; }
                        TouchArea { clicked => { root.toggle(); } }
                    }
                    Rectangle {
                        width: btn * 0.8; height: btn * 0.8;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 36 28 L 60 50 L 36 72 Z M 62 28 L 68 28 L 68 72 L 62 72 Z"; fill: white; }
                        TouchArea { clicked => { root.next-track(); } }
                    }
                    Rectangle {
                        width: btn * 0.62; height: btn * 0.62;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 34 38 L 60 38 A 14 14 0 0 1 74 52 L 74 58 M 66 53 L 74 62 L 82 53 M 66 62 L 40 62 A 14 14 0 0 1 26 48 L 26 42 M 18 47 L 26 38 L 34 47";
                            stroke: root.repeat ? on-col : off-col; stroke-width: 7px; fill: transparent; }
                        TouchArea { clicked => { root.toggle-repeat(); } }
                    }
                }
                Rectangle { vertical-stretch: 1; }
            }
        }
    }
}

// ── Library + streaming track ────────────────────────────────────────────────
#[derive(Clone)]
struct LibTrack {
    path: String,
    title: String,
    artist: String,
    album: String,
    genre: String,
    art_path: Option<String>,
    akey: String, // stable album key ("artist|album" from tags) — cover-cache key
}

struct Loaded {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    channels: usize,
    resampler: Option<LinearResampler>,
    pending: Vec<f32>,
    pending_pos: usize,
    eof: bool,
    total_frames: u64,
    title: String,
    subtitle: String,
    art: Option<(Vec<u8>, u32, u32)>, // local albumart fallback
    akey: String,                     // for the fetched-cover lookup
}

#[derive(Default)]
struct State {
    library: Vec<LibTrack>,
    albums: Vec<(String, Vec<usize>)>,
    artists: Vec<(String, Vec<usize>)>,
    genres: Vec<(String, Vec<usize>)>,
    // browse nav
    view: i32, // 0 library, 1 now-playing
    tab: i32,
    drill: Option<usize>,
    // play queue
    queue: Vec<usize>, // natural-order lib indices of the play context
    order: Vec<usize>, // queue, possibly shuffled
    order_pos: usize,
    scanned: bool,
    loaded: Option<Loaded>,
    pb: Option<wpcm::Playback>,
    playing: bool,
    anchor_dev: u64,
    anchor_track: u64,
    sw_frames: u64,
    ended: bool,
    pub_playing: bool,
    last_pub_sec: i64,
    shuffle: bool,
    repeat: bool,
    show_remaining: bool,
    rows_dirty: bool,
    meta_dirty: bool,
    art_cache: Option<(String, Vec<u8>, u32, u32)>,
    rng: u64,
    // Phase A — fetched/cached internet cover art, decoded RGBA, keyed by akey.
    album_art: HashMap<String, (Vec<u8>, u32, u32)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static UI: RefCell<Option<MainWindow>> = const { RefCell::new(None) };
}

// ── Linear streaming resampler (src → 48k) ───────────────────────────────────
struct LinearResampler {
    step: f64,
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

// ── Library scan (reads tags) ────────────────────────────────────────────────
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

fn ext_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, e)| e).unwrap_or("")
}

/// Read (title, artist, album, genre) tags without decoding audio.
fn read_tags(path: &str) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let none = (None, None, None, None);
    let Ok(file) = std::fs::File::open(path) else {
        return none;
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext_of(path));
    let Ok(mut probed) = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) else {
        return none;
    };
    let (mut ti, mut ar, mut al, mut ge) = (None, None, None, None);
    let mut scan = |rev: &symphonia::core::meta::MetadataRevision| {
        for tag in rev.tags() {
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) => ti = Some(tag.value.to_string()),
                Some(StandardTagKey::Artist) => ar = Some(tag.value.to_string()),
                Some(StandardTagKey::AlbumArtist) => {
                    if ar.is_none() {
                        ar = Some(tag.value.to_string())
                    }
                }
                Some(StandardTagKey::Album) => al = Some(tag.value.to_string()),
                Some(StandardTagKey::Genre) => ge = Some(tag.value.to_string()),
                _ => {}
            }
        }
    };
    if let Some(rev) = probed.format.metadata().current() {
        scan(rev);
    }
    if let Some(rev) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
        scan(rev);
    }
    (ti, ar, al, ge)
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
        let dir_album = apath.file_name().unwrap().to_string_lossy().to_string();
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
            let path = p.to_string_lossy().to_string();
            let fname = p.file_name().unwrap().to_string_lossy().to_string();
            let (ti, ar, al, ge) = read_tags(&path);
            let artist = ar.unwrap_or_else(|| "Unknown Artist".to_string());
            let album = al.unwrap_or_else(|| dir_album.clone());
            let akey = format!("{artist}|{album}");
            out.push(LibTrack {
                path,
                title: ti.unwrap_or_else(|| pretty_title(&fname)),
                artist,
                album,
                genre: ge.unwrap_or_else(|| "Unknown Genre".to_string()),
                art_path: art_path.clone(),
                akey,
            });
        }
    }
    out
}

fn group_by(lib: &[LibTrack], key: impl Fn(&LibTrack) -> &str) -> Vec<(String, Vec<usize>)> {
    let mut m: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, t) in lib.iter().enumerate() {
        m.entry(key(t).to_string()).or_default().push(i);
    }
    m.into_iter().collect()
}

fn load_art(s: &mut State, art_path: &Option<String>) -> Option<(Vec<u8>, u32, u32)> {
    let path = art_path.as_ref()?;
    if let Some((cached, rgba, w, h)) = &s.art_cache {
        if cached == path {
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

// ── Load + stream ────────────────────────────────────────────────────────────
fn load(s: &mut State, lib_index: usize) {
    s.pb = None;
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
    let format = probed.format;
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
    let total_frames = n_frames
        .map(|nf| (nf as u128 * OUT_RATE as u128 / src_rate.max(1) as u128) as u64)
        .unwrap_or(0);
    // Prefer a fetched/cached internet cover; fall back to the local albumart file.
    let art = s.album_art.get(&entry.akey).cloned().or_else(|| load_art(s, &entry.art_path));
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
        title: entry.title.clone(),
        subtitle: format!("{} — {}", entry.artist, entry.album),
        art,
        akey: entry.akey.clone(),
    });
    s.anchor_dev = 0;
    s.anchor_track = 0;
    s.sw_frames = 0;
    s.ended = false;
    s.rows_dirty = true;
    s.meta_dirty = true;
    publish_metadata(s, &entry);
    publish_state(s);
    publish_position(s);
    s.pub_playing = s.playing;
    s.last_pub_sec = -1;
}

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
                break;
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
    s.ended = false;
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
    if s.order_pos + 1 < s.order.len() {
        go_to(s, s.order_pos + 1, true);
    } else if s.repeat {
        go_to(s, 0, true);
    } else {
        s.pb = None;
        s.playing = false;
        s.ended = true;
        if let Some(l) = &s.loaded {
            s.sw_frames = l.total_frames;
        }
    }
}

fn fisher_yates(v: &mut [usize], rng: &mut u64) {
    let mut i = v.len();
    while i > 1 {
        i -= 1;
        *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (*rng >> 33) as usize % (i + 1);
        v.swap(i, j);
    }
}

/// Rebuild `order` from `queue` (shuffled or natural), positioned at `keep`.
fn apply_order(s: &mut State, keep_lib: Option<usize>) {
    if s.shuffle {
        let mut v = s.queue.clone();
        fisher_yates(&mut v, &mut s.rng);
        s.order = v;
    } else {
        s.order = s.queue.clone();
    }
    if let Some(lib) = keep_lib {
        s.order_pos = s.order.iter().position(|&k| k == lib).unwrap_or(0);
    } else {
        s.order_pos = 0;
    }
}

/// Start playing from a browsed track list (becomes the queue).
fn play_from(s: &mut State, tracks: Vec<usize>, tapped: usize) {
    if tracks.is_empty() {
        return;
    }
    let keep = tracks[tapped.min(tracks.len() - 1)];
    s.queue = tracks;
    apply_order(s, Some(keep));
    go_to(s, s.order_pos, true);
}

// ── Browse helpers ───────────────────────────────────────────────────────────
fn showing_tracks(s: &State) -> bool {
    s.tab == TAB_SONGS || s.drill.is_some()
}

fn groups_for(s: &State) -> &[(String, Vec<usize>)] {
    match s.tab {
        TAB_ALBUMS => &s.albums,
        TAB_ARTISTS => &s.artists,
        TAB_GENRES => &s.genres,
        _ => &[],
    }
}

fn current_track_list(s: &State) -> Vec<usize> {
    if s.tab == TAB_SONGS {
        return (0..s.library.len()).collect();
    }
    match s.drill {
        Some(g) => groups_for(s).get(g).map(|(_, idxs)| idxs.clone()).unwrap_or_default(),
        None => Vec::new(),
    }
}

// ── media-session publishing ────────────────────────────────────────────────
fn publish_metadata(_s: &State, entry: &LibTrack) {
    wsession::set_metadata(&wsession::Metadata {
        title: entry.title.clone(),
        artist: entry.artist.clone(),
        album: entry.album.clone(),
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

// ── Engine step (bg-tick) ────────────────────────────────────────────────────
fn engine_step(s: &mut State) -> u32 {
    if !s.scanned {
        s.scanned = true;
        s.rng = 0x9E3779B97F4A7C15;
        wandr_step_executor::init(); // Phase A — async metadata-fetch reactor
        s.library = scan_library();
        s.albums = group_by(&s.library, |t| &t.album);
        s.artists = group_by(&s.library, |t| &t.artist);
        s.genres = group_by(&s.library, |t| &t.genre);
        s.queue = (0..s.library.len()).collect();
        s.order = s.queue.clone();
        s.rows_dirty = true;
        // Phase A — covers: load from the /state cache where present, else queue a
        // fetch. One sequential task does the network work (rate-limit-friendly).
        let albums: Vec<(String, String, String)> = s
            .albums
            .iter()
            .filter_map(|(_, idxs)| {
                idxs.first().map(|&i| {
                    let t = &s.library[i];
                    (t.akey.clone(), t.artist.clone(), t.album.clone())
                })
            })
            .collect();
        let mut to_fetch = Vec::new();
        for (akey, artist, album) in albums {
            if let Some(dec) = load_cached_cover(&akey) {
                s.album_art.insert(akey, dec);
            } else {
                to_fetch.push((akey, artist, album));
            }
        }
        spawn_library_fetch(to_fetch);
        if !s.library.is_empty() {
            go_to(s, 0, false);
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
            // now-playing fields
            let (title, subtitle, total) = match &s.loaded {
                Some(l) => (l.title.clone(), l.subtitle.clone(), l.total_frames),
                None => ("—".to_string(), String::new(), 0),
            };
            let pos = position_frames(&s).min(if total > 0 { total } else { u64::MAX });
            ui.set_np_title(title.into());
            ui.set_np_sub(subtitle.into());
            ui.set_elapsed(fmt_time(pos).into());
            ui.set_right_time(
                if s.show_remaining { format!("-{}", fmt_time(total.saturating_sub(pos))) } else { fmt_time(total) }.into(),
            );
            ui.set_progress(if total > 0 { (pos as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 });
            ui.set_playing(s.playing);
            ui.set_shuffle(s.shuffle);
            ui.set_repeat(s.repeat);
            ui.set_view(s.view);
            ui.set_tab(s.tab);
            ui.set_in_drill(s.drill.is_some());
            ui.set_crumb(
                s.drill
                    .and_then(|g| groups_for(&s).get(g).map(|(n, _)| n.clone()))
                    .unwrap_or_default()
                    .into(),
            );

            if s.meta_dirty {
                s.meta_dirty = false;
                // Prefer a fetched/cached internet cover (by the current album's
                // akey), else the local albumart loaded with the track.
                let art = s
                    .loaded
                    .as_ref()
                    .and_then(|l| s.album_art.get(&l.akey).cloned().or_else(|| l.art.clone()));
                match art {
                    Some((rgba, w, h)) => {
                        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
                        buf.make_mut_bytes().copy_from_slice(&rgba);
                        ui.set_cover(Image::from_rgba8(buf));
                        ui.set_has_cover(true);
                    }
                    None => ui.set_has_cover(false),
                }
            }

            if s.rows_dirty {
                s.rows_dirty = false;
                let cur = cur_lib_index(&s);
                let rows: Vec<Row> = if showing_tracks(&s) {
                    current_track_list(&s)
                        .iter()
                        .map(|&li| {
                            let t = &s.library[li];
                            Row {
                                primary: t.title.as_str().into(),
                                secondary: t.artist.as_str().into(),
                                current: Some(li) == cur,
                            }
                        })
                        .collect()
                } else {
                    groups_for(&s)
                        .iter()
                        .map(|(name, idxs)| Row {
                            primary: name.as_str().into(),
                            secondary: format!("{} song{}", idxs.len(), if idxs.len() == 1 { "" } else { "s" }).into(),
                            current: false,
                        })
                        .collect()
                };
                ui.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
            }
        });
    });
}

// ── Commands ─────────────────────────────────────────────────────────────────
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
fn cmd_toggle_shuffle() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        s.shuffle = !s.shuffle;
        let cur = cur_lib_index(&s);
        apply_order(&mut s, cur);
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
fn cmd_set_tab(tab: i32) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        s.tab = tab;
        s.drill = None;
        s.rows_dirty = true;
    });
    push_ui();
}
fn cmd_go_back() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        s.drill = None;
        s.rows_dirty = true;
    });
    push_ui();
}
fn cmd_row_tap(i: usize) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        if showing_tracks(&s) {
            let tracks = current_track_list(&s);
            if i < tracks.len() {
                play_from(&mut s, tracks, i);
            }
        } else if i < groups_for(&s).len() {
            s.drill = Some(i);
            s.rows_dirty = true;
        }
    });
    push_ui();
}
fn cmd_open_np() {
    STATE.with(|st| st.borrow_mut().view = 1);
    push_ui();
}
fn cmd_close_np() {
    STATE.with(|st| st.borrow_mut().view = 0);
    push_ui();
}

fn engine_tick() -> u32 {
    let delay = STATE.with(|st| engine_step(&mut st.borrow_mut()));
    push_ui();
    wandr_step_executor::step(); // advance any in-flight metadata fetch (non-blocking)
    delay
}

// ── Phase A: internet metadata lookup (increment 1 — fetch + log) ────────────
fn q_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push_str("%20");
        } else {
            out.push('%');
            let h = |n: u8| (if n < 10 { b'0' + n } else { b'A' + n - 10 }) as char;
            out.push(h(b >> 4));
            out.push(h(b & 0xf));
        }
    }
    out
}

/// stderr→logcat bridges per write(), so build the whole line + emit it in ONE
/// write_all (a plain eprintln! splits a long line across many logcat entries).
fn log1(s: &str) {
    use std::io::Write;
    let _ = std::io::stderr().write_all(format!("{s}\n").as_bytes());
}

/// One source's album candidate.
#[derive(Default, Clone)]
struct Cand {
    name: String,
    artist: String,
    cover: String,
    src: &'static str,
}
impl Cand {
    fn ok(&self) -> bool {
        !self.cover.is_empty()
    }
}

// ── cover download + cache ───────────────────────────────────────────────────
fn decode_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((rgba.into_raw(), w, h))
}

fn cache_file(akey: &str) -> String {
    let safe: String = akey.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("{CACHE_DIR}/{safe}.img")
}

fn load_cached_cover(akey: &str) -> Option<(Vec<u8>, u32, u32)> {
    let bytes = std::fs::read(cache_file(akey)).ok()?;
    decode_rgba(&bytes)
}

fn save_cached_cover(akey: &str, bytes: &[u8]) {
    let _ = std::fs::create_dir_all(CACHE_DIR);
    let _ = std::fs::write(cache_file(akey), bytes);
}

/// GET raw bytes, following up to 6 redirects manually (the shim sends one
/// request; Cover Art Archive 302s to archive.org).
async fn download_bytes(client: &reqwest::Client, url0: &str) -> Option<Vec<u8>> {
    let mut url = url0.to_string();
    for _ in 0..6 {
        let u = url::Url::parse(&url).ok()?;
        let resp = client.get(u).send().await.ok()?;
        let st = resp.status();
        if st.is_redirection() {
            let loc = resp.headers().get("location").and_then(|h| h.to_str().ok())?.to_string();
            url = if loc.starts_with("http") {
                loc
            } else {
                url::Url::parse(&url).ok()?.join(&loc).ok()?.to_string()
            };
            continue;
        }
        if !st.is_success() {
            return None;
        }
        return Some(resp.bytes().await.ok()?.to_vec());
    }
    None
}

async fn get_json(client: &reqwest::Client, url: &str) -> Option<serde_json::Value> {
    let u = url::Url::parse(url).ok()?;
    let resp = client.get(u).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    serde_json::from_str(&body).ok()
}

fn jstr(v: &serde_json::Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        let next = match p.parse::<usize>() {
            Ok(i) => cur.get(i),   // array index
            Err(_) => cur.get(*p), // object key
        };
        cur = match next {
            Some(x) => x,
            None => return String::new(),
        };
    }
    cur.as_str().unwrap_or("").to_string()
}

/// MusicBrainz release search → Cover Art Archive front image by release MBID.
async fn lookup_musicbrainz(client: &reqwest::Client, artist: &str, album: &str) -> Cand {
    let q = q_encode(&format!("release:\"{album}\" AND artist:\"{artist}\""));
    let url = format!("https://musicbrainz.org/ws/2/release?query={q}&fmt=json&limit=3");
    let Some(v) = get_json(client, &url).await else { return Cand::default() };
    let Some(rel) = v.get("releases").and_then(|r| r.get(0)) else { return Cand::default() };
    let mbid = jstr(rel, &["id"]);
    if mbid.is_empty() {
        return Cand::default();
    }
    Cand {
        name: jstr(rel, &["title"]),
        artist: jstr(rel, &["artist-credit", "0", "name"]),
        cover: format!("https://coverartarchive.org/release/{mbid}/front-500"),
        src: "musicbrainz",
    }
}

/// Deezer album search (no key) → cover_xl.
async fn lookup_deezer(client: &reqwest::Client, artist: &str, album: &str) -> Cand {
    let q = q_encode(&format!("artist:\"{artist}\" album:\"{album}\""));
    let url = format!("https://api.deezer.com/search/album?q={q}&limit=3");
    let Some(v) = get_json(client, &url).await else { return Cand::default() };
    let Some(d) = v.get("data").and_then(|d| d.get(0)) else { return Cand::default() };
    let cover = {
        let xl = jstr(d, &["cover_xl"]);
        if xl.is_empty() { jstr(d, &["cover_big"]) } else { xl }
    };
    Cand { name: jstr(d, &["title"]), artist: jstr(d, &["artist", "name"]), cover, src: "deezer" }
}

/// iTunes Search (no key) → artworkUrl (upscaled to 600×600).
async fn lookup_itunes(client: &reqwest::Client, artist: &str, album: &str) -> Cand {
    let term = q_encode(&format!("{artist} {album}"));
    let url = format!("https://itunes.apple.com/search?term={term}&entity=album&limit=3");
    let Some(v) = get_json(client, &url).await else { return Cand::default() };
    let Some(r) = v.get("results").and_then(|r| r.get(0)) else { return Cand::default() };
    let cover = jstr(r, &["artworkUrl100"]).replace("100x100bb", "600x600bb");
    Cand { name: jstr(r, &["collectionName"]), artist: jstr(r, &["artistName"]), cover, src: "itunes" }
}

/// Last.fm album.getInfo (needs LASTFM_API_KEY) → largest non-placeholder image.
async fn lookup_lastfm(client: &reqwest::Client, artist: &str, album: &str) -> Cand {
    if LASTFM_API_KEY.is_empty() {
        return Cand::default();
    }
    let url = format!(
        "https://ws.audioscrobbler.com/2.0/?method=album.getinfo&api_key={}&artist={}&album={}&format=json",
        LASTFM_API_KEY,
        q_encode(artist),
        q_encode(album),
    );
    let Some(v) = get_json(client, &url).await else { return Cand::default() };
    // image[] is ordered small→mega; keep the last non-empty, non-placeholder url.
    let cover = v
        .get("album")
        .and_then(|a| a.get("image"))
        .and_then(|i| i.as_array())
        .map(|arr| {
            let mut best = String::new();
            for img in arr {
                let u = img.get("#text").and_then(|s| s.as_str()).unwrap_or("");
                // skip Last.fm's "no cover" star placeholder
                if !u.is_empty() && !u.contains("2a96cbd8b46e442fc41c2b86b821562f") {
                    best = u.to_string();
                }
            }
            best
        })
        .unwrap_or_default();
    Cand { name: jstr(&v, &["album", "name"]), artist: jstr(&v, &["album", "artist"]), cover, src: "lastfm" }
}

/// One album's multi-source lookup: query MusicBrainz → Deezer → iTunes, log
/// each, return the first candidate that yielded a cover. (Caching + UI next.)
async fn fetch_one(client: &reqwest::Client, artist: &str, album: &str) -> Option<Cand> {
    log1(&format!("[meta] lookup {artist:?} / {album:?}"));
    let mb = lookup_musicbrainz(client, artist, album).await;
    log1(&format!("[meta]   musicbrainz: name={:?} artist={:?} cover={}", mb.name, mb.artist, mb.cover));
    let dz = lookup_deezer(client, artist, album).await;
    log1(&format!("[meta]   deezer:      name={:?} artist={:?} cover={}", dz.name, dz.artist, dz.cover));
    let it = lookup_itunes(client, artist, album).await;
    log1(&format!("[meta]   itunes:      name={:?} artist={:?} cover={}", it.name, it.artist, it.cover));
    let lf = lookup_lastfm(client, artist, album).await;
    log1(&format!("[meta]   lastfm:      name={:?} artist={:?} cover={}", lf.name, lf.artist, lf.cover));
    let chosen = [mb, dz, it, lf].into_iter().find(|c| c.ok());
    match &chosen {
        Some(c) => log1(&format!("[meta] CHOSEN [{}] name={:?} artist={:?} cover={}", c.src, c.name, c.artist, c.cover)),
        None => log1("[meta] CHOSEN: none"),
    }
    chosen
}

/// Pre-fetch metadata/art for every album in one sequential task (on the
/// step-executor, so it runs across bg-ticks without blocking audio). Sequential
/// + a courtesy delay respects MusicBrainz's 1 req/s limit.
fn spawn_library_fetch(albums: Vec<(String, String, String)>) {
    if albums.is_empty() {
        return;
    }
    wandr_step_executor::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent("wandr-audio-player/0.1 ( https://codeberg.org/harryzz/wandr )")
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                log1(&format!("[meta] client err: {e:?}"));
                return;
            }
        };
        for (akey, artist, album) in albums {
            if let Some(c) = fetch_one(&client, &artist, &album).await {
                match download_bytes(&client, &c.cover).await {
                    Some(bytes) => {
                        save_cached_cover(&akey, &bytes);
                        if let Some(dec) = decode_rgba(&bytes) {
                            log1(&format!("[meta] cover {akey} {}x{} ({} bytes) via {}", dec.1, dec.2, bytes.len(), c.src));
                            STATE.with(|st| {
                                let mut s = st.borrow_mut();
                                s.album_art.insert(akey.clone(), dec);
                                s.meta_dirty = true;
                            });
                            push_ui();
                        }
                    }
                    None => log1(&format!("[meta] cover download failed: {}", c.cover)),
                }
            }
            wandr_step_executor::sleep(Duration::from_millis(1100)).await;
        }
        log1("[meta] library fetch complete");
    })
    .detach();
}

// ── WIT bindings (alongside slint_wandr::launch!) ────────────────────────────
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
    ui.on_set_tab(|t| cmd_set_tab(t));
    ui.on_row_tap(|i| cmd_row_tap(i as usize));
    ui.on_go_back(cmd_go_back);
    ui.on_open_np(cmd_open_np);
    ui.on_close_np(cmd_close_np);
    UI.with(|u| *u.borrow_mut() = Some(ui.clone_strong()));
    push_ui();
    ui.show().expect("audio-player: show");
    ui
});
