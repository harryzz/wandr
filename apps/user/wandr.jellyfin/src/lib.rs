//! wandr.jellyfin — a Jellyfin DirectPlay streaming client, Slint UI (task 120).
//!
//! Data layer is the full typed `jellyfin-sdk` (vendored fork, decoupled onto
//! wandr-reqwest/wasi:tls). First-run pairing (Quick Connect) is the one bridge the SDK
//! lacks (see pairing.rs); once paired the saved token drives the SDK directly.
//!
//! UI (Slint over slint-wandr → density-correct): tabs Home · Movies · Shows. Movies and
//! Shows are 2-column POSTER grids; Shows drills Series → Seasons → Episodes. A playable
//! (Movie/Episode) opens a DETAIL page (backdrop + overview + Play/Resume) → the video
//! player. VIDEO composites BEHIND the transparent Slint window via wandr:video
//! ZLayer::BehindUi; the pump (`pump_stream` submit + present-at-ns) runs on the render
//! path via slint_wandr::on_render_frame (host present clock) with continuous render
//! forced while playing; demux/decode/seek/session-reporting run in bg-tick.
#![allow(clippy::too_many_arguments)]

mod pairing;
use pairing::Session;

use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use wandr_media_engine as engine;

use jellyfin_sdk::api::{ItemImageRequest, ItemsQuery, LatestMediaQuery, ResumeItemsQuery};
use jellyfin_sdk::models::{
    BaseItemKind, ImageFormat, ImageType, ItemSortBy, PlayMethod, PlaybackInfoRequest,
    PlaybackProgress, PlaybackStart, PlaybackStop, SortOrder,
};
use jellyfin_sdk::JellyfinClient;
use uuid::Uuid;

// ── Slint UI ─────────────────────────────────────────────────────────────────
slint::slint! {
    import { ListView } from "std-widgets.slint";

    // A grid/list cell. kind: 0 Movie · 1 Series · 2 Season · 3 Episode.
    struct Cell { id: string, name: string, sub: string, kind: int, has-art: bool, art: image, valid: bool }
    // A 2-wide grid row.
    struct GridRow { a: Cell, b: Cell }

    component PosterPlaceholder inherits Rectangle {
        background: #16202a;
        Rectangle { width: parent.width * 0.5; height: parent.height * 0.3; border-radius: 4px; background: #2b3a48; }
    }

    // One poster cell (2:3) with a title under it.
    component Poster inherits Rectangle {
        in property <Cell> cell;
        callback tapped();
        VerticalLayout {
            spacing: 4px;
            Rectangle {
                height: self.width * 1.5; border-radius: 8px; clip: true; background: #16202a;
                if cell.has-art : Image { width: 100%; height: 100%; source: cell.art; image-fit: ImageFit.cover; }
                if !cell.has-art && cell.valid : PosterPlaceholder { width: 100%; height: 100%; }
            }
            if cell.valid : Text { text: cell.name; color: white; font-size: 13px; overflow: elide; horizontal-alignment: center; }
        }
        TouchArea { clicked => { if (cell.valid) { root.tapped(); } } }
    }

    export component MainWindow inherits Window {
        background: transparent;
        in property <int> view: 0;            // 0 browse · 1 detail · 2 player
        in property <int> tab: 0;             // 0 Home · 1 Movies · 2 Shows
        in property <bool> in-drill: false;
        in property <string> crumb: "";
        in property <string> status: "";
        in property <bool> pairing: false;
        in property <string> pair-code: "";
        // browse content
        in property <[GridRow]> grid: [];     // Home/Movies/Shows-series
        in property <[Cell]> list: [];        // Seasons/Episodes
        in property <bool> is-list: false;
        // detail
        in property <string> d-title: "";
        in property <string> d-sub: "";
        in property <string> d-overview: "";
        in property <bool> d-has-backdrop: false;
        in property <image> d-backdrop;
        in property <bool> d-resumable: false;
        // player overlay
        in property <bool> overlay: true;
        in property <string> np-title: "";
        in property <string> elapsed: "0:00";
        in property <string> total: "0:00";
        in property <float> progress: 0.0;
        in property <bool> playing: false;
        in property <bool> opening: false;
        callback set-tab(int);
        callback cell-tap(string);            // id of the tapped cell
        callback back();
        callback play();
        callback play-resume();
        callback player-tap();
        callback toggle();
        callback seek(float);

        property <color> accent: #4ac0ff;
        property <color> dim: #8a9098;

        // ── Browse ───────────────────────────────────────────────────────────
        if (root.view == 0) : Rectangle {
            width: 100%; height: 100%; background: #0b0d10;

            if root.pairing : VerticalLayout {
                alignment: center; spacing: 16px; padding: 30px;
                Text { text: "Jellyfin"; color: white; font-size: 30px; font-weight: 800; horizontal-alignment: center; }
                Text { text: "Quick Connect — enter this code in Jellyfin:"; color: dim; font-size: 15px; horizontal-alignment: center; wrap: word-wrap; }
                Rectangle { height: 90px; border-radius: 12px; background: #16202a;
                    Text { text: root.pair-code; color: accent; font-size: 46px; font-weight: 800; horizontal-alignment: center; vertical-alignment: center; } }
                Text { text: root.status; color: dim; font-size: 12px; horizontal-alignment: center; }
            }

            if !root.pairing : VerticalLayout {
                padding: 12px; spacing: 8px;
                // Tabs
                HorizontalLayout {
                    spacing: 6px; height: 42px;
                    for t[i] in [ "Home", "Movies", "Shows" ] : Rectangle {
                        horizontal-stretch: 1; border-radius: 8px;
                        background: root.tab == i ? #16202a : transparent;
                        Text { text: t; font-size: 15px; font-weight: root.tab == i ? 700 : 400;
                            horizontal-alignment: center; vertical-alignment: center; color: root.tab == i ? accent : #c8ccd2; }
                        TouchArea { clicked => { root.set-tab(i); } }
                    }
                }
                if root.in-drill : Rectangle {
                    height: 44px;
                    HorizontalLayout {
                        spacing: 10px;
                        Rectangle { width: 34px; height: 34px; y: (parent.height - self.height)/2;
                            Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100; commands: "M 62 24 L 36 50 L 62 76"; stroke: white; stroke-width: 9px; fill: transparent; }
                            TouchArea { clicked => { root.back(); } } }
                        Text { text: root.crumb; color: white; font-size: 18px; font-weight: 700; vertical-alignment: center; horizontal-stretch: 1; overflow: elide; }
                    }
                }
                Text { text: root.status; color: dim; font-size: 12px; overflow: elide; }

                // Grid (posters) or list (seasons/episodes)
                if !root.is-list : ListView {
                    vertical-stretch: 1;
                    for row[i] in root.grid : HorizontalLayout {
                        padding: 6px; spacing: 12px;
                        Poster { cell: row.a; horizontal-stretch: 1; tapped => { root.cell-tap(row.a.id); } }
                        Poster { cell: row.b; horizontal-stretch: 1; tapped => { root.cell-tap(row.b.id); } }
                    }
                }
                if root.is-list : ListView {
                    vertical-stretch: 1;
                    for c[i] in root.list : Rectangle {
                        height: 58px;
                        HorizontalLayout {
                            padding: 8px; spacing: 12px;
                            VerticalLayout {
                                alignment: center; horizontal-stretch: 1;
                                Text { text: c.name; color: white; font-size: 16px; overflow: elide; }
                                if c.sub != "" : Text { text: c.sub; color: dim; font-size: 12px; overflow: elide; }
                            }
                            if c.kind < 3 : Rectangle { width: 14px; height: 14px; y: (parent.height - self.height)/2;
                                Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100; commands: "M 38 24 L 64 50 L 38 76"; stroke: #55606a; stroke-width: 9px; fill: transparent; } }
                        }
                        TouchArea { clicked => { root.cell-tap(c.id); } }
                    }
                }
            }
        }

        // ── Detail ───────────────────────────────────────────────────────────
        if (root.view == 1) : Rectangle {
            width: 100%; height: 100%; background: #0b0d10;
            VerticalLayout {
                // Backdrop hero
                Rectangle {
                    height: root.width * 0.56; clip: true; background: #16202a;
                    if root.d-has-backdrop : Image { width: 100%; height: 100%; source: root.d-backdrop; image-fit: ImageFit.cover; }
                    Rectangle { width: 100%; height: 100%; background: @linear-gradient(0deg, #0b0d10ff 0%, #0b0d1000 60%); }
                    Rectangle { width: 40px; height: 40px; x: 12px; y: 12px;
                        Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100; commands: "M 62 20 L 34 50 L 62 80"; stroke: white; stroke-width: 9px; fill: transparent; }
                        TouchArea { clicked => { root.back(); } } }
                }
                VerticalLayout {
                    padding: 18px; spacing: 12px;
                    Text { text: root.d-title; color: white; font-size: 24px; font-weight: 800; overflow: elide; }
                    Text { text: root.d-sub; color: dim; font-size: 14px; overflow: elide; }
                    HorizontalLayout {
                        spacing: 12px;
                        Rectangle {
                            width: 150px; height: 50px; border-radius: 25px; background: accent;
                            Text { text: root.d-resumable ? "▶  Resume" : "▶  Play"; color: #06283a; font-size: 17px; font-weight: 700; horizontal-alignment: center; vertical-alignment: center; }
                            TouchArea { clicked => { if (root.d-resumable) { root.play-resume(); } else { root.play(); } } }
                        }
                        if root.d-resumable : Rectangle {
                            width: 120px; height: 50px; border-radius: 25px; border-width: 1px; border-color: #3a4048;
                            Text { text: "From start"; color: white; font-size: 15px; horizontal-alignment: center; vertical-alignment: center; }
                            TouchArea { clicked => { root.play(); } }
                        }
                    }
                    Text { text: root.d-overview; color: #c0c4c8; font-size: 14px; wrap: word-wrap; }
                }
                Rectangle { }
            }
        }

        // ── Player (transparent — video behind) ──────────────────────────────
        if (root.view == 2) : Rectangle {
            width: 100%; height: 100%; background: transparent;
            TouchArea { clicked => { root.player-tap(); } }
            if root.overlay : Rectangle {
                width: 100%; height: 100%;
                Rectangle { y: 0; width: 100%; height: 96px; background: @linear-gradient(180deg, #000000d0 0%, #00000000 100%);
                    HorizontalLayout { padding: 14px; spacing: 12px;
                        Rectangle { width: 40px; height: 40px;
                            Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100; commands: "M 62 20 L 34 50 L 62 80"; stroke: white; stroke-width: 9px; fill: transparent; }
                            TouchArea { clicked => { root.back(); } } }
                        Text { text: root.np-title; color: white; font-size: 18px; font-weight: 700; vertical-alignment: center; horizontal-stretch: 1; overflow: elide; } } }
                if root.opening : Text { text: "opening…"; color: white; font-size: 16px; x: (parent.width - self.width)/2; y: (parent.height - self.height)/2; }
                Rectangle { y: parent.height - self.height; width: 100%; height: 150px; background: @linear-gradient(0deg, #000000d0 0%, #00000000 100%);
                    VerticalLayout { padding: 18px; spacing: 8px; alignment: end;
                        prog := Rectangle {
                            height: 44px; // tall hit area — easy to tap/drag the thin bar
                            property <bool> dragging: false;
                            property <float> drag-frac: 0.0;
                            property <float> shown: dragging ? drag-frac : root.progress;
                            Rectangle { width: 100%; height: 5px; y: (parent.height - self.height)/2; border-radius: 2px; background: #40484f; }
                            Rectangle { width: parent.width * prog.shown; height: 5px; y: (parent.height - self.height)/2; border-radius: 2px; background: accent; }
                            Rectangle { width: 18px; height: 18px; border-radius: 9px; background: white; x: prog.shown * (parent.width - self.width); y: (parent.height - self.height)/2; }
                            ta := TouchArea {
                                // Tap → seek to the tapped point (mouse-x is current at `clicked`,
                                // unlike at pointer `down` where it's stale on the first press).
                                clicked => { root.seek(clamp(ta.mouse-x / ta.width, 0.0, 1.0)); }
                                // Drag → live preview, commit on release. `clicked` does not fire for a drag.
                                moved => { prog.dragging = true; prog.drag-frac = clamp(ta.mouse-x / ta.width, 0.0, 1.0); }
                                pointer-event(ev) => {
                                    if (ev.kind == PointerEventKind.up && prog.dragging) {
                                        root.seek(prog.drag-frac); prog.dragging = false;
                                    }
                                }
                            }
                        }
                        HorizontalLayout {
                            Text { text: root.elapsed; color: #d0d4d8; font-size: 13px; vertical-alignment: center; }
                            Rectangle { horizontal-stretch: 1; }
                            Rectangle { width: 54px; height: 54px; border-radius: 27px; background: #4ac0ffcc;
                                if !root.playing : Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100; commands: "M 38 26 L 74 50 L 38 74 Z"; fill: white; }
                                if root.playing : Path { width: 100%; height: 100%; viewbox-width: 100; viewbox-height: 100; commands: "M 35 28 L 45 28 L 45 72 L 35 72 Z M 55 28 L 65 28 L 65 72 L 55 72 Z"; fill: white; }
                                TouchArea { clicked => { root.toggle(); } } }
                            Rectangle { horizontal-stretch: 1; }
                            Text { text: root.total; color: #d0d4d8; font-size: 13px; vertical-alignment: center; }
                        }
                    } }
            }
        }
    }
}

// ── constants ────────────────────────────────────────────────────────────────
const MAX_POSTER_INFLIGHT: usize = 4;
const OVERLAY_LINGER_NS: u64 = 4_000_000_000;

fn build_http() -> Option<reqwest::Client> {
    reqwest::Client::builder().user_agent("wandr-jellyfin/0.1").build().ok()
}

fn log(msg: impl Into<String>) {
    let m = msg.into();
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.last = m.clone();
        e.ui_dirty = true;
    });
    engine::log(m);
}

// ── model ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Movie,
    Series,
    Season,
    Episode,
}
impl Kind {
    fn code(self) -> i32 {
        match self {
            Kind::Movie => 0,
            Kind::Series => 1,
            Kind::Season => 2,
            Kind::Episode => 3,
        }
    }
    fn playable(self) -> bool {
        matches!(self, Kind::Movie | Kind::Episode)
    }
    fn poster(self) -> bool {
        matches!(self, Kind::Movie | Kind::Series | Kind::Season)
    }
}

#[derive(Clone)]
struct Media {
    id: String,
    name: String,
    sub: String,
    kind: Kind,
}

/// A pushed browse screen (a tab root, or a drilled Seasons/Episodes list).
struct Screen {
    title: String,
    media: Vec<Media>,
    is_list: bool, // true = Seasons/Episodes list; false = poster grid
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Boot,
    Pairing,
    Loading,
    Browse,
    Detail,
    Resolving,
    Playing,
    Failed,
}

struct Eng {
    phase: Phase,
    driver_spawned: bool,
    client: Option<JellyfinClient>,
    uid: Option<Uuid>,
    token: String,
    server: String,
    tab: i32,
    stack: Vec<Screen>,
    pairing_code: Option<String>,
    // detail
    detail: Option<Media>,
    detail_resume_us: i64,
    // playback
    resolved: Option<(String, String, String)>, // (item_id, media_source_id, play_session_id)
    pending_open: Option<(String, u64, String, i64, (u32, u32), bool)>, // url,total,title,dur_us,surface,is_mkv
    pending_seek_us: i64,
    last_report_us: i64,
    view: i32,
    overlay_until_ns: u64,
    last_surface: (u32, u32),
    /// Current immersive (chrome-hidden) state pushed to the arbiter — hide the status
    /// bar + taskbar whenever the player's transport controls are hidden.
    immersive: bool,
    // posters (by item id)
    posters: HashMap<String, Image>,
    poster_inflight: HashSet<String>,
    pending_posters: Vec<(String, Vec<u8>)>,
    // detail backdrop
    pending_backdrop: Option<(String, Vec<u8>)>,
    backdrop_id: Option<String>,
    // ui
    model_dirty: bool,
    ui_dirty: bool,
    last: String,
}

thread_local! {
    static ENG: RefCell<Eng> = RefCell::new(Eng {
        phase: Phase::Boot,
        driver_spawned: false,
        client: None,
        uid: None,
        token: String::new(),
        server: String::new(),
        tab: 0,
        stack: Vec::new(),
        pairing_code: None,
        detail: None,
        detail_resume_us: 0,
        resolved: None,
        pending_open: None,
        pending_seek_us: 0,
        last_report_us: 0,
        view: 0,
        overlay_until_ns: 0,
        last_surface: (0, 0),
        immersive: false,
        posters: HashMap::new(),
        poster_inflight: HashSet::new(),
        pending_posters: Vec::new(),
        pending_backdrop: None,
        backdrop_id: None,
        model_dirty: false,
        ui_dirty: true,
        last: "starting…".into(),
    });
    static UI: RefCell<Option<MainWindow>> = const { RefCell::new(None) };
    static GRID: RefCell<Option<Rc<VecModel<GridRow>>>> = const { RefCell::new(None) };
    static LAST_NANOS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn set_phase(p: Phase) {
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.phase = p;
        e.ui_dirty = true;
    });
}
fn client() -> Option<JellyfinClient> {
    ENG.with(|e| e.borrow().client.clone())
}
fn uid() -> Option<Uuid> {
    ENG.with(|e| e.borrow().uid)
}
fn request_redraw() {
    UI.with(|u| {
        if let Some(ui) = u.borrow().as_ref() {
            ui.window().request_redraw();
        }
    });
}
fn fmt_secs(sec: i64) -> String {
    let s = sec.max(0);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

// ── data-layer helpers over the SDK ──────────────────────────────────────────

fn kind_of(k: Option<BaseItemKind>) -> Option<Kind> {
    match k {
        Some(BaseItemKind::Movie) => Some(Kind::Movie),
        Some(BaseItemKind::Series) => Some(Kind::Series),
        Some(BaseItemKind::Season) => Some(Kind::Season),
        Some(BaseItemKind::Episode) => Some(Kind::Episode),
        _ => None,
    }
}

async fn items_query(q: ItemsQuery) -> Vec<Media> {
    let Some(c) = client() else { return Vec::new() };
    match c.items().get_items(q).await {
        Ok(res) => res
            .items
            .into_iter()
            .filter_map(|it| {
                let kind = kind_of(it.kind)?;
                Some(Media {
                    id: it.id?.to_string(),
                    name: it.name.unwrap_or_default(),
                    sub: it.production_year.map(|y| y.to_string()).unwrap_or_default(),
                    kind,
                })
            })
            .collect(),
        Err(e) => {
            log(format!("browse: {e}"));
            Vec::new()
        }
    }
}

fn base_query(u: Uuid) -> ItemsQuery {
    ItemsQuery::new().user_id(u).sort_by(ItemSortBy::SortName).sort_order(SortOrder::Ascending)
}

/// Load a tab root (Home / Movies / Shows). Clears the stack.
async fn load_tab(tab: i32) {
    let Some(u) = uid() else { return };
    set_phase(Phase::Loading);
    let (title, media, is_list) = match tab {
        1 => (
            "Movies".to_string(),
            items_query(base_query(u).recursive(true).include_item_type(BaseItemKind::Movie).limit(500)).await,
            false,
        ),
        2 => (
            "Shows".to_string(),
            items_query(base_query(u).recursive(true).include_item_type(BaseItemKind::Series).limit(500)).await,
            false,
        ),
        _ => ("Home".to_string(), home_media(u).await, false),
    };
    push_screen(Screen { title, media, is_list }, true);
}

/// Home = Continue Watching (resume) then Latest movies + series.
async fn home_media(u: Uuid) -> Vec<Media> {
    let Some(c) = client() else { return Vec::new() };
    let mut out = Vec::new();
    if let Ok(res) = c.user_library().get_resume_items(ResumeItemsQuery::new().user_id(u).limit(20)).await {
        for it in res.items {
            if let Some(kind) = kind_of(it.kind) {
                if let Some(id) = it.id {
                    out.push(Media { id: id.to_string(), name: it.name.unwrap_or_default(), sub: "▶ Continue".into(), kind });
                }
            }
        }
    }
    let latest = c
        .user_library()
        .get_latest_media(LatestMediaQuery::new().user_id(u).limit(24).param("includeItemTypes", "Movie,Series"))
        .await
        .unwrap_or_default();
    for it in latest {
        if let Some(kind) = kind_of(it.kind) {
            if let Some(id) = it.id {
                out.push(Media { id: id.to_string(), name: it.name.unwrap_or_default(), sub: "Recently added".into(), kind });
            }
        }
    }
    out
}

async fn drill(parent: String, into: Kind, title: String) {
    let Some(u) = uid() else { return };
    let Ok(pid) = Uuid::parse_str(&parent) else { return };
    let bik = match into {
        Kind::Season => BaseItemKind::Season,
        Kind::Episode => BaseItemKind::Episode,
        _ => return,
    };
    set_phase(Phase::Loading);
    let media = items_query(ItemsQuery::new().user_id(u).parent_id(pid).include_item_type(bik)).await;
    push_screen(Screen { title, media, is_list: true }, false);
}

fn push_screen(s: Screen, reset: bool) {
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        if reset {
            e.stack.clear();
        }
        e.stack.push(s);
        e.phase = Phase::Browse;
        e.view = 0;
        e.model_dirty = true;
        e.ui_dirty = true;
    });
    request_redraw();
}

// ── poster / backdrop image fetch (SDK get_image → bytes) ────────────────────

async fn fetch_image(id: String, backdrop: bool) {
    let Some(c) = client() else { return };
    let Ok(uu) = Uuid::parse_str(&id) else { return };
    let (ty, w) = if backdrop { (ImageType::Backdrop, 800) } else { (ImageType::Primary, 300) };
    let req = ItemImageRequest::new().max_width(w).format(ImageFormat::Webp);
    match c.items().get_image(uu, ty, req).await {
        Ok(resp) => match resp.bytes().await {
            Ok(bytes) => ENG.with(|e| {
                let mut e = e.borrow_mut();
                if backdrop {
                    e.pending_backdrop = Some((id, bytes.to_vec()));
                } else {
                    e.pending_posters.push((id, bytes.to_vec()));
                }
            }),
            Err(_) => ENG.with(|e| { e.borrow_mut().poster_inflight.remove(&id); }),
        },
        Err(_) => ENG.with(|e| { e.borrow_mut().poster_inflight.remove(&id); }),
    }
}

fn decode_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((rgba.into_raw(), w, h))
}

// ── open a title for playback (PlaybackInfo → stream URL → engine) ───────────

async fn open_media(id: String, resume_us: i64) {
    let (Some(c), Some(u)) = (client(), uid()) else { return };
    let Ok(uu) = Uuid::parse_str(&id) else { return };
    set_phase(Phase::Resolving);
    let req = PlaybackInfoRequest::new().user_id(u).enable_direct_play(true).enable_transcoding(false);
    let pb = match c.items().post_playback_info(uu, req).await {
        Ok(pb) => pb,
        Err(e) => {
            log(format!("PlaybackInfo: {e}"));
            return set_phase(Phase::Browse);
        }
    };
    let Some(ms) = pb.media_sources.into_iter().next() else {
        log("no media source");
        return set_phase(Phase::Browse);
    };
    if ms.supports_direct_play != Some(true) || ms.transcoding_url.is_some() {
        log("⚠ server would transcode — skipped (DirectPlay only)");
        return set_phase(Phase::Browse);
    }
    // Jellyfin's scanned runtime → the engine's transport total + the basis for seek
    // (streamed-container duration parsing is unreliable, so this is authoritative).
    let dur_us = ms.run_time_ticks.unwrap_or(0).max(0) / 10; // 100 ns ticks → µs
    let msid = ms.id.clone().unwrap_or_else(|| id.clone());
    let psid = pb.play_session_id.clone().unwrap_or_default();
    let container = ms.container.clone().unwrap_or_default();
    let (server, token) = ENG.with(|e| { let e = e.borrow(); (e.server.clone(), e.token.clone()) });
    let cont = if container.is_empty() { "mp4".to_string() } else { container.clone() };
    let url = format!(
        "{}/Videos/{id}/stream.{cont}?static=true&mediaSourceId={msid}&playSessionId={psid}&api_key={token}",
        server.trim_end_matches('/')
    );
    // Probe head to sniff MKV vs MP4 (container can be empty/ambiguous) + get total len.
    let Some(hc) = build_http() else { return set_phase(Phase::Browse) };
    let probe = match engine::net::fetch_range(&hc, &url, 0, Some(65_535)).await {
        Ok(r) => r,
        Err(e) => {
            log(format!("stream probe: {e}"));
            return set_phase(Phase::Browse);
        }
    };
    let is_mkv = probe.bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) || matches!(cont.as_str(), "mkv" | "webm");
    let title = ENG.with(|e| e.borrow().detail.as_ref().map(|d| d.name.clone()).unwrap_or_default());
    let surface = engine::CONTROLS.with(|c| c.borrow().surface);
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.resolved = Some((id.clone(), msid, psid));
        e.pending_seek_us = resume_us;
        e.pending_open = Some((url, probe.total_len, title, dur_us, surface, is_mkv));
    });
}

// ── session reporting ────────────────────────────────────────────────────────

fn report(kind: u8, position_us: i64, paused: bool) {
    let info = ENG.with(|e| e.borrow().resolved.clone());
    let (Some((id, msid, psid)), Some(c)) = (info, client()) else { return };
    let Ok(uu) = Uuid::parse_str(&id) else { return };
    let ticks = position_us * 10;
    match kind {
        0 => {
            let mut p = PlaybackStart::new(uu).play_session_id(psid).position_ticks(ticks);
            p.media_source_id = Some(msid);
            p.play_method = Some(PlayMethod::DirectPlay);
            reqwest::task::spawn(async move { let _ = c.playstate().report_playback_start(p).await; });
        }
        1 => {
            let mut p = PlaybackProgress::new(uu).play_session_id(psid).position_ticks(ticks);
            p.media_source_id = Some(msid);
            p.is_paused = paused;
            p.play_method = Some(PlayMethod::DirectPlay);
            reqwest::task::spawn(async move { let _ = c.playstate().report_playback_progress(p).await; });
        }
        _ => {
            let mut p = PlaybackStop::new(uu).play_session_id(psid).position_ticks(ticks);
            p.media_source_id = Some(msid);
            reqwest::task::spawn(async move { let _ = c.playstate().report_playback_stopped(p).await; });
        }
    }
}

fn stop_playback() {
    let final_us = engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.clock_us()).unwrap_or(0));
    report(2, final_us, false);
    engine::STREAM.with(|s| *s.borrow_mut() = None);
    let _ = engine::with_audio(|pb| pb.flush());
    slint_wandr::set_continuous_render(false);
    // Restore the system chrome we hid for fullscreen playback.
    if ENG.with(|e| std::mem::replace(&mut e.borrow_mut().immersive, false)) {
        slint_wandr::set_immersive(false);
    }
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.resolved = None;
        e.view = 0;
        e.phase = Phase::Browse;
        e.ui_dirty = true;
    });
    request_redraw();
}

// ── tap handlers ─────────────────────────────────────────────────────────────

fn find_media(id: &str) -> Option<Media> {
    ENG.with(|e| {
        let e = e.borrow();
        e.stack.last().and_then(|s| s.media.iter().find(|m| m.id == id).cloned())
    })
}

fn cell_tap(id: String) {
    let Some(m) = find_media(&id) else { return };
    match m.kind {
        Kind::Series => {
            reqwest::task::spawn(drill(m.id.clone(), Kind::Season, m.name.clone()));
        }
        Kind::Season => {
            reqwest::task::spawn(drill(m.id.clone(), Kind::Episode, m.name.clone()));
        }
        Kind::Movie | Kind::Episode => open_detail(m),
    }
}

fn open_detail(m: Media) {
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.detail = Some(m.clone());
        e.detail_resume_us = 0;
        e.backdrop_id = None;
        e.view = 1;
        e.phase = Phase::Detail;
        e.ui_dirty = true;
    });
    request_redraw();
    // Fetch backdrop + detail overview + resume position.
    reqwest::task::spawn(fetch_image(m.id.clone(), true));
    reqwest::task::spawn(load_detail(m.id));
}

async fn load_detail(id: String) {
    let (Some(c), Some(u)) = (client(), uid()) else { return };
    let Ok(uu) = Uuid::parse_str(&id) else { return };
    if let Ok(det) = c.user_library().get_item(uu, Some(u)).await {
        let overview = det.overview.clone().unwrap_or_default();
        ENG.with(|e| {
            if let Some(d) = e.borrow_mut().detail.as_mut() {
                if !overview.is_empty() {
                    d.sub = format!("{}   ·   {}", d.sub, fmt_secs(det.run_time_ticks.unwrap_or(0) / 10_000_000));
                }
            }
        });
        DETAIL_OVERVIEW.with(|o| *o.borrow_mut() = overview);
        ENG.with(|e| e.borrow_mut().ui_dirty = true);
    }
    if let Ok(ud) = c.items().get_user_data(uu, Some(u)).await {
        let resume_us = ud.playback_position_ticks.unwrap_or(0) / 10;
        ENG.with(|e| e.borrow_mut().detail_resume_us = resume_us);
    }
    request_redraw();
}

thread_local! {
    static DETAIL_OVERVIEW: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_tab(t: i32) {
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.tab = t;
        e.stack.clear();
        e.model_dirty = true;
        e.ui_dirty = true;
        e.last = "loading…".into();
    });
    request_redraw();
    reqwest::task::spawn(load_tab(t));
}

fn go_back() {
    let popped = ENG.with(|e| {
        let mut e = e.borrow_mut();
        match e.view {
            2 => false, // player handled by cmd_back
            1 => {
                e.view = 0;
                e.phase = Phase::Browse;
                e.ui_dirty = true;
                true
            }
            _ => {
                if e.stack.len() > 1 {
                    e.stack.pop();
                    e.model_dirty = true;
                    e.ui_dirty = true;
                }
                true
            }
        }
    });
    if popped {
        request_redraw();
    }
}

fn play_detail(resume: bool) {
    let (id, resume_us) = ENG.with(|e| {
        let e = e.borrow();
        (e.detail.as_ref().map(|d| d.id.clone()), if resume { e.detail_resume_us } else { 0 })
    });
    let Some(id) = id else { return };
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.view = 2;
        e.overlay_until_ns = u64::MAX;
        e.ui_dirty = true;
    });
    slint_wandr::set_continuous_render(true);
    request_redraw();
    reqwest::task::spawn(open_media(id, resume_us));
}

fn player_tap() {
    let now = LAST_NANOS.with(|n| n.get());
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        let visible = now < e.overlay_until_ns;
        e.overlay_until_ns = if visible { 0 } else { now + OVERLAY_LINGER_NS };
    });
}

fn cmd_back() {
    let in_player = ENG.with(|e| e.borrow().view == 2);
    if in_player {
        stop_playback();
    } else {
        go_back();
    }
}

fn cmd_toggle() {
    engine::CONTROLS.with(|c| {
        let mut c = c.borrow_mut();
        c.controls_bump = true;
        c.paused = !c.paused;
    });
    let now = LAST_NANOS.with(|n| n.get());
    ENG.with(|e| e.borrow_mut().overlay_until_ns = now + OVERLAY_LINGER_NS);
}

fn cmd_seek(frac: f32) {
    let dur = engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.duration_us()).unwrap_or(0));
    if dur > 0 {
        let target = (frac.clamp(0.0, 1.0) as f64 * dur as f64) as i64;
        engine::CONTROLS.with(|c| {
            let mut c = c.borrow_mut();
            c.controls_bump = true;
            c.seek_request = Some(target);
        });
    }
    let now = LAST_NANOS.with(|n| n.get());
    ENG.with(|e| e.borrow_mut().overlay_until_ns = now + OVERLAY_LINGER_NS);
}

// ── driver: pair or load session, then Home ──────────────────────────────────

async fn driver() {
    let Some(http) = build_http() else {
        log("no HTTP client");
        return set_phase(Phase::Failed);
    };
    let session = match pairing::load_session() {
        Some(s) => s,
        None => match pair(&http).await {
            Ok(s) => s,
            Err(e) => {
                log(format!("pairing failed: {e}"));
                return set_phase(Phase::Failed);
            }
        },
    };
    let client = match JellyfinClient::builder(&session.server_url) {
        Ok(b) => match b.client_name("wandr").device_name("wandr").device_id(&session.device_id).build() {
            Ok(c) => c,
            Err(e) => {
                log(format!("client: {e}"));
                return set_phase(Phase::Failed);
            }
        },
        Err(e) => {
            log(format!("client url: {e}"));
            return set_phase(Phase::Failed);
        }
    };
    client.set_token(&session.access_token);
    let uid = Uuid::parse_str(&session.user_id).ok();
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.client = Some(client);
        e.uid = uid;
        e.token = session.access_token.clone();
        e.server = session.server_url.clone();
    });
    log(format!("connected to {}", session.server_url));
    load_tab(0).await;
}

async fn pair(http: &reqwest::Client) -> Result<Session, String> {
    let server = pairing::server_override().unwrap_or_else(|| pairing::DEFAULT_SERVER.to_string());
    let device_id = pairing::load_or_make_device_id();
    if !pairing::qc_enabled(http, &server).await {
        return Err("Quick Connect is disabled on the server".into());
    }
    let qc = pairing::qc_initiate(http, &server, &device_id).await?;
    log(format!("Quick Connect code: {}", qc.code));
    ENG.with(|e| {
        let mut e = e.borrow_mut();
        e.pairing_code = Some(qc.code.clone());
        e.phase = Phase::Pairing;
        e.ui_dirty = true;
    });
    for _ in 0..150 {
        if pairing::qc_poll(http, &server, &device_id, &qc.secret).await? {
            let (token, user_id) = pairing::qc_exchange(http, &server, &device_id, &qc.secret).await?;
            let s = Session { server_url: server, user_id, device_id, access_token: token };
            pairing::save_session(&s).map_err(|e| format!("save session: {e}"))?;
            ENG.with(|e| e.borrow_mut().pairing_code = None);
            return Ok(s);
        }
        reqwest::task::sleep(Duration::from_secs(2)).await;
    }
    Err("Quick Connect timed out".into())
}

// ── bg-tick: demux/decode/report (video present runs on the render path) ─────

fn engine_tick() -> u32 {
    if ENG.with(|e| {
        let mut e = e.borrow_mut();
        if !e.driver_spawned {
            e.driver_spawned = true;
            true
        } else {
            false
        }
    }) {
        reqwest::task::spawn(driver());
    }

    // Open a resolved title (async context — the demuxers' block_on reader).
    if let Some((url, total, title, dur_us, surface, is_mkv)) = ENG.with(|e| e.borrow_mut().pending_open.take()) {
        let opened = if is_mkv {
            engine::open_mkv_sync(url, total, title, dur_us, surface)
        } else {
            engine::open_mp4_sync(url, total, title, dur_us, surface)
        };
        match opened {
            Ok(()) => {
                set_phase(Phase::Playing);
                ENG.with(|e| e.borrow_mut().last_report_us = 0);
                let seek = ENG.with(|e| e.borrow().pending_seek_us);
                if seek > 5_000_000 {
                    engine::CONTROLS.with(|c| c.borrow_mut().seek_request = Some(seek));
                    log(format!("resume: {:.0}s", seek as f64 / 1e6));
                }
                report(0, seek.max(0), false);
            }
            Err(msg) => {
                log(format!("open failed — {msg}"));
                stop_playback();
            }
        }
    }

    if ENG.with(|e| e.borrow().phase == Phase::Playing) {
        if let Some(h) = engine::STREAM.with(|s| s.borrow().as_ref().and_then(|p| p.prefetch_handle())) {
            engine::drive_prefetch(&h);
        }
        let seek = engine::CONTROLS.with(|c| c.borrow_mut().seek_request.take());
        engine::STREAM.with(|s| {
            if let Some(p) = s.borrow_mut().as_mut() {
                if let Some(t) = seek {
                    // KNOWN BUG: seek is intermittent across titles — engine `do_seek` for a
                    // STREAMED container depends on the MP4 moov sample-table / MKV cues being
                    // parsed + the target byte-range being fetchable; some titles lack cues or
                    // have moov-at-end not yet loaded, so the seek doesn't land. Engine-level
                    // robustness fix (index-on-open / cue fallback) is a follow-up, not UI.
                    engine::do_seek(p, t);
                }
                engine::fill_queues(p);
                engine::decode_audio(p);
            }
        });
        if engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.is_ended()).unwrap_or(false)) {
            stop_playback();
        } else {
            let clk = engine::STREAM.with(|s| s.borrow().as_ref().map(|p| p.clock_us()).unwrap_or(0));
            let due = ENG.with(|e| {
                let mut e = e.borrow_mut();
                if clk - e.last_report_us >= 10_000_000 {
                    e.last_report_us = clk;
                    true
                } else {
                    false
                }
            });
            if due {
                let paused = engine::CONTROLS.with(|c| c.borrow().paused);
                report(1, clk, paused);
            }
        }
    }

    reconcile_posters();
    push_ui();

    ENG.with(|e| match e.borrow().phase {
        Phase::Playing => 16,
        Phase::Resolving | Phase::Loading | Phase::Boot => 60,
        Phase::Pairing => 400,
        _ => 120,
    })
}

fn reconcile_posters() {
    // decode posters
    let pend: Vec<(String, Vec<u8>)> = ENG.with(|e| std::mem::take(&mut e.borrow_mut().pending_posters));
    for (id, bytes) in pend {
        match decode_rgba(&bytes) {
            Some((rgba, w, h)) => {
                let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
                buf.make_mut_bytes().copy_from_slice(&rgba);
                let img = Image::from_rgba8(buf);
                ENG.with(|e| {
                    let mut e = e.borrow_mut();
                    e.posters.insert(id.clone(), img);
                    e.poster_inflight.remove(&id);
                    e.model_dirty = true; // rebuild grid rows (cheap; grid is small on screen)
                    e.ui_dirty = true;
                });
            }
            None => ENG.with(|e| { e.borrow_mut().poster_inflight.remove(&id); }),
        }
    }
    // decode backdrop
    if let Some((id, bytes)) = ENG.with(|e| e.borrow_mut().pending_backdrop.take()) {
        if let Some((rgba, w, h)) = decode_rgba(&bytes) {
            let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
            buf.make_mut_bytes().copy_from_slice(&rgba);
            let img = Image::from_rgba8(buf);
            ENG.with(|e| {
                let mut e = e.borrow_mut();
                e.posters.insert(format!("bd:{id}"), img);
                e.backdrop_id = Some(id);
                e.ui_dirty = true;
            });
        }
    }
    // enqueue poster fetches for the visible poster-grid screen
    let to_fetch: Vec<String> = ENG.with(|e| {
        let e = e.borrow();
        if e.phase != Phase::Browse {
            return Vec::new();
        }
        let Some(scr) = e.stack.last() else { return Vec::new() };
        if scr.is_list {
            return Vec::new();
        }
        let budget = MAX_POSTER_INFLIGHT.saturating_sub(e.poster_inflight.len());
        let mut out = Vec::new();
        for m in &scr.media {
            if out.len() >= budget {
                break;
            }
            if !m.kind.poster() || e.posters.contains_key(&m.id) || e.poster_inflight.contains(&m.id) {
                continue;
            }
            out.push(m.id.clone());
        }
        out
    });
    for id in to_fetch {
        ENG.with(|e| { e.borrow_mut().poster_inflight.insert(id.clone()); });
        reqwest::task::spawn(fetch_image(id, false));
    }
}

// ── push state → Slint ───────────────────────────────────────────────────────

fn cell_for(m: &Media, posters: &HashMap<String, Image>) -> Cell {
    let art = if m.kind.poster() { posters.get(&m.id).cloned() } else { None };
    Cell {
        id: m.id.as_str().into(),
        name: m.name.as_str().into(),
        sub: m.sub.as_str().into(),
        kind: m.kind.code(),
        has_art: art.is_some(),
        art: art.unwrap_or_default(),
        valid: true,
    }
}
fn empty_cell() -> Cell {
    Cell { id: "".into(), name: "".into(), sub: "".into(), kind: 0, has_art: false, art: Image::default(), valid: false }
}

fn push_ui() {
    UI.with(|u| {
        let b = u.borrow();
        let Some(ui) = b.as_ref() else { return };
        ENG.with(|st| {
            let mut e = st.borrow_mut();
            ui.set_view(e.view);
            ui.set_tab(e.tab);
            ui.set_status(e.last.as_str().into());
            ui.set_pairing(matches!(e.phase, Phase::Pairing));
            ui.set_pair_code(e.pairing_code.clone().unwrap_or_default().as_str().into());
            let in_drill = e.stack.len() > 1;
            ui.set_in_drill(in_drill);
            ui.set_crumb(if in_drill { e.stack.last().map(|s| s.title.clone()).unwrap_or_default() } else { String::new() }.as_str().into());

            let is_list = e.stack.last().map(|s| s.is_list).unwrap_or(false);
            ui.set_is_list(is_list);

            if e.model_dirty {
                e.model_dirty = false;
                if let Some(scr) = e.stack.last() {
                    if is_list {
                        let cells: Vec<Cell> = scr.media.iter().map(|m| cell_for(m, &e.posters)).collect();
                        ui.set_list(ModelRc::from(Rc::new(VecModel::from(cells))));
                    } else {
                        let mut rows: Vec<GridRow> = Vec::new();
                        let cells: Vec<Cell> = scr.media.iter().map(|m| cell_for(m, &e.posters)).collect();
                        let mut it = cells.into_iter();
                        loop {
                            let a = it.next();
                            let Some(a) = a else { break };
                            let b = it.next().unwrap_or_else(empty_cell);
                            rows.push(GridRow { a, b });
                        }
                        let model = Rc::new(VecModel::from(rows));
                        ui.set_grid(ModelRc::from(model.clone()));
                        GRID.with(|g| *g.borrow_mut() = Some(model));
                    }
                }
            }

            // Detail
            if let Some(d) = e.detail.as_ref() {
                ui.set_d_title(d.name.as_str().into());
                ui.set_d_sub(d.sub.as_str().into());
                ui.set_d_resumable(e.detail_resume_us > 5_000_000);
                match e.backdrop_id.as_ref().and_then(|id| e.posters.get(&format!("bd:{id}")).cloned()) {
                    Some(img) => { ui.set_d_backdrop(img); ui.set_d_has_backdrop(true); }
                    None => ui.set_d_has_backdrop(false),
                }
                ui.set_d_overview(DETAIL_OVERVIEW.with(|o| o.borrow().clone()).as_str().into());
                ui.set_np_title(d.name.as_str().into());
            }
            ui.set_opening(matches!(e.phase, Phase::Resolving));

            if e.ui_dirty {
                e.ui_dirty = false;
                ui.window().request_redraw();
            }
        });
    });
}

// ── render-path video pump (host present clock) ──────────────────────────────

fn on_render_frame(nanos: u64) {
    LAST_NANOS.with(|n| n.set(nanos));
    // Reconcile window size → engine (video letterbox rect).
    UI.with(|u| {
        if let Some(ui) = u.borrow().as_ref() {
            let sz = ui.window().size();
            let cur = (sz.width.max(1), sz.height.max(1));
            let changed = ENG.with(|e| {
                let mut e = e.borrow_mut();
                if e.last_surface != cur { e.last_surface = cur; true } else { false }
            });
            if changed {
                engine::log(format!("jf-geo: surface {}x{} scale {:.2}", cur.0, cur.1, ui.window().scale_factor()));
                engine::set_surface(cur.0, cur.1);
            }
        }
    });
    let playing = ENG.with(|e| matches!(e.borrow().phase, Phase::Playing));
    if playing && engine::STREAM.with(|s| s.borrow().is_some()) {
        engine::pump_stream(nanos);
    }
    if ENG.with(|e| e.borrow().view == 2) {
        UI.with(|u| {
            let b = u.borrow();
            let Some(ui) = b.as_ref() else { return };
            let (clk, tot) = engine::STREAM.with(|s| match s.borrow().as_ref() {
                Some(p) => (p.clock_us(), p.duration_us()),
                None => (0, 0),
            });
            ui.set_elapsed(fmt_secs(clk / 1_000_000).as_str().into());
            ui.set_total(fmt_secs(tot / 1_000_000).as_str().into());
            ui.set_progress(if tot > 0 { (clk as f32 / tot as f32).clamp(0.0, 1.0) } else { 0.0 });
            let is_playing = playing && !engine::CONTROLS.with(|c| c.borrow().paused);
            ui.set_playing(is_playing);
            let overlay_vis = ENG.with(|e| nanos < e.borrow().overlay_until_ns);
            ui.set_overlay(overlay_vis);
            // Fullscreen immersive: chrome (status + task bar) hides in lockstep with the
            // transport controls — hidden controls → immersive on. Only fire on a change so
            // we hit the arbiter socket at transitions, not every frame.
            let want_imm = !overlay_vis;
            if ENG.with(|e| {
                let mut e = e.borrow_mut();
                if e.immersive != want_imm { e.immersive = want_imm; true } else { false }
            }) {
                slint_wandr::set_immersive(want_imm);
            }
        });
    }
}

// ── WIT bindings + launch ─────────────────────────────────────────────────────
mod bindings {
    slint_wandr::__wit_bindgen::generate!({
        path: "wit-p3",
        world: "jellyfin-extras",
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

slint_wandr::launch!(|| {
    let ui = MainWindow::new().expect("jellyfin: create MainWindow");
    ui.on_set_tab(set_tab);
    ui.on_cell_tap(|id| cell_tap(id.to_string()));
    ui.on_back(cmd_back);
    ui.on_play(|| play_detail(false));
    ui.on_play_resume(|| play_detail(true));
    ui.on_player_tap(player_tap);
    ui.on_toggle(cmd_toggle);
    ui.on_seek(cmd_seek);
    UI.with(|u| *u.borrow_mut() = Some(ui.clone_strong()));
    slint_wandr::on_render_frame(on_render_frame);
    push_ui();
    ui.show().expect("jellyfin: show");
    ui
});
