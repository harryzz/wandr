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
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

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
// Spectrum visualizer. FFT_SIZE = analysis window; NUM_BARS = log-spaced bands
// rendered (a spectral-resolution choice, not device geometry); SCOPE_CAP = mono
// history ring, sized to cover the device buffer depth + one FFT window so the
// visualized window can be aligned to the audible position.
const FFT_SIZE: usize = 1024;
const NUM_BARS: usize = 48;
const SCOPE_CAP: usize = 1 << 16; // 65536 frames ≈ 1.36 s @ 48 kHz
const VIZ_LO_HZ: f32 = 40.0;
const VIZ_HI_HZ: f32 = 16_000.0;
// 10-band graphic EQ — ISO octave centers, peaking biquads (RBJ cookbook),
// Q for ~1-octave bandwidth, ±EQ_RANGE_DB user range. Applied in the decode path.
const EQ_BANDS: usize = 10;
const EQ_FREQS: [f32; EQ_BANDS] =
    [31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
const EQ_Q: f32 = 1.41; // sqrt(2) → ~1 octave
const EQ_RANGE_DB: f32 = 12.0;
const EQ_FILE: &str = "/state/eq.json";
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
        in property <[float]> bars: [];
        // graphic EQ
        in property <bool> eq-enabled: false;
        in property <[float]> eq-gains: [];   // dB per band
        in property <[string]> eq-labels: [];
        in property <float> eq-range: 12.0;   // ± dB
        in property <string> xfade-label: "Off";
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
        callback open-eq();
        callback close-eq();
        callback eq-toggle();
        callback eq-flat();
        callback eq-set-band(int, float);
        callback cycle-xfade();

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

                // close (down) button + EQ entry
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
                    Rectangle {
                        width: 44px; height: 30px; border-radius: 8px;
                        background: root.eq-enabled ? #20203a : transparent;
                        Text {
                            text: "EQ"; font-size: 14px; font-weight: 700;
                            horizontal-alignment: center; vertical-alignment: center;
                            color: root.eq-enabled ? #4285f4 : #c8c8d8;
                        }
                        TouchArea { clicked => { root.open-eq(); } }
                    }
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

                // Spectrum visualizer — log-spaced bands of the audible audio.
                Rectangle {
                    height: min(root.width, root.height) * 0.13;
                    HorizontalLayout {
                        spacing: 3px;
                        for b in root.bars : Rectangle {
                            horizontal-stretch: 1;
                            Rectangle {
                                width: parent.width;
                                height: max(2px, parent.height * b);
                                y: parent.height - self.height;
                                border-radius: 2px;
                                background: @linear-gradient(0deg, #4285f4 0%, #34e0c8 100%);
                            }
                        }
                    }
                }

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

        // ── EQ view ───────────────────────────────────────────────────────
        if (root.view == 2) : Rectangle {
            width: 100%; height: 100%;
            VerticalLayout {
                padding: root.width * 0.05;
                spacing: root.height * 0.02;

                // header: back · title · flat · enable
                HorizontalLayout {
                    spacing: 10px;
                    Rectangle {
                        width: 30px; height: 30px;
                        Path {
                            width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 60 24 L 34 50 L 60 76"; stroke: white; stroke-width: 9px; fill: transparent;
                        }
                        TouchArea { clicked => { root.close-eq(); } }
                    }
                    Text {
                        text: "Equalizer"; color: white; font-size: 18px; font-weight: 700;
                        vertical-alignment: center; horizontal-stretch: 1;
                    }
                    Rectangle {
                        width: 56px; height: 30px; border-radius: 8px; background: #20203a;
                        Text { text: "Flat"; font-size: 13px; color: #c8c8d8;
                            horizontal-alignment: center; vertical-alignment: center; }
                        TouchArea { clicked => { root.eq-flat(); } }
                    }
                    Rectangle {
                        width: 52px; height: 30px; border-radius: 15px;
                        background: root.eq-enabled ? #4285f4 : #3a3a52;
                        Rectangle {
                            width: 24px; height: 24px; border-radius: 12px; background: white;
                            x: root.eq-enabled ? parent.width - self.width - 3px : 3px;
                            y: (parent.height - self.height) / 2;
                        }
                        TouchArea { clicked => { root.eq-toggle(); } }
                    }
                }

                // band sliders
                HorizontalLayout {
                    spacing: 4px;
                    for g[i] in root.eq-gains : VerticalLayout {
                        horizontal-stretch: 1;
                        spacing: 4px;
                        sl := Rectangle {
                            vertical-stretch: 1;
                            property <length> usable: self.height - 16px;
                            // center reference line
                            Rectangle {
                                width: 3px; height: 100%; border-radius: 1px; background: #2a2a40;
                                x: (parent.width - self.width) / 2;
                            }
                            // thumb
                            Rectangle {
                                width: 70%; height: 16px; border-radius: 4px;
                                x: (parent.width - self.width) / 2;
                                y: (1 - (g + root.eq-range) / (2 * root.eq-range)) * sl.usable;
                                background: root.eq-enabled ? #4285f4 : #555568;
                            }
                            TouchArea {
                                moved => {
                                    root.eq-set-band(i, (1 - self.mouse-y / self.height) * 2 * root.eq-range - root.eq-range);
                                }
                                pointer-event(ev) => {
                                    if (ev.kind == PointerEventKind.down) {
                                        root.eq-set-band(i, (1 - self.mouse-y / self.height) * 2 * root.eq-range - root.eq-range);
                                    }
                                }
                            }
                        }
                        Text {
                            text: root.eq-labels[i]; font-size: 9px; color: #8a8aa0;
                            horizontal-alignment: center;
                        }
                    }
                }

                // crossfade
                HorizontalLayout {
                    spacing: 10px;
                    Text {
                        text: "Crossfade"; color: #c8c8d8; font-size: 14px;
                        vertical-alignment: center; horizontal-stretch: 1;
                    }
                    Rectangle {
                        width: 64px; height: 32px; border-radius: 8px; background: #20203a;
                        Text {
                            text: root.xfade-label; font-size: 14px; font-weight: 700;
                            color: #4285f4; horizontal-alignment: center; vertical-alignment: center;
                        }
                        TouchArea { clicked => { root.cycle-xfade(); } }
                    }
                }
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
    track_no: Option<u32>, // from tags — orders canonical titles within an album
    art_path: Option<String>,
    akey: String, // stable album key ("artist|album" from tags) — cover-cache key
    // ReplayGain (from tags) — dB gains + linear peaks, track + album scope.
    rg_track_gain: Option<f32>,
    rg_track_peak: Option<f32>,
    rg_album_gain: Option<f32>,
    rg_album_peak: Option<f32>,
}

/// The track currently being DECODED (the decode head). Decode runs ahead of
/// playback for gapless transitions; all display/metadata is derived from the
/// library via the audible `Seg`, not from here.
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
    gain: f32,        // ReplayGain linear factor (1.0 = none)
    base: u64,        // track-frame of the first decoded sample (0, or a seek target)
    seg_started: bool, // a Seg has been pushed for this decode head
    decoded: u64,     // output (48k) frames decoded so far — drives crossfade onset
    eq_z: Vec<[f32; 2]>, // per-stream EQ filter state (so a crossfade pair filters cleanly)
}

/// An in-progress crossfade: the incoming track decoded in parallel, mixed with
/// the outgoing one over `dur` output frames (equal-power curve).
struct Xfade {
    next: Loaded,
    order: usize, // order index of the incoming track
    pos: u64,
    dur: u64,
}

/// One written-but-not-fully-played track segment. Maps device frames (the
/// playback `position()` counter, which never resets across gapless transitions)
/// to a track + its position. The front segment is the currently AUDIBLE track.
#[derive(Clone)]
struct Seg {
    order_pos: usize,
    lib_index: usize,
    total_frames: u64,
    dev_start: u64, // device-frame at which this segment's `base` frame is heard
    base: u64,      // track-frame at dev_start (0, or a seek target)
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
    loaded: Option<Loaded>,       // decode head
    decode_order: usize,          // order index of the decode head
    segments: VecDeque<Seg>,      // written tracks; front = audible
    pb: Option<wpcm::Playback>,
    pb_stereo: bool,              // output layout the device was opened with
    gapless_blocked: bool,        // next track's layout differs → fall back to reopen
    playing: bool,
    sw_frames: u64,               // position shown when pb is None (pre-play / stopped)
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
    // Visualizer: mono history ring (written-frame coords), smoothed band levels,
    // a reusable FFT plan, and a precomputed Hann window.
    scope: Vec<f32>,
    scope_written: u64, // mono frames WRITTEN to the device since open (not decoded)
    open_dev: u64,      // pb.position() at open/flush → played = position() - open_dev
    bars: Vec<f32>,
    bars_were_active: bool, // pushed live last frame → flatten once when stopping
    fft: Option<Arc<dyn rustfft::Fft<f32>>>,
    hann: Vec<f32>,
    // Graphic EQ: per-band gain (dB) + normalized biquad coeffs [b0,b1,b2,a1,a2].
    // Filter state ([z1,z2] per channel/band) lives per-Loaded. eq_active =
    // enabled && non-flat.
    eq_enabled: bool,
    eq_gains: Vec<f32>,
    eq_coeffs: Vec<[f32; 5]>,
    eq_active: bool,
    // Crossfade: blend duration (s, 0 = off, gapless) + the in-progress blend.
    xfade_secs: f32,
    xfade: Option<Xfade>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static UI: RefCell<Option<MainWindow>> = const { RefCell::new(None) };
    // Persistent visualizer model — updated in place (set_row_data) every frame so
    // only the bar heights re-evaluate (Slint partial redraw), never rebuilding the
    // 48-item repeater. Bound to the `bars` property once at launch.
    static BARS_MODEL: Rc<VecModel<f32>> = Rc::new(VecModel::from(vec![0.0_f32; NUM_BARS]));
    // Drives continuous frames while the visualizer is on screen. The host's
    // on-demand renderer only reschedules via next_frame_delay() (polled AFTER a
    // render); a pending Slint timer makes it return ~16ms, so frames keep coming.
    // Its callback requests the redraw. Bootstraps off the tap that opens
    // now-playing (input → render → re-poll). Stopped when not visualizing.
    static VIZ_TIMER: slint::Timer = slint::Timer::default();
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

// ── Graphic EQ (biquad peaking, RBJ cookbook) ────────────────────────────────
/// Recompute normalized peaking-biquad coeffs for every band from eq_gains, and
/// set eq_active (enabled and at least one non-flat band).
fn eq_recalc(s: &mut State) {
    if s.eq_coeffs.len() != EQ_BANDS {
        s.eq_coeffs = vec![[1.0, 0.0, 0.0, 0.0, 0.0]; EQ_BANDS];
    }
    let mut nonflat = false;
    for b in 0..EQ_BANDS {
        let gain = s.eq_gains.get(b).copied().unwrap_or(0.0);
        if gain.abs() > 0.05 {
            nonflat = true;
        }
        let a = 10f32.powf(gain / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * EQ_FREQS[b] / OUT_RATE as f32;
        let (sw, cw) = (w0.sin(), w0.cos());
        let alpha = sw / (2.0 * EQ_Q);
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cw;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cw;
        let a2 = 1.0 - alpha / a;
        s.eq_coeffs[b] = [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
    }
    s.eq_active = s.eq_enabled && nonflat;
}

/// Apply the band cascade in place to interleaved output (DF2T per band/channel).
fn eq_apply(coeffs: &[[f32; 5]], z: &mut [[f32; 2]], out: &mut [f32], ch: usize) {
    let n = out.len() / ch;
    for f in 0..n {
        for c in 0..ch.min(2) {
            let mut x = out[f * ch + c];
            for b in 0..EQ_BANDS {
                let co = &coeffs[b];
                let st = &mut z[c * EQ_BANDS + b];
                let y = co[0] * x + st[0];
                st[0] = co[1] * x - co[3] * y + st[1];
                st[1] = co[2] * x - co[4] * y;
                x = y;
            }
            out[f * ch + c] = x;
        }
    }
}

fn eq_reset_state(s: &mut State) {
    if let Some(l) = s.loaded.as_mut() {
        for v in &mut l.eq_z {
            *v = [0.0, 0.0];
        }
    }
}

fn save_eq(s: &State) {
    let _ = std::fs::create_dir_all("/state");
    let v = serde_json::json!({
        "enabled": s.eq_enabled,
        "gains": s.eq_gains,
        "xfade": s.xfade_secs,
    });
    let _ = std::fs::write(EQ_FILE, v.to_string());
}

fn load_eq(s: &mut State) {
    let Ok(body) = std::fs::read_to_string(EQ_FILE) else { return };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else { return };
    s.eq_enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    if let Some(arr) = v.get("gains").and_then(|x| x.as_array()) {
        for (b, g) in arr.iter().take(EQ_BANDS).enumerate() {
            if let Some(d) = g.as_f64() {
                s.eq_gains[b] = (d as f32).clamp(-EQ_RANGE_DB, EQ_RANGE_DB);
            }
        }
    }
    s.xfade_secs = v.get("xfade").and_then(|x| x.as_f64()).unwrap_or(0.0).clamp(0.0, 12.0) as f32;
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

#[derive(Default)]
struct Tags {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    track_no: Option<u32>,
    rg_track_gain: Option<f32>,
    rg_track_peak: Option<f32>,
    rg_album_gain: Option<f32>,
    rg_album_peak: Option<f32>,
}

/// ReplayGain gain tags look like "-7.89 dB"; peaks like "0.987264".
fn parse_rg_db(v: &str) -> Option<f32> {
    v.trim().trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == ' ').trim().parse().ok()
}
fn parse_rg_peak(v: &str) -> Option<f32> {
    v.trim().parse().ok()
}

/// Read tags without decoding audio.
fn read_tags(path: &str) -> Tags {
    let Ok(file) = std::fs::File::open(path) else {
        return Tags::default();
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
        return Tags::default();
    };
    let mut t = Tags::default();
    let mut scan = |rev: &symphonia::core::meta::MetadataRevision| {
        for tag in rev.tags() {
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) => t.title = Some(tag.value.to_string()),
                Some(StandardTagKey::Artist) => t.artist = Some(tag.value.to_string()),
                Some(StandardTagKey::AlbumArtist) => {
                    if t.artist.is_none() {
                        t.artist = Some(tag.value.to_string())
                    }
                }
                Some(StandardTagKey::Album) => t.album = Some(tag.value.to_string()),
                Some(StandardTagKey::Genre) => t.genre = Some(tag.value.to_string()),
                Some(StandardTagKey::TrackNumber) => {
                    // value may be "3" or "3/12" — take the leading integer.
                    let s = tag.value.to_string();
                    let n: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(v) = n.parse::<u32>() {
                        t.track_no = Some(v);
                    }
                }
                Some(StandardTagKey::ReplayGainTrackGain) => {
                    t.rg_track_gain = parse_rg_db(&tag.value.to_string())
                }
                Some(StandardTagKey::ReplayGainTrackPeak) => {
                    t.rg_track_peak = parse_rg_peak(&tag.value.to_string())
                }
                Some(StandardTagKey::ReplayGainAlbumGain) => {
                    t.rg_album_gain = parse_rg_db(&tag.value.to_string())
                }
                Some(StandardTagKey::ReplayGainAlbumPeak) => {
                    t.rg_album_peak = parse_rg_peak(&tag.value.to_string())
                }
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
    t
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
            let tags = read_tags(&path);
            let artist = tags.artist.unwrap_or_else(|| "Unknown Artist".to_string());
            let album = tags.album.unwrap_or_else(|| dir_album.clone());
            let akey = format!("{artist}|{album}");
            out.push(LibTrack {
                path,
                title: tags.title.unwrap_or_else(|| pretty_title(&fname)),
                artist,
                album,
                genre: tags.genre.unwrap_or_else(|| "Unknown Genre".to_string()),
                track_no: tags.track_no,
                art_path: art_path.clone(),
                akey,
                rg_track_gain: tags.rg_track_gain,
                rg_track_peak: tags.rg_track_peak,
                rg_album_gain: tags.rg_album_gain,
                rg_album_peak: tags.rg_album_peak,
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

/// Apply canonical album/artist/track-title to every track sharing `akey`.
/// Empty canonical fields fall back to the existing tag/filename value (so a
/// source that returns no name leaves the raw value untouched). Track titles are
/// matched by canonical order — the album's tracks sorted by tag track-number
/// (else path) zipped against the source tracklist. `akey` itself never changes
/// (it stays the tag-derived cover-cache key). Does NOT recompute groupings.
fn apply_album_meta(s: &mut State, akey: &str, album: &str, artist: &str, tracks: &[String]) -> bool {
    let mut idxs: Vec<usize> =
        s.library.iter().enumerate().filter(|(_, t)| t.akey == akey).map(|(i, _)| i).collect();
    idxs.sort_by(|&a, &b| {
        let ta = s.library[a].track_no.unwrap_or(u32::MAX);
        let tb = s.library[b].track_no.unwrap_or(u32::MAX);
        ta.cmp(&tb).then_with(|| s.library[a].path.cmp(&s.library[b].path))
    });
    let mut changed = false;
    for (n, &li) in idxs.iter().enumerate() {
        let t = &mut s.library[li];
        if !album.is_empty() && t.album != album {
            t.album = album.to_string();
            changed = true;
        }
        if !artist.is_empty() && t.artist != artist {
            t.artist = artist.to_string();
            changed = true;
        }
        if let Some(title) = tracks.get(n) {
            if !title.is_empty() && &t.title != title {
                t.title = title.clone();
                changed = true;
            }
        }
    }
    changed
}

/// After names changed for `akey`: recompute the groupings (preserving the
/// drilled group by name across the re-sort), refresh the now-playing display if
/// the current track is in this album, and mark rows dirty.
fn regroup_after_meta(s: &mut State, akey: &str) {
    let drilled_name = s.drill.and_then(|g| groups_for(s).get(g).map(|(n, _)| n.clone()));
    s.albums = group_by(&s.library, |t| &t.album);
    s.artists = group_by(&s.library, |t| &t.artist);
    s.genres = group_by(&s.library, |t| &t.genre);
    if let Some(name) = drilled_name {
        s.drill = groups_for(s).iter().position(|(n, _)| *n == name);
    }
    if let Some(cur) = cur_lib_index(s) {
        if s.library[cur].akey == akey {
            let e = s.library[cur].clone();
            publish_metadata(s, &e);
            s.meta_dirty = true;
        }
    }
    s.rows_dirty = true;
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
fn out_layout_stereo(channels: usize) -> bool {
    channels >= 2
}

/// ReplayGain dB → linear factor with peak clip-protection (cap so gain*peak ≤ 1).
fn rg_factor(gain_db: Option<f32>, peak: Option<f32>) -> f32 {
    match gain_db {
        Some(g) => {
            let mut f = 10f32.powf(g / 20.0);
            if let Some(p) = peak {
                if p > 0.0 && f * p > 1.0 {
                    f = 1.0 / p;
                }
            }
            f.clamp(0.0, 4.0)
        }
        None => 1.0,
    }
}

fn queue_is_single_album(s: &State) -> bool {
    let mut it = s.queue.iter();
    let Some(&first) = it.next() else { return false };
    let a = &s.library[first].akey;
    it.all(|&i| &s.library[i].akey == a)
}

/// Album gain when playing one album in order (preserves intra-album dynamics),
/// else track gain. Falls back across scopes; absent tags → 1.0 (no-op).
fn compute_rg_gain(s: &State, e: &LibTrack) -> f32 {
    let album_mode = !s.shuffle && queue_is_single_album(s);
    if album_mode && e.rg_album_gain.is_some() {
        rg_factor(e.rg_album_gain, e.rg_album_peak)
    } else {
        rg_factor(e.rg_track_gain.or(e.rg_album_gain), e.rg_track_peak.or(e.rg_album_peak))
    }
}

/// Open a track's decoder (pure — no playback/UI state mutation beyond the art
/// cache, which it doesn't touch). Returns the decode head.
fn open_track(s: &State, lib_index: usize) -> Option<Loaded> {
    let entry = s.library.get(lib_index)?;
    let file = std::fs::File::open(&entry.path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext_of(&entry.path));
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions { enable_gapless: true, ..Default::default() },
            &MetadataOptions::default(),
        )
        .ok()?;
    let format = probed.format;
    let track = format.default_track()?.clone();
    let track_id = track.id;
    let src_rate = track.codec_params.sample_rate.unwrap_or(OUT_RATE);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);
    let n_frames = track.codec_params.n_frames;
    let decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;
    let total_frames = n_frames
        .map(|nf| (nf as u128 * OUT_RATE as u128 / src_rate.max(1) as u128) as u64)
        .unwrap_or(0);
    let resampler =
        if src_rate != OUT_RATE { Some(LinearResampler::new(src_rate, channels)) } else { None };
    let gain = compute_rg_gain(s, entry);
    Some(Loaded {
        format,
        decoder,
        track_id,
        channels,
        resampler,
        pending: Vec::new(),
        pending_pos: 0,
        eof: false,
        total_frames,
        gain,
        base: 0,
        seg_started: false,
        decoded: 0,
        eq_z: vec![[0.0, 0.0]; 2 * EQ_BANDS],
    })
}

/// Hard transition to `order_pos` (user skip / explicit jump / first load):
/// flush the device, drop decode-ahead, start fresh. Reopens the device only if
/// the new track's output layout differs.
fn hard_load(s: &mut State, order_pos: usize, autoplay: bool) {
    if order_pos >= s.order.len() {
        return;
    }
    s.order_pos = order_pos;
    s.decode_order = order_pos;
    let lib = s.order[order_pos];
    s.loaded = open_track(s, lib);
    s.segments.clear();
    s.xfade = None;
    s.gapless_blocked = false;
    s.ended = false;
    s.sw_frames = 0;
    s.rows_dirty = true;
    s.meta_dirty = true;
    if s.pb.is_some() {
        let new_stereo = s.loaded.as_ref().map(|l| out_layout_stereo(l.channels)).unwrap_or(s.pb_stereo);
        if new_stereo != s.pb_stereo {
            s.pb = None; // play() reopens with the right layout
        } else if let Some(pb) = s.pb.as_ref() {
            pb.flush();
        }
    }
    scope_reset(s);
    eq_reset_state(s);
    if autoplay {
        play(s);
    }
    after_change_publish(s);
}

/// Gapless continue: open the next track as the new decode head WITHOUT touching
/// the device, so its samples are written straight after the current track's
/// tail. Returns false if the next track can't continue on the open device
/// (open failed or output layout differs → caller falls back to hard_load).
fn soft_advance(s: &mut State, next_order: usize) -> bool {
    let Some(&lib) = s.order.get(next_order) else { return false };
    let Some(loaded) = open_track(s, lib) else { return false };
    if out_layout_stereo(loaded.channels) != s.pb_stereo {
        return false;
    }
    s.loaded = Some(loaded);
    s.decode_order = next_order;
    true
}

/// Create the audible `Seg` for the current decode head right before its first
/// sample is written (so dev_start is the device frame where it becomes audible).
fn ensure_seg(s: &mut State) {
    if !matches!(&s.loaded, Some(l) if !l.seg_started) {
        return;
    }
    let Some(pb) = s.pb.as_ref() else { return };
    let dev_start = pb.position() + pb.buffered_frames() as u64;
    let (total, base) = {
        let l = s.loaded.as_ref().unwrap();
        (l.total_frames, l.base)
    };
    let order_pos = s.decode_order;
    let lib_index = s.order.get(order_pos).copied().unwrap_or(0);
    s.segments.push_back(Seg { order_pos, lib_index, total_frames: total, dev_start, base });
    if let Some(l) = s.loaded.as_mut() {
        l.seg_started = true;
    }
}

/// The next order index the decode head should advance to, or None at the end.
fn next_decode_order(s: &State) -> Option<usize> {
    let from = s.decode_order;
    if from + 1 < s.order.len() {
        Some(from + 1)
    } else if s.repeat && !s.order.is_empty() {
        Some(0)
    } else {
        None
    }
}

/// Pop fully-played front segments so segments.front() is the audible track;
/// returns true if the audible track changed.
fn advance_audible(s: &mut State) -> bool {
    let played = s.pb.as_ref().map(|p| p.position()).unwrap_or(0);
    let mut changed = false;
    while s.segments.len() > 1 && played >= s.segments[1].dev_start {
        s.segments.pop_front();
        changed = true;
    }
    if let Some(front) = s.segments.front() {
        s.order_pos = front.order_pos;
    }
    changed
}

fn cur_total(s: &State) -> u64 {
    s.segments
        .front()
        .map(|f| f.total_frames)
        .or_else(|| s.loaded.as_ref().map(|l| l.total_frames))
        .unwrap_or(0)
}

// ── Spectrum visualizer ──────────────────────────────────────────────────────
fn scope_reset(s: &mut State) {
    if s.scope.len() == SCOPE_CAP {
        for x in &mut s.scope {
            *x = 0.0;
        }
    }
    s.scope_written = 0;
    s.open_dev = s.pb.as_ref().map(|p| p.position()).unwrap_or(0);
}

/// FFT the audible window (aligned to playback by compensating the device buffer
/// depth) and fold it into NUM_BARS log-spaced bands, each mapped dBFS → 0..1.
fn compute_spectrum(s: &State) -> Vec<f32> {
    let zero = vec![0.0; NUM_BARS];
    let (Some(fft), Some(pb), true) = (s.fft.as_ref(), s.pb.as_ref(), s.scope.len() == SCOPE_CAP)
    else {
        return zero;
    };
    // Audible "now" = frames the device has PLAYED since open. The scope is
    // indexed by frames written to the device, so this window is what's heard.
    let end = pb.position().saturating_sub(s.open_dev).min(s.scope_written);
    let buffered = s.scope_written.saturating_sub(end);
    if end < FFT_SIZE as u64 || buffered + FFT_SIZE as u64 > SCOPE_CAP as u64 {
        return zero;
    }
    let start = end - FFT_SIZE as u64;
    let mut buf: Vec<rustfft::num_complex::Complex<f32>> = (0..FFT_SIZE)
        .map(|i| {
            let v = s.scope[((start as usize) + i) % SCOPE_CAP] * s.hann[i];
            rustfft::num_complex::Complex { re: v, im: 0.0 }
        })
        .collect();
    fft.process(&mut buf);

    let bins = FFT_SIZE / 2;
    let mut bars = vec![0.0f32; NUM_BARS];
    let ratio = VIZ_HI_HZ / VIZ_LO_HZ;
    for (k, bar) in bars.iter_mut().enumerate() {
        let f_lo = VIZ_LO_HZ * ratio.powf(k as f32 / NUM_BARS as f32);
        let f_hi = VIZ_LO_HZ * ratio.powf((k + 1) as f32 / NUM_BARS as f32);
        let b_lo = ((f_lo * FFT_SIZE as f32 / OUT_RATE as f32) as usize).clamp(1, bins - 1);
        let b_hi = ((f_hi * FFT_SIZE as f32 / OUT_RATE as f32) as usize).clamp(b_lo + 1, bins);
        let mut acc = 0.0f32;
        for b in b_lo..b_hi {
            acc += buf[b].norm();
        }
        let mag = acc / (b_hi - b_lo) as f32 / FFT_SIZE as f32;
        let db = 20.0 * (mag + 1e-9).log10();
        *bar = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    }
    bars
}

/// Update smoothed bars: instant attack, fast decay (peak-style fall-off, tuned
/// for the ~60 Hz visualizer cadence). Only animates while the now-playing view
/// is visible and playing; otherwise flattens (no continuous redraw when idle).
fn update_bars(s: &mut State) {
    if s.bars.len() != NUM_BARS {
        s.bars = vec![0.0; NUM_BARS];
    }
    if s.playing && s.view == 1 {
        let raw = compute_spectrum(s);
        for i in 0..NUM_BARS {
            let prev = s.bars[i];
            s.bars[i] = if raw[i] > prev { raw[i] } else { prev * 0.78 + raw[i] * 0.22 };
        }
    } else {
        for v in &mut s.bars {
            *v = 0.0;
        }
    }
}

/// Decode one packet of a single track into `l.pending` — resample → ReplayGain
/// → EQ (per-stream filter state, so a crossfade pair filters independently).
fn decode_chunk(l: &mut Loaded, eq_coeffs: &[[f32; 5]], eq_active: bool) -> bool {
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
                let mut out = match l.resampler.as_mut() {
                    Some(r) => r.process(sb.samples()),
                    None => sb.samples().to_vec(),
                };
                if l.gain != 1.0 {
                    let g = l.gain;
                    for x in out.iter_mut() {
                        *x = (*x * g).clamp(-1.0, 1.0);
                    }
                }
                let ch = l.channels.max(1);
                if eq_active {
                    eq_apply(eq_coeffs, &mut l.eq_z, &mut out, ch);
                }
                l.decoded += (out.len() / ch) as u64;
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

fn decode_more(s: &mut State) -> bool {
    let eq_active = s.eq_active;
    let coeffs = &s.eq_coeffs;
    match s.loaded.as_mut() {
        Some(l) => decode_chunk(l, coeffs, eq_active),
        None => false,
    }
}

/// Start a crossfade once the decode head reaches `total - dur` (and there's a
/// next track of matching layout). No-op when crossfade is off or near no track.
fn maybe_start_xfade(s: &mut State) {
    if s.xfade_secs <= 0.0 || s.xfade.is_some() {
        return;
    }
    let (total, decoded) = match &s.loaded {
        Some(l) => (l.total_frames, l.decoded),
        None => return,
    };
    if total == 0 {
        return;
    }
    let dur = (s.xfade_secs * OUT_RATE as f32) as u64;
    if dur == 0 || decoded < total.saturating_sub(dur) {
        return;
    }
    let Some(no) = next_decode_order(s) else { return };
    let Some(mut next) = open_track(s, s.order[no]) else { return };
    if out_layout_stereo(next.channels) != s.pb_stereo {
        return; // layout change → let it end / hard-switch instead
    }
    // Flip the now-playing display to the incoming track at the START of the blend
    // (its position counts up from 0:00 through the overlap and beyond), so push
    // its segment here and mark it started so the post-promotion ensure_seg skips.
    let dev_start = {
        let pb = s.pb.as_ref().unwrap();
        pb.position() + pb.buffered_frames() as u64
    };
    next.seg_started = true;
    s.segments.push_back(Seg {
        order_pos: no,
        lib_index: s.order[no],
        total_frames: next.total_frames,
        dev_start,
        base: 0,
    });
    s.xfade = Some(Xfade { next, order: no, pos: 0, dur });
}

/// Promote the incoming track to the decode head when the blend completes.
fn xfade_finish(s: &mut State) {
    let Some(x) = s.xfade.take() else { return };
    // The incoming segment was already pushed (flip-at-start); just promote it to
    // the decode head and continue normally.
    s.decode_order = x.order;
    s.order_pos = x.order;
    s.loaded = Some(x.next); // seg_started already true → ensure_seg won't re-push
}

/// Mix the outgoing + incoming tracks (equal-power) and write the blend.
fn pump_xfade(s: &mut State) {
    loop {
        // Ensure both sides have decoded samples.
        if { let l = s.loaded.as_ref().unwrap(); l.pending_pos >= l.pending.len() }
            && !s.loaded.as_ref().unwrap().eof
        {
            decode_more(s);
        }
        if { let x = s.xfade.as_ref().unwrap(); x.next.pending_pos >= x.next.pending.len() }
            && !s.xfade.as_ref().unwrap().next.eof
        {
            let eq_active = s.eq_active;
            let coeffs = &s.eq_coeffs;
            let x = s.xfade.as_mut().unwrap();
            decode_chunk(&mut x.next, coeffs, eq_active);
        }
        let ch = s.loaded.as_ref().unwrap().channels.max(1);
        let la = { let l = s.loaded.as_ref().unwrap(); (l.pending.len() - l.pending_pos) / ch };
        let na = { let x = s.xfade.as_ref().unwrap(); (x.next.pending.len() - x.next.pending_pos) / ch };
        let (pos, dur) = { let x = s.xfade.as_ref().unwrap(); (x.pos, x.dur) };
        let remaining = dur.saturating_sub(pos);

        // Blend done, or the outgoing track ran out → promote the incoming.
        if remaining == 0 || (s.loaded.as_ref().unwrap().eof && la == 0) {
            xfade_finish(s);
            return;
        }
        let avail = la.min(na);
        if avail == 0 {
            if s.xfade.as_ref().unwrap().next.eof && na == 0 {
                xfade_finish(s); // incoming was shorter than the blend
            }
            break;
        }
        let n = avail.min(remaining as usize);
        let mut mix = vec![0.0f32; n * ch];
        {
            let l = s.loaded.as_ref().unwrap();
            let x = s.xfade.as_ref().unwrap();
            let (lp, np) = (l.pending_pos, x.next.pending_pos);
            for f in 0..n {
                let t = (pos + f as u64) as f32 / dur as f32;
                let og = (t * std::f32::consts::FRAC_PI_2).cos();
                let ig = (t * std::f32::consts::FRAC_PI_2).sin();
                for c in 0..ch {
                    mix[f * ch + c] = l.pending[lp + f * ch + c] * og + x.next.pending[np + f * ch + c] * ig;
                }
            }
        }
        let accepted = { s.pb.as_ref().unwrap().write(&mix[..]) as usize };
        if accepted == 0 {
            break; // device ring full
        }
        if s.scope.len() == SCOPE_CAP {
            for f in 0..accepted {
                let mut m = 0.0;
                for c in 0..ch {
                    m += mix[f * ch + c];
                }
                s.scope[(s.scope_written as usize + f) % SCOPE_CAP] = m / ch as f32;
            }
            s.scope_written += accepted as u64;
        }
        {
            let l = s.loaded.as_mut().unwrap();
            l.pending_pos += accepted * ch;
            if l.pending_pos >= l.pending.len() {
                l.pending.clear();
                l.pending_pos = 0;
            }
        }
        {
            let x = s.xfade.as_mut().unwrap();
            x.next.pending_pos += accepted * ch;
            if x.next.pending_pos >= x.next.pending.len() {
                x.next.pending.clear();
                x.next.pending_pos = 0;
            }
            x.pos += accepted as u64;
        }
    }
}

fn pump(s: &mut State) {
    if s.pb.is_none() || s.loaded.is_none() {
        return;
    }
    if s.xfade.is_none() {
        maybe_start_xfade(s);
    }
    if s.xfade.is_some() {
        pump_xfade(s);
        return;
    }
    loop {
        let has_pending = {
            let l = s.loaded.as_ref().unwrap();
            l.pending_pos < l.pending.len()
        };
        if has_pending {
            ensure_seg(s);
            let ch = s.loaded.as_ref().unwrap().channels.max(1);
            let old_pos = s.loaded.as_ref().unwrap().pending_pos;
            let accepted = {
                let l = s.loaded.as_ref().unwrap();
                let pb = s.pb.as_ref().unwrap();
                pb.write(&l.pending[l.pending_pos..]) as usize
            };
            // Tap exactly the frames accepted by the device into the visualizer
            // scope ring (written-frame coords, so it aligns with pb.position()).
            if accepted > 0 && s.scope.len() == SCOPE_CAP {
                let l = s.loaded.as_ref().unwrap();
                for f in 0..accepted {
                    let base = old_pos + f * ch;
                    let mut m = 0.0;
                    for c in 0..ch {
                        m += l.pending[base + c];
                    }
                    s.scope[(s.scope_written as usize + f) % SCOPE_CAP] = m / ch as f32;
                }
                s.scope_written += accepted as u64;
            }
            let l = s.loaded.as_mut().unwrap();
            l.pending_pos += accepted * ch;
            if l.pending_pos >= l.pending.len() {
                l.pending.clear();
                l.pending_pos = 0;
            }
            if accepted == 0 {
                break;
            }
        } else if !s.loaded.as_ref().unwrap().eof {
            // refill from the decode head (sets eof if exhausted; loop re-checks)
            decode_more(s);
            // crossfade may now be due (decoded reached total - dur)
            maybe_start_xfade(s);
            if s.xfade.is_some() {
                pump_xfade(s);
                return;
            }
        } else {
            // decode head fully drained — try to continue gaplessly into the next
            // track without stopping the device.
            if s.gapless_blocked {
                break;
            }
            match next_decode_order(s) {
                Some(n) => {
                    if !soft_advance(s, n) {
                        s.gapless_blocked = true; // layout change → engine reopens at drain
                        break;
                    }
                }
                None => break,
            }
        }
    }
}

fn position_frames(s: &State) -> u64 {
    if let Some(pb) = &s.pb {
        if let Some(sg) = s.segments.front() {
            let p = sg.base + pb.position().saturating_sub(sg.dev_start);
            return if sg.total_frames > 0 { p.min(sg.total_frames) } else { p };
        }
    }
    s.sw_frames
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
                s.pb = Some(pb);
                s.pb_stereo = out_layout_stereo(ch);
                s.gapless_blocked = false;
                s.segments.clear();
                scope_reset(s);
                if let Some(l) = s.loaded.as_mut() {
                    l.seg_started = false; // pump's ensure_seg anchors the first segment
                }
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
    // Seeks act on the AUDIBLE track; if the decode head ran ahead (gapless
    // window), re-sync it to the audible track first.
    if let Some(front) = s.segments.front().cloned() {
        if s.decode_order != front.order_pos {
            if let Some(l) = open_track(s, front.lib_index) {
                s.loaded = Some(l);
                s.decode_order = front.order_pos;
            }
        }
    }
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
        l.base = target;
        l.decoded = target; // decode position now starts at the seek target
        l.seg_started = false;
        if let Some(r) = l.resampler.as_mut() {
            r.reset();
        }
        target
    };
    s.segments.clear();
    s.xfade = None;
    s.gapless_blocked = false;
    s.sw_frames = target;
    s.ended = false;
    if let Some(pb) = s.pb.as_ref() {
        pb.flush();
    }
    scope_reset(s);
    eq_reset_state(s);
    pump(s);
    if s.playing {
        if let Some(pb) = s.pb.as_ref() {
            let _ = pb.start();
        }
    }
}

/// The currently AUDIBLE library index (front segment, else the decode head).
fn cur_lib_index(s: &State) -> Option<usize> {
    if let Some(sg) = s.segments.front() {
        return Some(sg.lib_index);
    }
    s.order.get(s.order_pos).copied()
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
/// Keeps in-flight segments + the decode head pointing at their tracks under the
/// new ordering, so reshuffling mid-playback only affects what comes next.
fn apply_order(s: &mut State, keep_lib: Option<usize>) {
    let decode_lib = s.order.get(s.decode_order).copied();
    if s.shuffle {
        let mut v = s.queue.clone();
        fisher_yates(&mut v, &mut s.rng);
        s.order = v;
    } else {
        s.order = s.queue.clone();
    }
    s.segments.retain(|sg| s.order.contains(&sg.lib_index));
    for sg in &mut s.segments {
        if let Some(p) = s.order.iter().position(|&k| k == sg.lib_index) {
            sg.order_pos = p;
        }
    }
    s.order_pos = if let Some(front) = s.segments.front() {
        front.order_pos
    } else if let Some(lib) = keep_lib {
        s.order.iter().position(|&k| k == lib).unwrap_or(0)
    } else {
        0
    };
    s.decode_order = decode_lib
        .and_then(|dl| s.order.iter().position(|&k| k == dl))
        .unwrap_or(s.order_pos);
}

/// Start playing from a browsed track list (becomes the queue).
fn play_from(s: &mut State, tracks: Vec<usize>, tapped: usize) {
    if tracks.is_empty() {
        return;
    }
    let keep = tracks[tapped.min(tracks.len() - 1)];
    s.queue = tracks;
    apply_order(s, Some(keep));
    hard_load(s, s.order_pos, true);
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
    if s.loaded.is_none() && s.segments.is_empty() {
        return;
    }
    let total = cur_total(s);
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
        // Visualizer: allocate the scope ring, FFT plan, Hann window, band state.
        s.scope = vec![0.0; SCOPE_CAP];
        s.bars = vec![0.0; NUM_BARS];
        s.fft = Some(rustfft::FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE));
        s.hann = (0..FFT_SIZE)
            .map(|i| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE as f32 - 1.0)).cos()
            })
            .collect();
        // Graphic EQ: persisted gains/enable → coeffs.
        s.eq_gains = vec![0.0; EQ_BANDS];
        load_eq(s);
        eq_recalc(s);
        wandr_step_executor::init(); // Phase A — async metadata-fetch reactor
        s.library = scan_library();
        // Phase A — per distinct album (akey): load the cached cover + cached
        // canonical names (applied before grouping so the library reflects them),
        // and queue a fetch for whatever is still missing. One sequential task
        // does the network work (rate-limit-friendly).
        let mut seen = HashSet::new();
        let mut to_fetch = Vec::new();
        for i in 0..s.library.len() {
            let akey = s.library[i].akey.clone();
            if !seen.insert(akey.clone()) {
                continue;
            }
            if let Some(dec) = load_cached_cover(&akey) {
                s.album_art.insert(akey.clone(), dec);
            }
            let meta = load_cached_meta(&akey);
            if let Some((al, ar, tr)) = &meta {
                apply_album_meta(s, &akey, al, ar, tr);
            }
            let have_cover = s.album_art.contains_key(&akey);
            let have_meta = meta.is_some();
            if !have_cover || !have_meta {
                let t = &s.library[i];
                to_fetch.push((akey.clone(), t.artist.clone(), t.album.clone(), have_cover));
            }
        }
        s.albums = group_by(&s.library, |t| &t.album);
        s.artists = group_by(&s.library, |t| &t.artist);
        s.genres = group_by(&s.library, |t| &t.genre);
        s.queue = (0..s.library.len()).collect();
        s.order = s.queue.clone();
        s.rows_dirty = true;
        spawn_library_fetch(to_fetch);
        if !s.library.is_empty() {
            hard_load(s, 0, false);
        }
    }
    if s.loaded.is_none() && s.segments.is_empty() {
        return 1000;
    }

    // Flip the audible track when playback crosses a gapless boundary.
    let flipped = advance_audible(s);

    if s.playing {
        pump(s);
    }

    update_bars(s); // spectrum visualizer (FFT only when now-playing + playing)

    // End / layout-change transition: only fires once the device ring has fully
    // drained AND the decode head is exhausted (gapless keeps the ring fed across
    // same-layout boundaries, so this is reached only at the real end or when the
    // next track needs a device reopen).
    if s.playing && s.xfade.is_none() {
        let head_eof =
            s.loaded.as_ref().map(|l| l.eof && l.pending_pos >= l.pending.len()).unwrap_or(true);
        let ring_empty = s.pb.as_ref().map(|p| p.buffered_frames() == 0).unwrap_or(true);
        if head_eof && ring_empty {
            match next_decode_order(s) {
                Some(n) => hard_load(s, n, true), // layout change → reopen device
                None => {
                    s.pb = None;
                    s.playing = false;
                    s.ended = true;
                    s.sw_frames = cur_total(s);
                    s.segments.clear();
                    after_change_publish(s);
                }
            }
        }
    }

    if flipped {
        s.meta_dirty = true;
        s.rows_dirty = true;
        if let Some(lib) = cur_lib_index(s) {
            let e = s.library[lib].clone();
            publish_metadata(s, &e);
        }
        publish_state(s);
        s.last_pub_sec = -1;
    }

    if s.pub_playing != s.playing {
        publish_state(s);
        s.pub_playing = s.playing;
    }
    let total = cur_total(s);
    let pos = position_frames(s).min(if total > 0 { total } else { u64::MAX });
    let cur_sec = (pos / OUT_RATE as u64) as i64;
    if cur_sec != s.last_pub_sec {
        publish_position(s);
        s.last_pub_sec = cur_sec;
    }

    // ~60 Hz while the spectrum is on screen (snappy bars), ~30 Hz otherwise
    // while playing (audio ring), idle when stopped.
    if s.playing {
        if s.view == 1 { 16 } else { 33 }
    } else {
        500
    }
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
            // now-playing fields — derived from the AUDIBLE library entry.
            let disp_lib = if s.loaded.is_some() || !s.segments.is_empty() {
                cur_lib_index(&s)
            } else {
                None
            };
            let (title, subtitle, total) = match disp_lib {
                Some(lib) => {
                    let t = &s.library[lib];
                    (t.title.clone(), format!("{} — {}", t.artist, t.album), cur_total(&s))
                }
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
            // Visualizer bars: update the persistent model IN PLACE every frame
            // while animating (only the heights re-evaluate → partial redraw); on
            // stop, flatten once so we don't spin the renderer idle.
            let viz_active = s.playing && s.view == 1 && s.bars.len() == NUM_BARS;
            if viz_active {
                BARS_MODEL.with(|m| {
                    for (i, &v) in s.bars.iter().enumerate() {
                        m.set_row_data(i, v);
                    }
                });
                if !s.bars_were_active {
                    // Entering the visualizer (off a tap → render context): start the
                    // frame pump so the host keeps rendering at ~60 fps.
                    VIZ_TIMER.with(|t| {
                        t.start(slint::TimerMode::Repeated, Duration::from_millis(16), || {
                            UI.with(|u| {
                                if let Some(ui) = u.borrow().as_ref() {
                                    ui.window().request_redraw();
                                }
                            });
                        });
                    });
                }
                ui.window().request_redraw();
                s.bars_were_active = true;
            } else if s.bars_were_active {
                BARS_MODEL.with(|m| {
                    for i in 0..NUM_BARS {
                        m.set_row_data(i, 0.0);
                    }
                });
                VIZ_TIMER.with(|t| t.stop());
                ui.window().request_redraw();
                s.bars_were_active = false;
            }
            ui.set_playing(s.playing);
            ui.set_shuffle(s.shuffle);
            ui.set_repeat(s.repeat);
            ui.set_eq_enabled(s.eq_enabled);
            ui.set_eq_gains(ModelRc::from(Rc::new(VecModel::from(s.eq_gains.clone()))));
            ui.set_xfade_label(
                if s.xfade_secs <= 0.0 { "Off".to_string() } else { format!("{}s", s.xfade_secs as i32) }
                    .into(),
            );
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
                // Prefer a fetched/cached internet cover (by the audible album's
                // akey), else the local albumart file.
                let art = match disp_lib {
                    Some(lib) => {
                        let ak = s.library[lib].akey.clone();
                        let ap = s.library[lib].art_path.clone();
                        match s.album_art.get(&ak).cloned() {
                            Some(a) => Some(a),
                            None => load_art(&mut s, &ap),
                        }
                    }
                    None => None,
                };
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
        let total = cur_total(&s);
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
        let total = cur_total(&s);
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
        hard_load(&mut s, np, true);
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
            hard_load(&mut s, pp, true);
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
fn cmd_open_eq() {
    STATE.with(|st| st.borrow_mut().view = 2);
    push_ui();
}
fn cmd_close_eq() {
    STATE.with(|st| st.borrow_mut().view = 1);
    push_ui();
}
fn cmd_eq_toggle() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        s.eq_enabled = !s.eq_enabled;
        eq_recalc(&mut s);
        save_eq(&s);
    });
    push_ui();
}
fn cmd_eq_set_band(band: usize, gain_db: f32) {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        if band < EQ_BANDS {
            s.eq_gains[band] = gain_db.clamp(-EQ_RANGE_DB, EQ_RANGE_DB);
            eq_recalc(&mut s);
            save_eq(&s);
        }
    });
    push_ui();
}
fn cmd_eq_flat() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        for g in &mut s.eq_gains {
            *g = 0.0;
        }
        eq_recalc(&mut s);
        save_eq(&s);
    });
    push_ui();
}
fn cmd_cycle_xfade() {
    STATE.with(|st| {
        let mut s = st.borrow_mut();
        // Off → 2 → 4 → 6 → 8 → Off
        s.xfade_secs = match s.xfade_secs as i32 {
            0 => 2.0,
            2 => 4.0,
            4 => 6.0,
            6 => 8.0,
            _ => 0.0,
        };
        save_eq(&s);
    });
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
    id: String,            // source album id (mbid / deezer id / itunes collectionId)
    tracks: Vec<String>,   // canonical tracklist if the first response carried it
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

fn safe_key(akey: &str) -> String {
    akey.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}
fn cache_file(akey: &str) -> String {
    format!("{CACHE_DIR}/{}.img", safe_key(akey))
}
fn meta_file(akey: &str) -> String {
    format!("{CACHE_DIR}/{}.json", safe_key(akey))
}

fn load_cached_cover(akey: &str) -> Option<(Vec<u8>, u32, u32)> {
    let bytes = std::fs::read(cache_file(akey)).ok()?;
    decode_rgba(&bytes)
}

fn save_cached_cover(akey: &str, bytes: &[u8]) {
    let _ = std::fs::create_dir_all(CACHE_DIR);
    let _ = std::fs::write(cache_file(akey), bytes);
}

/// Persist the canonical album/artist/tracklist so relaunch shows real names
/// without hitting the network (covers are cached too → the fetch is skipped).
fn save_cached_meta(akey: &str, album: &str, artist: &str, tracks: &[String]) {
    let _ = std::fs::create_dir_all(CACHE_DIR);
    let v = serde_json::json!({ "album": album, "artist": artist, "tracks": tracks });
    let _ = std::fs::write(meta_file(akey), v.to_string());
}

fn load_cached_meta(akey: &str) -> Option<(String, String, Vec<String>)> {
    let body = std::fs::read_to_string(meta_file(akey)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let album = v.get("album").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let artist = v.get("artist").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let tracks = v
        .get("tracks")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    Some((album, artist, tracks))
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
    if let Some(s) = cur.as_str() {
        s.to_string()
    } else if let Some(n) = cur.as_u64() {
        n.to_string()
    } else if let Some(n) = cur.as_i64() {
        n.to_string()
    } else {
        String::new()
    }
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
        id: mbid,
        tracks: Vec::new(),
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
    Cand {
        name: jstr(d, &["title"]),
        artist: jstr(d, &["artist", "name"]),
        cover,
        id: jstr(d, &["id"]),
        tracks: Vec::new(),
        src: "deezer",
    }
}

/// iTunes Search (no key) → artworkUrl (upscaled to 600×600).
async fn lookup_itunes(client: &reqwest::Client, artist: &str, album: &str) -> Cand {
    let term = q_encode(&format!("{artist} {album}"));
    let url = format!("https://itunes.apple.com/search?term={term}&entity=album&limit=3");
    let Some(v) = get_json(client, &url).await else { return Cand::default() };
    let Some(r) = v.get("results").and_then(|r| r.get(0)) else { return Cand::default() };
    let cover = jstr(r, &["artworkUrl100"]).replace("100x100bb", "600x600bb");
    Cand {
        name: jstr(r, &["collectionName"]),
        artist: jstr(r, &["artistName"]),
        cover,
        id: jstr(r, &["collectionId"]),
        tracks: Vec::new(),
        src: "itunes",
    }
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
    // album.getinfo carries the tracklist inline (track[].name, in order).
    let tracks = v
        .get("album")
        .and_then(|a| a.get("tracks"))
        .and_then(|t| t.get("track"))
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .map(|t| t.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    Cand {
        name: jstr(&v, &["album", "name"]),
        artist: jstr(&v, &["album", "artist"]),
        cover,
        id: String::new(),
        tracks,
        src: "lastfm",
    }
}

/// Fetch the canonical tracklist for the chosen candidate. Uses the inline list
/// if the first response already carried it (Last.fm), else one follow-up call
/// keyed by the source album id. Empty on any failure (titles then fall back).
async fn fetch_tracklist(client: &reqwest::Client, c: &Cand) -> Vec<String> {
    if !c.tracks.is_empty() {
        return c.tracks.clone();
    }
    if c.id.is_empty() {
        return Vec::new();
    }
    match c.src {
        "musicbrainz" => {
            let url = format!("https://musicbrainz.org/ws/2/release/{}?inc=recordings&fmt=json", c.id);
            let Some(v) = get_json(client, &url).await else { return Vec::new() };
            let mut out = Vec::new();
            if let Some(media) = v.get("media").and_then(|m| m.as_array()) {
                for md in media {
                    if let Some(tr) = md.get("tracks").and_then(|t| t.as_array()) {
                        for t in tr {
                            out.push(t.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string());
                        }
                    }
                }
            }
            out
        }
        "deezer" => {
            let url = format!("https://api.deezer.com/album/{}", c.id);
            let Some(v) = get_json(client, &url).await else { return Vec::new() };
            v.get("tracks")
                .and_then(|t| t.get("data"))
                .and_then(|d| d.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|t| t.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string())
                        .collect()
                })
                .unwrap_or_default()
        }
        "itunes" => {
            let url = format!("https://itunes.apple.com/lookup?id={}&entity=song", c.id);
            let Some(v) = get_json(client, &url).await else { return Vec::new() };
            v.get("results")
                .and_then(|r| r.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter(|x| x.get("wrapperType").and_then(|w| w.as_str()) == Some("track"))
                        .map(|t| t.get("trackName").and_then(|x| x.as_str()).unwrap_or("").to_string())
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// One album's multi-source lookup: query MusicBrainz → Deezer → iTunes →
/// Last.fm, log each, return the first candidate that yielded a cover (its name/
/// artist/tracklist are then used as the canonical metadata).
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
fn spawn_library_fetch(albums: Vec<(String, String, String, bool)>) {
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
        for (akey, artist, album, have_cover) in albums {
            if let Some(c) = fetch_one(&client, &artist, &album).await {
                // Canonical names + tracklist (fall back to raw tags/filenames
                // where a source returned nothing). Persist + apply in-memory.
                let tracks = fetch_tracklist(&client, &c).await;
                log1(&format!("[meta]   tracklist [{}]: {} title(s)", c.src, tracks.len()));
                save_cached_meta(&akey, &c.name, &c.artist, &tracks);
                let changed = STATE.with(|st| {
                    let mut s = st.borrow_mut();
                    let ch = apply_album_meta(&mut s, &akey, &c.name, &c.artist, &tracks);
                    if ch {
                        regroup_after_meta(&mut s, &akey);
                    }
                    ch
                });
                if changed {
                    push_ui();
                }
                // Cover (skip if we already had it cached).
                if !have_cover {
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
    ui.on_open_eq(cmd_open_eq);
    ui.on_close_eq(cmd_close_eq);
    ui.on_eq_toggle(cmd_eq_toggle);
    ui.on_eq_flat(cmd_eq_flat);
    ui.on_eq_set_band(|b, g| cmd_eq_set_band(b as usize, g));
    ui.on_cycle_xfade(cmd_cycle_xfade);
    BARS_MODEL.with(|m| ui.set_bars(ModelRc::from(m.clone())));
    ui.set_eq_range(EQ_RANGE_DB);
    ui.set_eq_labels(ModelRc::from(Rc::new(VecModel::from(
        EQ_FREQS
            .iter()
            .map(|&f| {
                if f >= 1000.0 {
                    format!("{}k", (f / 1000.0).round() as i32)
                } else {
                    format!("{}", f.round() as i32)
                }
                .into()
            })
            .collect::<Vec<slint::SharedString>>(),
    ))));
    UI.with(|u| *u.borrow_mut() = Some(ui.clone_strong()));
    push_ui();
    ui.show().expect("audio-player: show");
    ui
});
