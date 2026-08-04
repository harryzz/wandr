//! wandr.navidrome — a Subsonic/OpenSubsonic (Navidrome) music client, Slint UI.
//!
//! Canvas→Slint migration (task 120): density-correct Slint UI (crates/slint-wandr) over
//! the HEADLESS `wandr-media-engine` audio core (stream open → symphonia demux+decode →
//! wasi:audio + master clock + seek; device-audio fix `95c1e588`). The engine does NOT
//! render — bg-tick drives it and pushes state into Slint properties.
//!
//! UI is a navigation STACK of list screens (standard music-player IA):
//!   Menu → { Albums · Artists · Playlists · Songs · Search }
//!   Albums → Album (tracklist)     Artists → Artist (albums) → Album
//!   Playlists → Playlist (tracks)  Songs → flat song list     Search → mixed
//! Tapping a SONG builds a play QUEUE from that screen's songs and starts playback;
//! the back chevron pops the stack. A persistent now-playing bar sits at the bottom.
//!
//! COVER-ART POLICY (perf + convention): art is ALBUM-CENTRIC — shown on Album rows, the
//! album/artist/playlist detail HEADER, and now-playing. A flat Songs list is TEXT ONLY
//! (a library can hold thousands of songs; one fetch per row would stall the main thread).
//! Covers update PER-ROW (`set_row_data`), never a full-model rebuild, so scrolling/return
//! from the player stays smooth; the list only repaints at 60fps on the now-playing screen.
//!
//! CONFIG is read from `/state/navidrome/config.json` (server/user/pass) — never baked in.
//! All network I/O (connect, browse fetches, stream probe, cover art) is async (spawned on
//! the reqwest executor, advanced by the host p3 store loop); sync Slint callbacks only
//! mutate state and hand work to bg-tick.
#![allow(clippy::too_many_arguments)]

use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use wandr_media_engine as engine;

use opensubsonic::data::Child;
use opensubsonic::{AlbumListType, Auth, Client};
use serde::Deserialize;

// ── Slint UI (density-correct: slint-wandr dispatches ScaleFactorChanged from
//    wandr:ui-shell/metrics.get-density, so every `px` scales to the panel) ─────
slint::slint! {
    import { ListView, LineEdit } from "std-widgets.slint";

    // A generic browse row. `kind`: 0=menu 1=album 2=artist 3=playlist 4=song.
    // `show-art` reserves the cover column (albums only); songs/menus are text.
    struct Item {
        kind: int,
        title: string,
        subtitle: string,
        trailing: string,
        show-art: bool,
        has-art: bool,
        art: image,
        current: bool,
    }

    // No-cover placeholder — a vinyl-record shape (DRAWN, not a font glyph: the ♪ glyph
    // U+266A is absent from the Android sans font and renders as a "NO GLYPH" tofu).
    component CoverPlaceholder inherits Rectangle {
        Rectangle {
            width: parent.width * 0.56; height: self.width; border-radius: self.width / 2;
            background: #2b3a48;
            Rectangle { width: parent.width * 0.16; height: self.width; border-radius: self.width / 2; background: #0b0d10; }
        }
    }

    export component MainWindow inherits Window {
        background: #0b0d10;
        in property <int> view: 0;            // 0 = browse, 1 = now-playing
        // browse — audio.player-style tabs + drill breadcrumb
        in property <int> tab: 0;             // 0 Albums · 1 Artists · 2 Playlists · 3 Songs
        in property <bool> in-drill: false;   // showing a drilled screen (tracklist / artist albums)
        in property <string> crumb: "";       // drilled screen title
        in property <string> status: "connecting…";
        in property <bool> header-has-cover: false;
        in property <image> header-cover;
        in property <[Item]> rows: [];
        in property <bool> searching: false;
        // now-playing / mini-bar
        in property <string> np-title: "—";
        in property <string> np-sub: "";
        in property <string> elapsed: "0:00";
        in property <string> total: "0:00";
        in property <string> qpos: "";
        in property <float> progress: 0.0;
        in property <bool> playing: false;
        in property <bool> opening: false;
        in property <image> np-cover;
        in property <bool> np-has-cover: false;
        callback row-tap(int);
        callback set-tab(int);
        callback back();
        callback open-search();
        callback close-search();
        callback search-submit(string);
        callback toggle();
        callback prev-track();
        callback next-track();
        callback seek(float);
        callback open-np();
        callback close-np();

        property <color> accent: #4ac0ff;
        property <color> dim: #8a9098;

        // ── Browse view ──────────────────────────────────────────────────────
        if (root.view == 0) : Rectangle {
            width: 100%; height: 100%;
            VerticalLayout {
                padding: 12px; spacing: 8px;

                // Tab bar (Albums / Artists / Playlists / Songs) + a search shortcut.
                HorizontalLayout {
                    spacing: 6px; height: 42px;
                    for t[i] in [ "Albums", "Artists", "Playlists", "Songs" ] : Rectangle {
                        horizontal-stretch: 1; border-radius: 8px;
                        background: root.tab == i ? #16202a : transparent;
                        Text { text: t; font-size: 14px; font-weight: root.tab == i ? 700 : 400;
                            horizontal-alignment: center; vertical-alignment: center;
                            color: root.tab == i ? accent : #c8ccd2; }
                        TouchArea { clicked => { root.set-tab(i); } }
                    }
                    Rectangle {
                        width: 40px;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 44 20 A 24 24 0 1 0 44 68 A 24 24 0 1 0 44 20 M 62 62 L 82 82";
                            stroke: #8a9098; stroke-width: 8px; fill: transparent; }
                        TouchArea { clicked => { root.open-search(); } }
                    }
                }

                // Breadcrumb + back (only when drilled into a tracklist / artist / playlist).
                if root.in-drill : Rectangle {
                    height: 46px;
                    HorizontalLayout {
                        spacing: 10px;
                        Rectangle {
                            width: 34px; height: 34px; y: (parent.height - self.height)/2;
                            Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                                commands: "M 62 24 L 36 50 L 62 76"; stroke: white; stroke-width: 9px; fill: transparent; }
                            TouchArea { clicked => { root.back(); } }
                        }
                        if root.header-has-cover : Rectangle {
                            width: 40px; height: 40px; y: (parent.height - self.height)/2;
                            border-radius: 6px; clip: true; background: #16202a;
                            Image { width: 100%; height: 100%; source: root.header-cover; image-fit: ImageFit.cover; }
                        }
                        Text { text: root.crumb; color: white; font-size: 19px; font-weight: 700;
                            vertical-alignment: center; horizontal-stretch: 1; overflow: elide; }
                    }
                }
                Text { text: root.status; color: dim; font-size: 12px; overflow: elide; }

                // The current screen's rows.
                ListView {
                    vertical-stretch: 1;
                    for it[i] in root.rows : Rectangle {
                        height: it.show-art ? 66px : 58px;
                        background: it.current ? #16202a : transparent;
                        border-radius: 8px;
                        HorizontalLayout {
                            padding: 8px; spacing: 12px;
                            if it.show-art : Rectangle {
                                width: 50px; height: 50px; border-radius: 6px; clip: true; background: #16202a;
                                y: (parent.height - self.height)/2;
                                if it.has-art : Image { width: 100%; height: 100%; source: it.art; image-fit: ImageFit.cover; }
                                if !it.has-art : CoverPlaceholder { width: 100%; height: 100%; }
                            }
                            VerticalLayout {
                                alignment: center; horizontal-stretch: 1;
                                Text { text: it.title; color: it.current ? accent : white; font-size: 16px; overflow: elide; }
                                if it.subtitle != "" : Text { text: it.subtitle; color: dim; font-size: 12px; overflow: elide; }
                            }
                            if it.trailing != "" : Text { text: it.trailing; color: dim; font-size: 12px; vertical-alignment: center; }
                            if it.kind < 4 : Rectangle {
                                width: 14px; height: 14px; y: (parent.height - self.height)/2;
                                Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                                    commands: "M 38 24 L 64 50 L 38 76"; stroke: #55606a; stroke-width: 9px; fill: transparent; }
                            }
                        }
                        TouchArea { clicked => { root.row-tap(i); } }
                    }
                }

                // Mini now-playing bar → tap to open the full player.
                if (root.np-title != "—") : Rectangle {
                    height: 62px; border-radius: 12px; background: #14181f;
                    TouchArea { clicked => { root.open-np(); } }
                    HorizontalLayout {
                        padding: 10px; spacing: 10px;
                        Rectangle {
                            width: 42px; height: 42px; border-radius: 6px; clip: true; background: #24243a;
                            if root.np-has-cover : Image { width: 100%; height: 100%; source: root.np-cover; image-fit: ImageFit.cover; }
                            if !root.np-has-cover : CoverPlaceholder { width: 100%; height: 100%; }
                        }
                        VerticalLayout {
                            alignment: center; horizontal-stretch: 1;
                            Text { text: root.np-title; color: white; font-size: 14px; overflow: elide; }
                            Text { text: root.np-sub; color: dim; font-size: 11px; overflow: elide; }
                        }
                        Rectangle {
                            width: 44px; height: 44px; border-radius: 22px; background: accent;
                            if !root.playing : Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                                commands: "M 38 26 L 74 50 L 38 74 Z"; fill: white; }
                            if root.playing : Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                                commands: "M 35 28 L 45 28 L 45 72 L 35 72 Z M 55 28 L 65 28 L 65 72 L 55 72 Z"; fill: white; }
                            TouchArea { clicked => { root.toggle(); } }
                        }
                    }
                }
            }

            // Search overlay.
            if root.searching : Rectangle {
                width: 100%; height: 100%; background: #d0000000;
                TouchArea { clicked => { root.close-search(); } }
                Rectangle {
                    width: 88%; height: 60px; y: parent.height * 0.22;
                    se := LineEdit {
                        width: 100%; height: 100%; font-size: 18px;
                        placeholder-text: "Search artists, albums, songs…";
                        accepted(t) => { root.search-submit(t); }
                    }
                }
            }
        }

        // ── Now-playing view ─────────────────────────────────────────────────
        if (root.view == 1) : Rectangle {
            width: 100%; height: 100%;
            property <length> art-size: min(root.width, root.height) * 0.46;
            VerticalLayout {
                padding: root.width * 0.07; spacing: root.height * 0.02; alignment: center;

                HorizontalLayout {
                    Rectangle {
                        width: 32px; height: 32px;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 24 40 L 50 66 L 76 40"; stroke: white; stroke-width: 9px; fill: transparent; }
                        TouchArea { clicked => { root.close-np(); } }
                    }
                    Rectangle { }
                    Text { text: root.qpos; color: dim; font-size: 13px; vertical-alignment: center; }
                }

                HorizontalLayout {
                    alignment: center;
                    Rectangle {
                        width: art-size; height: art-size; border-radius: art-size * 0.06;
                        clip: true; background: #16202a;
                        if root.np-has-cover : Image { width: 100%; height: 100%; source: root.np-cover; image-fit: ImageFit.cover; }
                        if !root.np-has-cover : CoverPlaceholder { width: 100%; height: 100%; }
                    }
                }

                Text { text: root.np-title; color: white; font-size: 22px; font-weight: 700;
                    horizontal-alignment: center; overflow: elide; }
                Text { text: root.np-sub; color: #b0b4bc; font-size: 14px;
                    horizontal-alignment: center; overflow: elide; }
                Text { text: root.opening ? "opening…" : ""; color: dim; font-size: 12px; horizontal-alignment: center; }

                prog := Rectangle {
                    height: 20px;
                    property <bool> dragging: false;
                    property <float> drag-frac: 0.0;
                    property <float> shown: dragging ? drag-frac : root.progress;
                    Rectangle { width: 100%; height: 5px; y: (parent.height - self.height)/2; border-radius: 2px; background: #2a3038; }
                    Rectangle { width: parent.width * prog.shown; height: 5px; y: (parent.height - self.height)/2; border-radius: 2px; background: accent; }
                    Rectangle { width: 16px; height: 16px; border-radius: 8px; background: white;
                        x: prog.shown * (parent.width - self.width); y: (parent.height - self.height)/2; }
                    TouchArea {
                        moved => { prog.drag-frac = clamp(self.mouse-x / self.width, 0.0, 1.0); prog.dragging = true; }
                        pointer-event(ev) => {
                            if (ev.kind == PointerEventKind.down) { prog.drag-frac = clamp(self.mouse-x / self.width, 0.0, 1.0); prog.dragging = true; }
                            if (ev.kind == PointerEventKind.up) { root.seek(prog.drag-frac); prog.dragging = false; }
                        }
                    }
                }
                HorizontalLayout {
                    Text { text: root.elapsed; color: #b0b4bc; font-size: 12px; }
                    Rectangle { }
                    Text { text: root.total; color: #b0b4bc; font-size: 12px; }
                }

                HorizontalLayout {
                    alignment: center; spacing: root.width * 0.08;
                    Rectangle {
                        width: 44px; height: 44px;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 68 24 L 40 50 L 68 76 Z M 34 24 L 40 24 L 40 76 L 34 76 Z"; fill: white; }
                        TouchArea { clicked => { root.prev-track(); } }
                    }
                    Rectangle {
                        width: 66px; height: 66px; border-radius: 33px; background: accent;
                        if !root.playing : Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 38 26 L 74 50 L 38 74 Z"; fill: white; }
                        if root.playing : Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 35 28 L 45 28 L 45 72 L 35 72 Z M 55 28 L 65 28 L 65 72 L 55 72 Z"; fill: white; }
                        TouchArea { clicked => { root.toggle(); } }
                    }
                    Rectangle {
                        width: 44px; height: 44px;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100;
                            commands: "M 32 24 L 60 50 L 32 76 Z M 60 24 L 66 24 L 66 76 L 60 76 Z"; fill: white; }
                        TouchArea { clicked => { root.next-track(); } }
                    }
                }
                Rectangle { } // bottom spacer
            }
        }
    }
}

// ── config (read from the per-app /state preopen, NOT baked in) ───────────────
const STATE_DIR: &str = "/state/navidrome";
const CONFIG_PATH: &str = "/state/navidrome/config.json";
const CONFIG_TEMPLATE: &str = r#"{
  "server": "https://music.example.com",
  "user": "youruser",
  "pass": "yourpassword"
}
"#;

/// Max concurrent cover-art fetches (album-scoped, so this is plenty; covers stream in).
const MAX_COVER_INFLIGHT: usize = 4;

#[derive(Default, Clone, Deserialize)]
struct Config {
    #[serde(default)]
    server: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    pass: String,
}

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

// ── navigation model ─────────────────────────────────────────────────────────

/// A top-level browse destination (the Menu entries; also what a fetch targets).
#[derive(Clone, Copy, PartialEq)]
enum Dest {
    Albums,
    Artists,
    Playlists,
    Songs,
    Search,
}
impl Dest {
    fn label(self) -> &'static str {
        match self {
            Dest::Albums => "Albums",
            Dest::Artists => "Artists",
            Dest::Playlists => "Playlists",
            Dest::Songs => "Songs",
            Dest::Search => "Search…",
        }
    }
}

/// One row in a browse screen.
#[derive(Clone)]
enum Row {
    Menu(Dest),
    Album { id: String, name: String, sub: String, cover: Option<String> },
    Artist { id: String, name: String },
    Playlist { id: String, name: String, count: i64 },
    Song(Child),
}

/// A pushed browse screen. `cover` is the detail header artwork id (album/artist/playlist).
struct Screen {
    title: String,
    rows: Vec<Row>,
    cover: Option<String>,
}

/// A deferred network fetch, queued by a tap and run in bg-tick.
#[derive(Clone)]
enum Nav {
    Dest(Dest),
    Album(String),
    Artist(String),
    Playlist(String),
    Search(String),
}

/// Connection / browse state (the status line + initial load). INDEPENDENT of playback:
/// browsing to another tab must never disturb a song that is playing.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Loading,
    Ready,
    Error,
}

/// Playback state — owns the audio cadence. Kept separate from `Phase` so a tab switch
/// (which sets `Phase::Ready`) can't drop the bg-tick rate and starve the audio ring.
#[derive(Clone, Copy, PartialEq)]
enum Play {
    Idle,
    Opening,
    Playing,
}

// ── app state ────────────────────────────────────────────────────────────────

struct State {
    phase: Phase,
    play: Play,
    driver_spawned: bool,
    client: Option<Client>,
    /// Navigation stack; `stack.last()` is the visible screen.
    stack: Vec<Screen>,
    pending_nav: Option<Nav>,
    busy: bool,
    searching: bool,
    tab: i32,  // active browse tab: 0 Albums · 1 Artists · 2 Playlists · 3 Songs
    view: i32, // 0 = browse, 1 = now-playing
    // playback queue --------------------------------------------------------
    queue: Vec<Child>,
    qidx: usize,
    pending_play: Option<usize>,
    pending_open: Option<(String, u64, String, i64)>,
    last: String,
    // cover art (album-scoped) ---------------------------------------------
    covers: HashMap<String, Image>,
    cover_inflight: HashSet<String>,
    pending_covers: Vec<(String, Vec<u8>)>,
    // ui bookkeeping -------------------------------------------------------
    /// Rebuild the WHOLE row model (screen changed). Cover arrivals do NOT set this —
    /// they patch individual rows via set_row_data (the anti-jank rule).
    model_dirty: bool,
    /// A discrete visible change happened off the render clock → redraw once.
    ui_dirty: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        phase: Phase::Loading,
        play: Play::Idle,
        driver_spawned: false,
        client: None,
        stack: Vec::new(),
        pending_nav: None,
        busy: false,
        searching: false,
        tab: 0,
        view: 0,
        queue: Vec::new(),
        qidx: 0,
        pending_play: None,
        pending_open: None,
        last: "connecting…".into(),
        covers: HashMap::new(),
        cover_inflight: HashSet::new(),
        pending_covers: Vec::new(),
        model_dirty: false,
        ui_dirty: true,
    });
    static UI: RefCell<Option<MainWindow>> = const { RefCell::new(None) };
    /// The live row model — patched per-row on cover arrival, replaced on screen change.
    static ROWS: RefCell<Option<Rc<VecModel<Item>>>> = const { RefCell::new(None) };
    /// Monotonic origin for the playback clock (wasip2 std → wasi:clocks; no WIT import).
    static ORIGIN: Instant = Instant::now();
}

fn set_phase(p: Phase) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.phase = p;
        s.ui_dirty = true;
    });
}
fn set_play(p: Play) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.play = p;
        s.ui_dirty = true;
    });
}
fn note(msg: impl Into<String>) {
    let m = msg.into();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.last = m.clone();
        s.ui_dirty = true;
    });
    engine::log(m);
}
fn client() -> Option<Client> {
    STATE.with(|s| s.borrow().client.clone())
}
fn probe_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("wandr-navidrome/0.1 ( https://github.com/harryzz/wandr )")
        .build()
        .ok()
}
fn fmt_secs(sec: i64) -> String {
    let s = sec.max(0);
    format!("{}:{:02}", s / 60, s % 60)
}

/// Push a freshly-fetched screen and return to browsing.
fn push_screen(s: Screen) {
    STATE.with(|a| {
        let mut a = a.borrow_mut();
        a.stack.push(s);
        a.busy = false;
        a.phase = Phase::Ready;
        a.model_dirty = true;
        a.ui_dirty = true;
    });
}
fn fail(msg: impl Into<String>) {
    note(msg);
    STATE.with(|a| {
        let mut a = a.borrow_mut();
        a.busy = false;
        a.phase = if a.stack.is_empty() { Phase::Error } else { Phase::Ready };
    });
}

// ── async driver + fetches ───────────────────────────────────────────────────

/// Connect, then show the top-level menu.
async fn driver() {
    let cfg = match load_config() {
        Ok(c) => c,
        Err(e) => {
            note(e);
            return set_phase(Phase::Error);
        }
    };
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
    STATE.with(|a| {
        let mut a = a.borrow_mut();
        a.client = Some(client);
        // Land on the Albums tab (cover-art forward, like audio.player defaults to Albums).
        a.tab = 0;
        a.stack.clear();
        a.pending_nav = Some(Nav::Dest(Dest::Albums));
        a.busy = true;
        a.phase = Phase::Ready;
        a.model_dirty = true;
        a.ui_dirty = true;
        a.last = "loading albums…".into();
    });
}

fn dest_for_tab(t: i32) -> Dest {
    match t {
        0 => Dest::Albums,
        1 => Dest::Artists,
        2 => Dest::Playlists,
        _ => Dest::Songs,
    }
}

/// Switch browse tab: reset the stack to that section's root and (re)fetch it.
fn set_tab(t: i32) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.tab = t;
        s.stack.clear();
        s.searching = false;
        s.busy = true;
        s.last = "loading…".into();
        s.model_dirty = true;
        s.ui_dirty = true;
        s.pending_nav = Some(Nav::Dest(dest_for_tab(t)));
    });
    request_redraw();
}

fn album_row(a: opensubsonic::data::AlbumId3) -> Row {
    let mut sub = a.artist.clone().unwrap_or_default();
    if let Some(y) = a.year {
        if !sub.is_empty() {
            sub.push_str("  ·  ");
        }
        sub.push_str(&y.to_string());
    }
    Row::Album { id: a.id, name: a.name, sub, cover: a.cover_art }
}

/// Run the fetch behind a `Nav` and push the resulting screen.
async fn fetch(nav: Nav) {
    let Some(c) = client() else { return fail("not connected") };
    match nav {
        Nav::Dest(Dest::Albums) => match c
            .get_album_list2(AlbumListType::AlphabeticalByName, Some(200), None, None, None, None, None)
            .await
        {
            Ok(albums) => push_screen(Screen {
                title: "Albums".into(),
                cover: None,
                rows: albums.into_iter().map(album_row).collect(),
            }),
            Err(e) => fail(format!("albums: {e}")),
        },
        Nav::Dest(Dest::Artists) => match c.get_artists(None).await {
            Ok(idx) => {
                let rows: Vec<Row> = idx
                    .index
                    .into_iter()
                    .flat_map(|i| i.artist)
                    .map(|ar| Row::Artist { id: ar.id, name: ar.name })
                    .collect();
                push_screen(Screen { title: "Artists".into(), cover: None, rows });
            }
            Err(e) => fail(format!("artists: {e}")),
        },
        Nav::Dest(Dest::Playlists) => match c.get_playlists(None).await {
            Ok(pls) => {
                let rows: Vec<Row> = pls
                    .into_iter()
                    .map(|p| Row::Playlist { id: p.id, name: p.name, count: p.song_count.unwrap_or(0) })
                    .collect();
                push_screen(Screen { title: "Playlists".into(), cover: None, rows });
            }
            Err(e) => fail(format!("playlists: {e}")),
        },
        Nav::Dest(Dest::Songs) => match c.get_random_songs(Some(200), None, None, None, None).await {
            Ok(songs) => push_screen(Screen {
                title: "Songs".into(),
                cover: None,
                rows: songs.into_iter().map(Row::Song).collect(),
            }),
            Err(e) => fail(format!("songs: {e}")),
        },
        Nav::Dest(Dest::Search) => {} // opened via the search overlay
        Nav::Album(id) => match c.get_album(&id).await {
            Ok(al) => push_screen(Screen {
                title: al.name,
                cover: al.cover_art,
                rows: al.song.into_iter().map(Row::Song).collect(),
            }),
            Err(e) => fail(format!("album: {e}")),
        },
        Nav::Artist(id) => match c.get_artist(&id).await {
            Ok(ar) => push_screen(Screen {
                title: ar.name,
                cover: ar.cover_art,
                rows: ar.album.into_iter().map(album_row).collect(),
            }),
            Err(e) => fail(format!("artist: {e}")),
        },
        Nav::Playlist(id) => match c.get_playlist(&id).await {
            Ok(pl) => push_screen(Screen {
                title: pl.name,
                cover: pl.cover_art,
                rows: pl.entry.into_iter().map(Row::Song).collect(),
            }),
            Err(e) => fail(format!("playlist: {e}")),
        },
        Nav::Search(q) => match c.search3(&q, Some(20), None, Some(20), None, Some(80), None, None).await {
            Ok(r) => {
                let mut rows: Vec<Row> = Vec::new();
                rows.extend(r.artist.into_iter().map(|a| Row::Artist { id: a.id, name: a.name }));
                rows.extend(r.album.into_iter().map(album_row));
                rows.extend(r.song.into_iter().map(Row::Song));
                if rows.is_empty() {
                    note(format!("no results for \"{q}\""));
                }
                push_screen(Screen { title: format!("“{q}”"), cover: None, rows });
            }
            Err(e) => fail(format!("search: {e}")),
        },
    }
}

/// Build the stream URL for `queue[idx]`, probe its size, hand off to bg-tick.
async fn play(idx: usize) {
    let Some((song, c)) = STATE.with(|s| {
        let s = s.borrow();
        s.queue.get(idx).cloned().zip(s.client.clone())
    }) else {
        return set_play(Play::Idle);
    };
    let url = match c.stream_url(&song.id, None, Some("raw")) {
        Ok(u) => u.to_string(),
        Err(e) => {
            note(format!("stream url: {e}"));
            return set_play(Play::Idle);
        }
    };
    let Some(hc) = probe_client() else {
        note("no HTTP client");
        return set_play(Play::Idle);
    };
    let total_len = match engine::net::fetch_range(&hc, &url, 0, Some(0)).await {
        Ok(r) if r.total_len > 0 => r.total_len,
        Ok(_) => {
            note("stream: server did not report a length");
            return set_play(Play::Idle);
        }
        Err(e) => {
            note(format!("stream probe: {e}"));
            return set_play(Play::Idle);
        }
    };
    let title = format!("{} — {}", song.artist.clone().unwrap_or_default(), song.title);
    let dur_us = song.duration.unwrap_or(0).max(0) * 1_000_000;
    STATE.with(|s| s.borrow_mut().pending_open = Some((url, total_len, title, dur_us)));
}

async fn fetch_cover(id: String) {
    let Some(c) = client() else { return };
    match c.get_cover_art(&id, Some(400)).await {
        Ok(bytes) => STATE.with(|s| s.borrow_mut().pending_covers.push((id, bytes.to_vec()))),
        Err(_) => STATE.with(|s| {
            s.borrow_mut().cover_inflight.remove(&id);
        }),
    }
}

// ── tap handling (Slint callbacks, same UI thread as bg-tick) ─────────────────

/// Act on a tap of row `i` in the current screen: drill in, or play.
fn row_tap(i: usize) {
    let nav = STATE.with(|a| {
        let a_ref = a.borrow();
        let Some(scr) = a_ref.stack.last() else { return None };
        let Some(row) = scr.rows.get(i) else { return None };
        match row.clone() {
            Row::Menu(Dest::Search) => {
                drop(a_ref);
                a.borrow_mut().searching = true;
                a.borrow_mut().ui_dirty = true;
                None
            }
            Row::Menu(d) => Some(Nav::Dest(d)),
            Row::Album { id, .. } => Some(Nav::Album(id)),
            Row::Artist { id, .. } => Some(Nav::Artist(id)),
            Row::Playlist { id, .. } => Some(Nav::Playlist(id)),
            Row::Song(_) => {
                // Build a queue from every Song on this screen; start at the tapped one.
                let songs: Vec<Child> = scr
                    .rows
                    .iter()
                    .filter_map(|r| if let Row::Song(c) = r { Some(c.clone()) } else { None })
                    .collect();
                let start = scr.rows[..=i]
                    .iter()
                    .filter(|r| matches!(r, Row::Song(_)))
                    .count()
                    .saturating_sub(1);
                drop(a_ref);
                start_queue(a, songs, start);
                None
            }
        }
    });
    if let Some(n) = nav {
        STATE.with(|a| {
            let mut a = a.borrow_mut();
            a.pending_nav = Some(n);
            a.busy = true;
            a.last = "loading…".into();
            a.ui_dirty = true;
        });
    }
}

fn start_queue(a: &RefCell<State>, queue: Vec<Child>, idx: usize) {
    if queue.is_empty() {
        return;
    }
    let mut a = a.borrow_mut();
    a.qidx = idx.min(queue.len() - 1);
    a.queue = queue;
    a.pending_play = Some(a.qidx);
    a.model_dirty = true; // refresh the playing-row highlight
    a.ui_dirty = true;
}

fn go_back() {
    STATE.with(|a| {
        let mut a = a.borrow_mut();
        if a.stack.len() > 1 {
            a.stack.pop();
            a.model_dirty = true;
            a.ui_dirty = true;
        }
    });
}

fn skip(delta: i64) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.queue.is_empty() {
            return;
        }
        let n = s.queue.len() as i64;
        let next = (s.qidx as i64 + delta).clamp(0, n - 1);
        if next != s.qidx as i64 {
            s.qidx = next as usize;
            s.pending_play = Some(s.qidx);
            s.model_dirty = true;
            s.ui_dirty = true;
        }
    });
}

fn cmd_toggle() {
    engine::CONTROLS.with(|c| {
        let mut c = c.borrow_mut();
        c.controls_bump = true;
        c.paused = !c.paused;
    });
    STATE.with(|s| s.borrow_mut().ui_dirty = true);
}

fn cmd_seek_frac(frac: f32) {
    let dur = engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.duration_us()).unwrap_or(0));
    if dur > 0 {
        let target = (frac.clamp(0.0, 1.0) as f64 * dur as f64) as i64;
        engine::CONTROLS.with(|c| {
            let mut c = c.borrow_mut();
            c.controls_bump = true;
            c.seek_request = Some(target);
        });
    }
}

fn set_view(v: i32) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.view = v;
        s.ui_dirty = true;
    });
    request_redraw();
}

fn open_search() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.searching = true;
        s.ui_dirty = true;
    });
    request_redraw();
}
fn close_search() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.searching = false;
        s.ui_dirty = true;
    });
    request_redraw();
}
fn search_submit(q: &str) {
    let q = q.trim().to_string();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.searching = false;
        s.ui_dirty = true;
        if !q.is_empty() {
            s.pending_nav = Some(Nav::Search(q));
            s.busy = true;
            s.last = "searching…".into();
        }
    });
    request_redraw();
}

fn request_redraw() {
    UI.with(|u| {
        if let Some(ui) = u.borrow().as_ref() {
            ui.window().request_redraw();
        }
    });
}

fn teardown_stream() {
    engine::STREAM.with(|s| *s.borrow_mut() = None);
    let _ = engine::with_audio(|pb| pb.flush());
}

// ── the engine pump — runs in bg-tick (foreground AND background) ─────────────

fn engine_tick() -> u32 {
    if STATE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.driver_spawned {
            s.driver_spawned = true;
            true
        } else {
            false
        }
    }) {
        reqwest::task::spawn(driver());
    }

    // Queued browse fetch.
    if let Some(nav) = STATE.with(|a| a.borrow_mut().pending_nav.take()) {
        reqwest::task::spawn(fetch(nav));
    }

    // Queued (re)open of a queue track.
    if let Some(idx) = STATE.with(|s| s.borrow_mut().pending_play.take()) {
        teardown_stream();
        set_play(Play::Opening);
        reqwest::task::spawn(play(idx));
    }

    if engine::CONTROLS.with(|c| std::mem::take(&mut c.borrow_mut().stop_requested)) {
        teardown_stream();
        set_play(Play::Idle);
    }

    if let Some((u, total, title, dur)) = STATE.with(|s| s.borrow_mut().pending_open.take()) {
        let surface = engine::CONTROLS.with(|c| c.borrow().surface);
        match engine::open_audio_sync(u, total, title, dur, surface) {
            Ok(()) => set_play(Play::Playing),
            Err(e) => {
                note(format!("open failed: {e}"));
                set_play(Play::Idle);
            }
        }
    }

    // Drive the open stream: seek → demux-fill → decode → clock/audio-write → prefetch.
    if engine::STREAM.with(|s| s.borrow().is_some()) {
        let nanos = ORIGIN.with(|o| o.elapsed().as_nanos() as u64);
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
        engine::pump_stream(nanos);
        if let Some(h) = engine::STREAM.with(|s| s.borrow().as_ref().and_then(|p| p.prefetch_handle())) {
            engine::drive_prefetch(&h);
        }

        if STATE.with(|s| s.borrow().play == Play::Playing)
            && engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.is_ended()).unwrap_or(false))
        {
            let advanced = STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.qidx + 1 < s.queue.len() {
                    s.qidx += 1;
                    s.pending_play = Some(s.qidx);
                    s.model_dirty = true;
                    s.ui_dirty = true;
                    true
                } else {
                    false
                }
            });
            if !advanced {
                teardown_stream();
                set_play(Play::Idle);
            }
        }
    }

    reconcile_covers();
    push_ui();

    // Cadence: playback OWNS it (16 ms keeps the audio ring fed) regardless of what the
    // user is browsing; only when nothing is playing does the browse phase set the rate.
    STATE.with(|s| {
        let s = s.borrow();
        if matches!(s.play, Play::Opening | Play::Playing) {
            16
        } else {
            match s.phase {
                Phase::Loading => 30,
                _ => 120,
            }
        }
    })
}

/// The cover-art ids the current view wants: album rows on the visible screen + that
/// screen's detail header + the now-playing song's album cover. NEVER song rows.
fn desired_covers(s: &State) -> Vec<String> {
    let mut want = Vec::new();
    // Now-playing first (prioritized).
    if let Some(id) = s.queue.get(s.qidx).and_then(|c| c.cover_art.clone()) {
        want.push(id);
    }
    if s.view == 0 {
        if let Some(scr) = s.stack.last() {
            if let Some(id) = scr.cover.clone() {
                want.push(id);
            }
            for r in &scr.rows {
                if let Row::Album { cover: Some(id), .. } = r {
                    want.push(id.clone());
                }
            }
        }
    }
    want
}

fn reconcile_covers() {
    // 1. Decode fetched bytes → Slint Image, then PATCH affected rows (no full rebuild).
    let pend: Vec<(String, Vec<u8>)> = STATE.with(|s| std::mem::take(&mut s.borrow_mut().pending_covers));
    for (id, bytes) in pend {
        match decode_rgba(&bytes) {
            Some((rgba, w, h)) => {
                let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
                buf.make_mut_bytes().copy_from_slice(&rgba);
                let img = Image::from_rgba8(buf);
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.covers.insert(id.clone(), img.clone());
                    s.cover_inflight.remove(&id);
                });
                patch_cover(&id);
            }
            None => STATE.with(|s| {
                s.borrow_mut().cover_inflight.remove(&id);
            }),
        }
    }

    // 2. Enqueue still-missing wanted covers (bounded).
    let to_fetch: Vec<String> = STATE.with(|s| {
        let s = s.borrow();
        let budget = MAX_COVER_INFLIGHT.saturating_sub(s.cover_inflight.len());
        if budget == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for id in desired_covers(&s) {
            if out.len() >= budget {
                break;
            }
            if !seen.insert(id.clone()) || s.covers.contains_key(&id) || s.cover_inflight.contains(&id) {
                continue;
            }
            out.push(id);
        }
        out
    });
    for id in to_fetch {
        STATE.with(|s| {
            s.borrow_mut().cover_inflight.insert(id.clone());
        });
        reqwest::task::spawn(fetch_cover(id));
    }
}

/// A cover with id `id` just landed — update only what references it: matching album
/// rows (per-row set_row_data), the header cover, and the now-playing cover.
fn patch_cover(id: &str) {
    UI.with(|u| {
        let b = u.borrow();
        let Some(ui) = b.as_ref() else { return };
        STATE.with(|st| {
            let s = st.borrow();
            let img = match s.covers.get(id) {
                Some(i) => i.clone(),
                None => return,
            };
            let playing_id = s.queue.get(s.qidx).and_then(|c| c.cover_art.clone());
            if playing_id.as_deref() == Some(id) {
                ui.set_np_cover(img.clone());
                ui.set_np_has_cover(true);
            }
            if s.view == 0 {
                if let Some(scr) = s.stack.last() {
                    if scr.cover.as_deref() == Some(id) {
                        ui.set_header_cover(img.clone());
                        ui.set_header_has_cover(true);
                    }
                    ROWS.with(|r| {
                        if let Some(model) = r.borrow().as_ref() {
                            for (i, row) in scr.rows.iter().enumerate() {
                                if let Row::Album { cover: Some(cid), .. } = row {
                                    if cid == id {
                                        model.set_row_data(i, item_for(row, &s.covers, playing_id.as_deref()));
                                    }
                                }
                            }
                        }
                    });
                }
            }
        });
        ui.window().request_redraw();
    });
}

fn decode_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((rgba.into_raw(), w, h))
}

// ── build a Slint Item from a Row ─────────────────────────────────────────────
fn item_for(row: &Row, covers: &HashMap<String, Image>, playing_cover: Option<&str>) -> Item {
    match row {
        Row::Menu(d) => Item {
            kind: 0,
            title: d.label().into(),
            subtitle: Default::default(),
            trailing: Default::default(),
            show_art: false,
            has_art: false,
            art: Image::default(),
            current: false,
        },
        Row::Album { name, sub, cover, .. } => {
            let art = cover.as_ref().and_then(|id| covers.get(id).cloned());
            Item {
                kind: 1,
                title: name.as_str().into(),
                subtitle: sub.as_str().into(),
                trailing: Default::default(),
                show_art: true,
                has_art: art.is_some(),
                art: art.unwrap_or_default(),
                current: false,
            }
        }
        Row::Artist { name, .. } => Item {
            kind: 2,
            title: name.as_str().into(),
            subtitle: Default::default(),
            trailing: Default::default(),
            show_art: false,
            has_art: false,
            art: Image::default(),
            current: false,
        },
        Row::Playlist { name, count, .. } => Item {
            kind: 3,
            title: name.as_str().into(),
            subtitle: format!("{count} songs").into(),
            trailing: Default::default(),
            show_art: false,
            has_art: false,
            art: Image::default(),
            current: false,
        },
        Row::Song(c) => {
            let d = c.duration.unwrap_or(0).max(0);
            // Song highlight (current track) is applied in push_ui by song-id match, not here.
            let _ = playing_cover;
            Item {
                kind: 4,
                title: c.title.as_str().into(),
                subtitle: format!(
                    "{} · {}",
                    c.artist.clone().unwrap_or_default(),
                    c.album.clone().unwrap_or_default()
                )
                .into(),
                trailing: fmt_secs(d).into(),
                show_art: false,
                has_art: false,
                art: Image::default(),
                current: false,
            }
        }
    }
}

// ── push app state → Slint properties ────────────────────────────────────────

fn push_ui() {
    UI.with(|u| {
        let b = u.borrow();
        let Some(ui) = b.as_ref() else { return };
        STATE.with(|st| {
            let mut s = st.borrow_mut();
            ui.set_view(s.view);
            ui.set_searching(s.searching);
            ui.set_status(s.last.as_str().into());
            ui.set_tab(s.tab);

            // Tabs vs. drill breadcrumb. in-drill = a screen pushed past the tab root.
            let in_drill = s.stack.len() > 1;
            ui.set_in_drill(in_drill);
            let (crumb, header_cover) = match s.stack.last() {
                Some(scr) if in_drill => (scr.title.clone(), scr.cover.clone()),
                _ => (String::new(), None),
            };
            ui.set_crumb(crumb.as_str().into());
            match header_cover.as_ref().and_then(|id| s.covers.get(id).cloned()) {
                Some(img) => {
                    ui.set_header_cover(img);
                    ui.set_header_has_cover(true);
                }
                None => ui.set_header_has_cover(false),
            }

            // Now-playing metadata.
            let playing_id = s.queue.get(s.qidx).map(|c| c.id.clone());
            let (np_title, np_sub, song_dur_us, np_cover_id) = match s.queue.get(s.qidx) {
                Some(c) => (
                    c.title.clone(),
                    format!("{} · {}", c.artist.clone().unwrap_or_default(), c.album.clone().unwrap_or_default()),
                    c.duration.unwrap_or(0).max(0) * 1_000_000,
                    c.cover_art.clone(),
                ),
                None => ("—".into(), String::new(), 0, None),
            };
            let has_track = !s.queue.is_empty();
            let (clock_us, total_us) = engine::STREAM.with(|s2| match s2.borrow().as_ref() {
                Some(p) => (p.clock_us(), p.duration_us().max(song_dur_us)),
                None => (0, song_dur_us),
            });
            ui.set_np_title(if has_track { np_title.as_str().into() } else { "—".into() });
            ui.set_np_sub(np_sub.as_str().into());
            ui.set_elapsed(fmt_secs(clock_us / 1_000_000).as_str().into());
            ui.set_total(fmt_secs(total_us / 1_000_000).as_str().into());
            ui.set_progress(if total_us > 0 { (clock_us as f32 / total_us as f32).clamp(0.0, 1.0) } else { 0.0 });
            ui.set_opening(matches!(s.play, Play::Opening));
            let is_playing = matches!(s.play, Play::Playing) && !engine::CONTROLS.with(|c| c.borrow().paused);
            ui.set_playing(is_playing);
            ui.set_qpos(if has_track { format!("{} / {}", s.qidx + 1, s.queue.len()) } else { String::new() }.into());
            match np_cover_id.as_ref().and_then(|id| s.covers.get(id).cloned()) {
                Some(img) => {
                    ui.set_np_cover(img);
                    ui.set_np_has_cover(true);
                }
                None => ui.set_np_has_cover(false),
            }

            // Row model — rebuilt ONLY on a screen change (model_dirty). Cover arrivals
            // patch rows individually (patch_cover), never here.
            if s.model_dirty {
                s.model_dirty = false;
                let items: Vec<Item> = match s.stack.last() {
                    Some(scr) => scr
                        .rows
                        .iter()
                        .map(|r| {
                            let mut it = item_for(r, &s.covers, None);
                            // Song highlight = the currently-playing track.
                            if it.kind == 4 {
                                if let (Row::Song(c), Some(pid)) = (r, playing_id.as_ref()) {
                                    if &c.id == pid {
                                        it.current = true;
                                    }
                                }
                            }
                            it
                        })
                        .collect(),
                    None => Vec::new(),
                };
                let model = Rc::new(VecModel::from(items));
                ui.set_rows(ModelRc::from(model.clone()));
                ROWS.with(|r| *r.borrow_mut() = Some(model));
            }

            // Redraw policy: animate (60fps) ONLY on the now-playing screen while playing;
            // on the browse list, repaint only on a discrete change (the anti-jank rule).
            let animating = is_playing && s.view == 1;
            if animating || matches!(s.play, Play::Opening) || s.ui_dirty {
                s.ui_dirty = false;
                ui.window().request_redraw();
            }
        });
    });
}

// ── WIT bindings (alongside slint_wandr::launch!) — bg-tick only ──────────────
mod bindings {
    slint_wandr::__wit_bindgen::generate!({
        path: "wit-p3",
        world: "navidrome-extras",
        generate_all,
        runtime_path: "::slint_wandr::__wit_bindgen::rt",
    });

    struct Extras;

    impl exports::wandr::background::background::Guest for Extras {
        async fn bg_tick() -> u32 {
            crate::engine_tick()
        }
    }

    export!(Extras);
}

// ── Slint launch ─────────────────────────────────────────────────────────────
slint_wandr::launch!(|| {
    let ui = MainWindow::new().expect("navidrome: create MainWindow");
    ui.on_row_tap(|i| row_tap(i as usize));
    ui.on_set_tab(set_tab);
    ui.on_back(go_back);
    ui.on_open_search(open_search);
    ui.on_close_search(close_search);
    ui.on_search_submit(|t| search_submit(&t));
    ui.on_toggle(cmd_toggle);
    ui.on_prev_track(|| skip(-1));
    ui.on_next_track(|| skip(1));
    ui.on_seek(cmd_seek_frac);
    ui.on_open_np(|| set_view(1));
    ui.on_close_np(|| set_view(0));
    UI.with(|u| *u.borrow_mut() = Some(ui.clone_strong()));
    push_ui();
    ui.show().expect("navidrome: show");
    ui
});
