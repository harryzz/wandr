//! signal-ui — a dioxus-canvas guest that drives the Signal engine purely through
//! the `wandr:signal/chat` contract (task-67 Phase 2 item 2). It renders the
//! conversation (history + live events from `poll-events`), sends typed messages
//! via `send`, and surfaces link/connection state. Composed with signal-engine
//! through `wac plug` (item 3); standalone it imports `wandr:signal/chat`, which
//! the host (or the plugged engine) satisfies.
//!
//! Senders are shown as raw ACIs (v1 — display-name/username resolution is a
//! deferred follow-up; see tasks/67).
//!
//! Engine pumping: the engine only advances during `poll-events`, so the UI can't
//! be purely on-demand. `pre_frame` lowers the frame-delay floor (the host keeps
//! calling `render_frame` ~8×/s), polls the engine every frame, and `mark_dirty`s
//! only when the model actually changed; `app` calls `needs_update()` to stay
//! armed so those changes reach the tree. Idle cost is a cheap poll per tick (no
//! relayout unless something arrived).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::prelude::*;

// One generate! for everything this guest talks to: the skiko-gfx world
// (canvas/paragraph/ime imports + renderer/frame-pacing exports) AND the engine's
// `wandr:signal/chat` import (see wit/). A second generate! would conflict on
// `_rt`/`cabi_realloc`/the component-type section, so they share one. Same
// `export_macro_name` + `runtime_path` as dioxus-canvas's `skiko_world!`, so
// `wire!` finds the bindings and the export macro.
dioxus_canvas::__wit_bindgen::generate!({
    world: "signal-ui-app",
    path: "wit",
    generate_all,
    pub_export_macro: true,
    export_macro_name: "__dioxus_canvas_export",
    runtime_path: "::dioxus_canvas::__wit_bindgen::rt",
});

use wandr::signal::chat;
// M4 — host imports the guest calls: raise/clear notifications + schedule the
// keep-alive alarm. (The matching exports are impl'd on `__DioxusCanvasGuest`
// after the `wire!` below.)
use wandr::alarm::scheduler;
use wandr::notify::notifier;

/// M4 — keep-alive alarm: re-wake Signal if it dies (crash / OOM / reboot). When
/// alive it's a no-op refresh; when dead the arbiter relaunches it hidden (M1) →
/// it reconnects as a background-service (M2). Coarse interval — the persistent
/// socket handles liveness while running; this is only the dead-app backstop.
const KEEPALIVE_ID: u64 = 1;
// 15 min — matches Android's minimum periodic-job interval (its battery policy).
// This alarm only does real work when Signal is DEAD (relaunch + reconnect); while
// alive (the common case — resident bg-service) each fire is just a cheap redundant
// pump, so a long interval costs little and saves wakeups. A dead Signal recovers
// within one interval.
const KEEPALIVE_MS: u64 = 900_000;

// Renderer/sink/IME wiring from dioxus-canvas, over the bindings generated above.
// The pre_frame hook is where we pump the engine each tick.
//
// NOTE: making the composer ride above the soft keyboard is deferred to task 68
// (host-driven keyboard inset) — the app must NOT hard-code the overlay height
// (it's 1200px today but resizable via request-overlay-height). For now the
// composer is a plain bottom bar; while typing it sits behind the keyboard.
// Frames the engine has produced nothing — drives the adaptive idle cadence below.
thread_local! {
    static IDLE_FRAMES: Cell<u32> = const { Cell::new(0) };
    /// M4 — consecutive `bg-tick`s the engine produced nothing on; drives the
    /// background idle-ramp (battery) the same way `IDLE_FRAMES` drives the
    /// foreground one. Reset to 0 whenever a backgrounded pump sees activity.
    static BG_IDLE: Cell<u32> = const { Cell::new(0) };
}

// M4 — background-pump idle-ramp (battery). When backgrounded the host calls
// `bg-tick` (no rendering); we pump FAST right after activity so a message lands
// quickly, then ramp DOWN when the socket is quiet so a truly-idle Signal wakes
// the CPU/radio far less. The host clamps to its ~1 s idle cap, so IDLE_MS is the
// effective floor; an incoming message snaps the cadence back to ACTIVE on the
// next tick (≤ the current delay of latency). Guest-authored (the host applies
// the returned value verbatim, clamped).
const BG_ACTIVE_MS: u32 = 250; // ~4 Hz right after activity (snappy receive)
const BG_COOL_MS: u32 = 500; // ~2 Hz cooling down
const BG_IDLE_MS: u32 = 1000; // ~1 Hz when quiet (= the host idle cap)
const BG_COOL_AFTER: u32 = 8; // ticks of quiet before cooling
const BG_IDLE_AFTER: u32 = 24; // ticks of quiet before fully idle

dioxus_canvas::wire!(app, pre_frame: |r| {
    r.set_scale(2.0); // hi-dpi panel — author px are small; 2× for readability
    // Pump the engine; a change resets the idle counter (and re-renders).
    let changed = pump();
    if changed {
        r.mark_dirty();
        IDLE_FRAMES.with(|c| c.set(0));
    } else {
        IDLE_FRAMES.with(|c| c.set(c.get().saturating_add(1)));
    }
    // Adaptive poll/repaint cadence: this loop's only job when idle is to keep the
    // live Signal socket serviced + receive promptly, but a fixed 8 fps repaints
    // the whole screen for nothing (~14% CPU). So poll FAST (~8/s) for ~1s after
    // any activity — snappy send/receive bursts — then ramp DOWN to ~2/s when
    // truly idle (incoming still lands within ~0.5s; keepalive is serviced far
    // more often than it needs). Input (scroll/type) drives its own immediate
    // frames, so interactivity is unaffected by the idle floor.
    // NOTE: do NOT speed up RENDERING during a call. The engine's audio/UDP pump
    // runs on the render-INDEPENDENT bg-tick (~60/s, cheap engine steps) — see
    // bg_tick(). The visible call screen changes slowly (timer/state), so it can
    // idle-ramp like any other screen. Forcing 10 ms here made skia re-render the
    // whole UI at ~60 fps → ~250% CPU. Keep render on the idle ramp; bg-tick feeds
    // the audio ring.
    let delay = if r.is_paused() {
        // Backgrounded: the surface is hidden, so every frame's repaint is wasted.
        // Ask for a slow cadence (the host clamps to its ~1/s idle floor) — still
        // ticks the engine so messages arrive in the background, just no longer
        // repaints off-screen at the foreground rate.
        2000
    } else {
        let idle = IDLE_FRAMES.with(|c| c.get());
        if idle < 8 {
            120 // ~8/s, first ~1s after activity
        } else if idle < 24 {
            250 // ~4/s, cooling down
        } else {
            500 // ~2/s, fully idle
        }
    };
    r.set_min_frame_delay(delay);
});

// ── M4: background-receipt exports ───────────────────────────────────────────
// `wire!` above defines `__DioxusCanvasGuest` (the renderer/frame-pacing export
// target) and `export!`s the whole world. The world now also exports background /
// notify-handler / alarm-handler, so we impl those on the SAME guest type here
// (trait impls are crate-wide, order-independent — `export!` finds them).

impl crate::exports::wandr::background::background::Guest for __DioxusCanvasGuest {
    /// Backgrounded: the host calls this instead of render_frame. Pump the engine
    /// (drains the socket → events → notifications) without rendering the hidden
    /// surface, then return an idle-adaptive delay: fast right after activity,
    /// ramping down to ~1 Hz when the socket is quiet (battery).
    fn bg_tick() -> u32 {
        let changed = pump();
        // Active call: pump fast (host clamps to its ~16 ms floor ≈ 60/s) so the
        // ~32 ms audio ring stays fed. The foreground render loop is fps-capped
        // (~20/s) and too slow; bg-tick is render-free and runs in every role now.
        if !matches!(
            chat::call_status(),
            chat::CallState::Idle | chat::CallState::Ended
        ) {
            BG_IDLE.with(|c| c.set(0));
            return 10;
        }
        let idle = if changed {
            BG_IDLE.with(|c| c.set(0));
            0
        } else {
            BG_IDLE.with(|c| {
                let n = c.get().saturating_add(1);
                c.set(n);
                n
            })
        };
        if idle >= BG_IDLE_AFTER {
            BG_IDLE_MS
        } else if idle >= BG_COOL_AFTER {
            BG_COOL_MS
        } else {
            BG_ACTIVE_MS
        }
    }
}

impl crate::exports::wandr::notify::notify_handler::Guest for __DioxusCanvasGuest {
    /// A notification was tapped (the arbiter already foregrounded us). Resolve the
    /// thread it belongs to and request opening it (applied in `app`).
    fn on_notification_click(id: u64) {
        if let Some(thread) = NID_THREAD.with(|m| m.borrow().get(&id).cloned()) {
            PENDING_OPEN.with(|p| *p.borrow_mut() = Some(thread));
        }
    }
}

impl crate::exports::wandr::alarm::alarm_handler::Guest for __DioxusCanvasGuest {
    /// Keep-alive wake. If we were dead the arbiter relaunched us (hidden) and
    /// this runs after `chat::init`; either way force an immediate sync.
    fn on_alarm(_id: u64) {
        pump();
    }
}

// ── palette ─────────────────────────────────────────────────────────────────
const BG: &str = "#0E0E16";
const BAR: &str = "#15151F";
const IN_BUBBLE: &str = "#26263A";
const OUT_BUBBLE: &str = "#2B5278";
const FIELD: &str = "#1E1E30";
const ACCENT: &str = "#4285F4";
const TEXT: &str = "#FFFFFF";
const MUTED: &str = "#9AA0B4";
const SENDER: &str = "#7FA8E0";
const META: &str = "#AEB6C8"; // bubble timestamp + sent/delivered checks
const READ_CHECK: &str = "#8FD0FF"; // read receipt (bright, reads on the blue bubble)

// ── time + delivery rendering ────────────────────────────────────────────────
thread_local! {
    // Device timezone offset (minutes) vs UTC, derived once from the host clock.
    static TZ_OFFSET_MIN: Cell<Option<i64>> = const { Cell::new(None) };
}

fn parse_hhmm(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    Some(h.trim().parse::<i64>().ok()? * 60 + m.trim().parse::<i64>().ok()?)
}

/// Local UTC offset in minutes, derived by comparing the host's local clock
/// (`status::clock-text`) to the guest's UTC wall-clock. Cached once known; 0
/// (UTC) until the host clock is available.
fn local_offset_min() -> i64 {
    TZ_OFFSET_MIN.with(|c| {
        if let Some(o) = c.get() {
            return o;
        }
        let clock = crate::my::skiko_gfx::status::clock_text();
        let utc_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        match parse_hhmm(&clock) {
            Some(local_min) if utc_ms > 0 => {
                let utc_min = ((utc_ms / 60000) % 1440) as i64;
                let mut off = local_min - utc_min;
                if off > 720 {
                    off -= 1440;
                } else if off <= -720 {
                    off += 1440;
                }
                c.set(Some(off));
                off
            },
            _ => 0, // host clock not ready yet — retry next call
        }
    })
}

/// Epoch-ms → local "HH:MM".
fn fmt_time(ts: u64) -> String {
    let local_ms = ts as i64 + local_offset_min() * 60_000;
    let mins = (((local_ms / 60_000) % 1440) + 1440) % 1440;
    format!("{:02}:{:02}", mins / 60, mins % 60)
}

/// Delivery rank → (glyph, color) for an outgoing bubble's receipt indicator.
fn check_marks(status: u8) -> (&'static str, &'static str) {
    match status {
        0 => ("·", META),          // sending
        1 => ("✓", META),          // sent (server accepted)
        2 => ("✓✓", META),         // delivered to the recipient's device
        _ => ("✓✓", READ_CHECK),   // read
    }
}

const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Local day index (days since the epoch in the device's timezone) — the key for
/// grouping messages under a date divider.
fn local_day(ts: u64) -> i64 {
    (ts as i64 + local_offset_min() * 60_000).div_euclid(86_400_000)
}

/// Civil (year, month, day) from a days-since-epoch index (Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

/// A date-divider label: "Today" / "Yesterday" / "D Mon YYYY".
fn date_label(day: i64) -> String {
    let today = local_day(now_ms());
    if day == today {
        return "Today".to_string();
    }
    if day == today - 1 {
        return "Yesterday".to_string();
    }
    let (y, m, d) = civil_from_days(day);
    let mon = MONTHS.get((m - 1) as usize).copied().unwrap_or("");
    format!("{d} {mon} {y}")
}

// ── the model the components read, updated by `pump` from the engine ──────────
#[derive(Clone, PartialEq, Default)]
struct UiMsg {
    id: u64,
    sender: String,
    text: String,
    outgoing: bool,
    /// Conversation key (peer ACI / group master key b64).
    thread: String,
    /// Wire timestamp (epoch ms) — formatted to local HH:MM for display.
    ts: u64,
    /// Delivery rank: 0 sending, 1 sent, 2 delivered, 3 read.
    status: u8,
    /// Emoji reactions on this message (distinct emojis concatenated); empty = none.
    reactions: String,
    /// Image attachments, ready to render (`data:` URI + display box in logical px).
    images: Vec<UiImage>,
    /// Call-history entry (renders as a call log, not a bubble); `None` = a message.
    call: Option<chat::CallLog>,
}

/// A renderable image attachment: a `data:` URI plus the display box (logical px,
/// derived from the source dims, capped to fit the bubble while keeping aspect).
/// The URI is `Rc`-shared so the per-frame `snapshot()` clone is a refcount bump,
/// not a memcpy of the (large) base64 payload.
#[derive(Clone, PartialEq, Default)]
struct UiImage {
    uri: Rc<String>,
    w: u32,
    h: u32,
}

/// Build renderable images from a message's attachments: keep `image/*`, encode
/// the bytes as a base64 `data:` URI, and fit each into a max-width box (logical
/// px) preserving aspect (falling back to a default box when dims are unknown).
fn ui_images(attachments: &[chat::Attachment]) -> Vec<UiImage> {
    use base64::Engine as _;
    const MAX_W: u32 = 460;
    const DEFAULT_W: u32 = 440;
    const DEFAULT_H: u32 = 330;
    attachments
        .iter()
        .filter(|a| a.content_type.starts_with("image/"))
        .map(|a| {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&a.data);
            let uri = Rc::new(format!("data:{};base64,{}", a.content_type, b64));
            let (w, h) = if a.width > 0 && a.height > 0 {
                let w = a.width.min(MAX_W);
                (w, (w as u64 * a.height as u64 / a.width as u64) as u32)
            } else {
                (DEFAULT_W, DEFAULT_H)
            };
            UiImage { uri, w, h }
        })
        .collect()
}

/// chat::Delivery → rank (0 sending … 3 read).
fn status_rank(d: chat::Delivery) -> u8 {
    match d {
        chat::Delivery::Sending => 0,
        chat::Delivery::Sent => 1,
        chat::Delivery::Delivered => 2,
        chat::Delivery::Read => 3,
    }
}

/// An open conversation: which thread the user tapped into.
#[derive(Clone, PartialEq)]
struct Thread {
    id: String,
    title: String,
    is_group: bool,
}

#[derive(Clone, PartialEq, Default)]
struct UiContact {
    id: String,
    name: String,
    phone: Option<String>,
    /// `data:…;base64,…` URI for `img { src }`, encoded once when contacts load.
    avatar_uri: Option<String>,
}

#[derive(Clone, PartialEq, Default)]
struct UiGroup {
    id: String,
    title: String,
    members: Vec<String>,
    /// `data:…;base64,…` URI for `img { src }`, encoded once when groups load.
    avatar_uri: Option<String>,
}

/// Our own profile for display (name + phone + avatar data-uri). `Rc` avatar so
/// the per-frame snapshot clone is cheap.
#[derive(Clone, PartialEq, Default)]
struct UiProfile {
    name: String,
    phone: String,
    /// Profile bio/status text + its emoji (both optional).
    about: String,
    about_emoji: String,
    avatar: Option<Rc<String>>,
}

/// Build a `UiProfile` from the engine's profile record; `None` until anything is
/// known (still connecting / not yet fetched).
fn ui_profile(p: chat::Profile) -> Option<UiProfile> {
    let name = format!("{} {}", p.given_name, p.family_name).trim().to_string();
    if name.is_empty() && p.phone.is_empty() && p.avatar.is_none() {
        return None;
    }
    let avatar = p.avatar.filter(|a| !a.is_empty()).map(|bytes| {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Rc::new(format!("data:image/jpeg;base64,{}", b64))
    });
    Some(UiProfile {
        name,
        phone: p.phone,
        about: p.about,
        about_emoji: p.about_emoji,
        avatar,
    })
}

#[derive(Default)]
struct Model {
    state: String,
    link_url: Option<String>,
    messages: Vec<UiMsg>,
    contacts: Vec<UiContact>,
    groups: Vec<UiGroup>,
    /// Our own Signal profile (name/phone/avatar), once fetched.
    my_profile: Option<UiProfile>,
    /// This account's own ACI — the Note-to-Self thread id.
    account_id: String,
    /// Per-thread unread count (incoming messages arrived while not viewing that
    /// thread). Keyed by thread id; cleared when the thread is opened.
    unread: std::collections::HashMap<String, u32>,
}

thread_local! {
    /// The thread currently open (so `pump`, which runs outside the dioxus
    /// runtime, knows not to count incoming messages there as unread). Set when a
    /// thread is opened, cleared on back.
    static VIEWING: RefCell<Option<String>> = const { RefCell::new(None) };
    /// M4 — notification id → thread, so an `on-notification-click` tap resolves
    /// back to which conversation to open. One notification per thread.
    static NID_THREAD: RefCell<std::collections::HashMap<u64, String>> =
        RefCell::new(std::collections::HashMap::new());
    /// M4 — a thread the user asked to open via a notification tap (set outside
    /// the dioxus runtime by `on-notification-click`; applied in `app`).
    static PENDING_OPEN: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// M4 — stable per-thread notification id (one notification per conversation,
/// updated in place rather than spamming). FNV-1a over the thread key.
fn thread_nid(thread: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce4_84222325;
    for b in thread.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Open a conversation: mark it the viewed thread, clear its unread badge, and
/// send read receipts for its received messages (the peer sees "read").
fn open_thread(id: &str) {
    VIEWING.with(|v| *v.borrow_mut() = Some(id.to_string()));
    MODEL.with(|m| {
        m.borrow_mut().unread.remove(id);
    });
    chat::mark_read(id);
    // M4 — viewing a thread clears its notification.
    let nid = thread_nid(id);
    notifier::cancel(nid);
    NID_THREAD.with(|m| {
        m.borrow_mut().remove(&nid);
    });
}

/// Leave the open conversation (back to the list).
fn close_thread() {
    VIEWING.with(|v| *v.borrow_mut() = None);
}

/// Fetch the engine's groups, sorted by title. Built off the dioxus runtime.
fn load_groups() -> Vec<UiGroup> {
    use base64::Engine as _;
    let mut v: Vec<UiGroup> = chat::groups()
        .into_iter()
        .map(|g| UiGroup {
            avatar_uri: g.avatar.map(|b| {
                format!(
                    "data:image/jpeg;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(b)
                )
            }),
            id: g.id,
            title: g.title,
            members: g.members,
        })
        .collect();
    v.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    v
}

/// Fetch the engine's contacts, encode avatars into data URIs (once), sort by
/// name. Built off the dioxus runtime (called from `pump`).
fn load_contacts() -> Vec<UiContact> {
    use base64::Engine as _;
    let mut v: Vec<UiContact> = chat::contacts()
        .into_iter()
        .map(|c| UiContact {
            avatar_uri: c.avatar.map(|b| {
                format!(
                    "data:image/jpeg;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(b)
                )
            }),
            id: c.id,
            name: c.name,
            phone: c.phone,
        })
        .collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

thread_local! {
    static MODEL: RefCell<Model> = RefCell::new(Model::default());
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// Init the engine once, then drain its events + refresh state into `MODEL`.
/// Returns true if anything changed (→ the renderer should re-diff). Runs every
/// frame from the `pre_frame` hook (outside the dioxus runtime — globals only).
fn pump() -> bool {
    STARTED.with(|s| {
        if !s.get() {
            chat::init();
            s.set(true);
            // M4 — arm the keep-alive alarm so a dead Signal (crash/OOM/reboot)
            // is relaunched + reconnects. Idempotent on the arbiter side.
            scheduler::schedule(KEEPALIVE_ID, KEEPALIVE_MS, KEEPALIVE_MS);
            // Backfill persisted history (messages from before this UI started;
            // live ones arrive as events below).
            let hist = chat::history();
            let contacts = load_contacts();
            let groups = load_groups();
            let my_profile = ui_profile(chat::my_profile());
            MODEL.with(|m| {
                let mut m = m.borrow_mut();
                m.contacts = contacts;
                m.groups = groups;
                m.my_profile = my_profile;
                for msg in hist {
                    let images = ui_images(&msg.attachments);
                    m.messages.push(UiMsg {
                        id: msg.id,
                        sender: msg.sender,
                        text: msg.text,
                        outgoing: msg.outgoing,
                        thread: msg.thread,
                        ts: msg.ts,
                        status: status_rank(msg.status),
                        reactions: msg.reactions,
                        images,
                        call: msg.call,
                    });
                }
            });
        }
    });

    let mut changed = false;
    let mut refresh = false;
    let mut refresh_groups = false;
    // M4 — (thread, body-preview) for each new inbound message not in the open
    // thread; resolved to titles + posted as notifications after the borrow ends.
    let mut to_notify: Vec<(String, String)> = Vec::new();
    let events = chat::poll_events();
    if !events.is_empty() {
        MODEL.with(|m| {
            let mut m = m.borrow_mut();
            for e in events {
                match e {
                    chat::Event::Message(msg) => {
                        // Dedup by id (a live message is also in history()).
                        if !m.messages.iter().any(|x| x.id == msg.id) {
                            // Incoming message in a thread we're not viewing → unread.
                            if !msg.outgoing {
                                let viewing = VIEWING.with(|v| v.borrow().clone());
                                if viewing.as_deref() != Some(msg.thread.as_str()) {
                                    *m.unread.entry(msg.thread.clone()).or_insert(0) += 1;
                                    // M4 — alert the user (works backgrounded too).
                                    let preview: String = if msg.text.is_empty() && !msg.attachments.is_empty() {
                                        "\u{1F4F7} Photo".to_string()
                                    } else {
                                        msg.text.chars().take(120).collect()
                                    };
                                    to_notify.push((msg.thread.clone(), preview));
                                } else {
                                    // Arrived in the open thread → it's read now.
                                    chat::mark_read(&msg.thread);
                                }
                            }
                            let images = ui_images(&msg.attachments);
                            m.messages.push(UiMsg {
                                id: msg.id,
                                sender: msg.sender,
                                text: msg.text,
                                outgoing: msg.outgoing,
                                thread: msg.thread,
                                ts: msg.ts,
                                status: status_rank(msg.status),
                                reactions: msg.reactions,
                                images,
                        call: msg.call,
                            });
                        }
                    }
                    // A delivery/read receipt advanced an outgoing message.
                    chat::Event::StatusChanged(ds) => {
                        if let Some(x) = m.messages.iter_mut().find(|x| x.id == ds.id) {
                            x.status = status_rank(ds.status);
                        }
                    }
                    // Someone added/removed an emoji reaction on a message.
                    chat::Event::ReactionChanged(ru) => {
                        if let Some(x) = m.messages.iter_mut().find(|x| x.id == ru.id) {
                            x.reactions = ru.reactions;
                        }
                    }
                    chat::Event::LinkUrl(url) => m.link_url = Some(url),
                    chat::Event::Linked(_) | chat::Event::Connected => {
                        m.link_url = None;
                    }
                    // Our own profile was fetched/updated.
                    chat::Event::ProfileUpdated => {
                        m.my_profile = ui_profile(chat::my_profile());
                    }
                    chat::Event::Disconnected => {}
                    // Re-fetched below (load_* needs no &mut m borrow).
                    chat::Event::ContactsUpdated(_) => refresh = true,
                    chat::Event::GroupsUpdated(_) => refresh_groups = true,
                    // Voice-call events (Phase 2b-ii). The in-call UI is a
                    // follow-up (step 4); for now the engine drives the call and
                    // these are surfaced via `chat::call-status`/`call-peer`.
                    chat::Event::CallIncoming(_) | chat::Event::CallStateChanged(_) => {}
                }
            }
        });
        changed = true;
    }

    if refresh {
        let contacts = load_contacts();
        MODEL.with(|m| m.borrow_mut().contacts = contacts);
    }
    if refresh_groups {
        let groups = load_groups();
        MODEL.with(|m| m.borrow_mut().groups = groups);
    }

    // M4 — post one notification per thread that got a new (unviewed) message.
    // Resolved here (after the MODEL borrow) so titles use the latest contacts.
    if !to_notify.is_empty() {
        let (self_id, contacts, groups) = MODEL.with(|m| {
            let m = m.borrow();
            (m.account_id.clone(), m.contacts.clone(), m.groups.clone())
        });
        for (thread, body) in to_notify {
            let (title, _is_group) = resolve_thread(&thread, &self_id, &contacts, &groups);
            let nid = thread_nid(&thread);
            notifier::post(nid, &title, &body);
            NID_THREAD.with(|m| {
                m.borrow_mut().insert(nid, thread);
            });
        }
    }

    let state = chat::state();
    let aid = chat::account_id();
    MODEL.with(|m| {
        let mut m = m.borrow_mut();
        if m.state != state {
            m.state = state;
            changed = true;
        }
        // Becomes known after resume/connect; latch it once.
        if !aid.is_empty() && m.account_id != aid {
            m.account_id = aid;
            changed = true;
        }
    });
    changed
}

type Snapshot = (
    String,
    Option<String>,
    Vec<UiMsg>,
    Vec<UiContact>,
    Vec<UiGroup>,
    String,
    std::collections::HashMap<String, u32>,
    Option<UiProfile>,
);
fn snapshot() -> Snapshot {
    MODEL.with(|m| {
        let m = m.borrow();
        (
            m.state.clone(),
            m.link_url.clone(),
            m.messages.clone(),
            m.contacts.clone(),
            m.groups.clone(),
            m.account_id.clone(),
            m.unread.clone(),
            m.my_profile.clone(),
        )
    })
}

/// Send the composer's current text through the engine, then clear it. Signals
/// are `Copy`, passed by value. The engine echoes the message back as an outgoing
/// event, so we don't append it locally.
fn submit(thread: String, mut value: Signal<String>, mut caret: Signal<usize>) {
    let t = value().trim().to_string();
    if !t.is_empty() {
        let _ = chat::send(&thread, &t);
        value.set(String::new());
        caret.set(0);
    }
}

/// Map a tap x (element-relative logical px) to the nearest caret index in the
/// composer value, measuring substrings at the field font (34px). 24px left inset
/// matches the renderer's `paint_input`.
fn caret_at(value: &str, x: f32) -> usize {
    let x = (x - 24.0).max(0.0);
    let chars: Vec<char> = value.chars().collect();
    let (mut best, mut best_d) = (0usize, f32::MAX);
    for i in 0..=chars.len() {
        let prefix: String = chars[..i].iter().collect();
        let w = if prefix.is_empty() {
            0.0
        } else {
            measure_text(&prefix, "sans-serif", 34.0, 400, false).0
        };
        let d = (w - x).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// Short, stable label for a raw ServiceId/ACI sender string (v1 — no name
/// resolution). `<ACI:3859f52f-…>` → `3859f52f`.
fn short_sender(s: &str) -> String {
    let inner = s.trim_start_matches('<').trim_end_matches('>');
    let inner = inner.split(':').nth(1).unwrap_or(inner);
    inner.chars().take(8).collect()
}

/// Resolve a thread id → (display title, is_group). Group ids match `group.id`;
/// 1:1 ids are a peer ACI resolved against contacts (else a short uuid).
fn resolve_thread(
    id: &str,
    self_id: &str,
    contacts: &[UiContact],
    groups: &[UiGroup],
) -> (String, bool) {
    if let Some(g) = groups.iter().find(|g| g.id == id) {
        return (g.title.clone(), true);
    }
    if id.is_empty() || (!self_id.is_empty() && id == self_id) {
        return ("Note to Self".to_string(), false);
    }
    let title = contacts
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| short_sender(id));
    (title, false)
}

/// Avatar data-uri for a thread id (the matching contact's / group's image).
fn thread_avatar(id: &str, contacts: &[UiContact], groups: &[UiGroup]) -> Option<String> {
    groups
        .iter()
        .find(|g| g.id == id)
        .and_then(|g| g.avatar_uri.clone())
        .or_else(|| {
            contacts.iter().find(|c| c.id == id).and_then(|c| c.avatar_uri.clone())
        })
}

// ── components ────────────────────────────────────────────────────────────────
fn app() -> Element {
    // Stay armed: re-schedule this scope every render so engine updates (pushed
    // into MODEL by `pump` + flagged via `mark_dirty`) re-run this body and reach
    // the tree.
    dioxus::core::needs_update();

    // 0 = conversations, 1 = contacts (groups folded in at the top).
    let mut tab = use_signal(|| 0u8);
    // None = list view; Some = an open conversation (full-screen thread view).
    let current = use_signal(|| None::<Thread>);
    // My-profile screen overlay.
    let show_profile = use_signal(|| false);

    let (state, link_url, messages, contacts, groups, self_id, unread, my_profile) = snapshot();

    // Call state read HERE (app re-renders on engine events) and passed down as
    // props, so the memoized CallBar/ThreadHeader actually re-render when it changes.
    let call_status = chat::call_status();
    let call_peer = chat::call_peer();

    // Task 93 Phase 5 — a connected call with video (ours or theirs) takes the
    // whole screen: the host composites the remote + PiP video surfaces above
    // this app's UI (above-ui layer), inside rects this screen lays out.
    let (vid_local, vid_remote) = chat::video_status();
    if matches!(call_status, chat::CallState::Connected) && (vid_local || vid_remote) {
        return rsx! { VideoCallScreen { local_on: vid_local } };
    }

    // M4 — a notification tap asked to open a thread (set by on-notification-click,
    // outside the runtime). Resolve + navigate to it now.
    if let Some(tid) = PENDING_OPEN.with(|p| p.borrow_mut().take()) {
        let (title, is_group) = resolve_thread(&tid, &self_id, &contacts, &groups);
        open_thread(&tid);
        let mut current = current;
        current.set(Some(Thread { id: tid, title, is_group }));
    }

    // Not linked yet → just the QR.
    if let Some(url) = link_url {
        return rsx! {
            div {
                style: "display:flex; flex-direction:column; height:100%; background:{BG};",
                TitleBar { state, my_profile: None, show_profile }
                LinkPanel { url }
            }
        };
    }

    // My Profile screen (full-screen).
    if show_profile() {
        if let Some(p) = my_profile.clone() {
            return rsx! { ProfileScreen { profile: p, show_profile } };
        }
    }

    // An open conversation → header + that thread's messages + composer.
    if let Some(thread) = current() {
        let thread_msgs: Vec<UiMsg> =
            messages.iter().filter(|m| m.thread == thread.id).cloned().collect();
        return rsx! {
            div {
                style: "display:flex; flex-direction:column; height:100%; background:{BG};",
                CallBar { status: call_status }
                CallControls { status: call_status }
                ThreadHeader { title: thread.title.clone(), current, call_status, call_peer: call_peer.clone(), is_self: thread.id == self_id }
                Conversation { messages: thread_msgs, contacts: contacts.clone(), stick_key: thread.id.clone() }
                Composer { thread: thread.id.clone() }
            }
        };
    }

    let nc = contacts.len() + groups.len();
    let tab_bg = |i: u8| if tab() == i { ACCENT } else { BAR };
    rsx! {
        div {
            style: "display:flex; flex-direction:column; height:100%; background:{BG};",
            CallBar { status: call_status }
            TitleBar { state, my_profile: my_profile.clone(), show_profile }
            div {
                style: "display:flex; flex-direction:row; gap:12px; padding:14px 24px; background:{BAR};",
                button {
                    style: format!("display:flex; justify-content:center; flex-grow:1; padding:14px; border-radius:14px; background:{};", tab_bg(0)),
                    onclick: move |_| tab.set(0),
                    div { style: "color:{TEXT}; font-size:28px;", "Chats" }
                }
                button {
                    style: format!("display:flex; justify-content:center; flex-grow:1; padding:14px; border-radius:14px; background:{};", tab_bg(1)),
                    onclick: move |_| tab.set(1),
                    div { style: "color:{TEXT}; font-size:28px;", "Contacts ({nc})" }
                }
            }
            if tab() == 0 {
                Conversations { messages, contacts: contacts.clone(), groups: groups.clone(), self_id: self_id.clone(), unread: unread.clone(), current }
            } else {
                Contacts { contacts, groups, self_id: self_id.clone(), unread: unread.clone(), current }
            }
        }
    }
}

/// The app title bar (name + connection state + a tappable own-profile avatar).
#[component]
fn TitleBar(state: String, my_profile: Option<UiProfile>, show_profile: Signal<bool>) -> Element {
    let mut show_profile = show_profile;
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; gap:16px; padding:28px; background:{BAR};",
            div { style: "color:{TEXT}; font-size:44px; font-weight:700; flex-grow:1;", "Signal" }
            div { style: "color:{MUTED}; font-size:26px;", "{state}" }
            // Own-profile avatar → opens the My Profile screen.
            if let Some(p) = my_profile {
                button {
                    style: "display:flex; align-items:center; justify-content:center; width:72px; height:72px; border-radius:50%; background:{OUT_BUBBLE};",
                    onmousedown: move |_| {},
                    onclick: move |_| show_profile.set(true),
                    if let Some(uri) = p.avatar.clone() {
                        img { src: "{uri}", style: "width:72px; height:72px; border-radius:50%;" }
                    } else {
                        div { style: "color:{TEXT}; font-size:34px; font-weight:700;", "{initial(&p.name)}" }
                    }
                }
            }
        }
    }
}

/// The My Profile screen: large avatar, name, phone, and a back button.
#[component]
fn ProfileScreen(profile: UiProfile, show_profile: Signal<bool>) -> Element {
    let mut show_profile = show_profile;
    let name = if profile.name.is_empty() { "(no name)".to_string() } else { profile.name.clone() };
    rsx! {
        div {
            style: "display:flex; flex-direction:column; height:100%; background:{BG};",
            // Header: back arrow + title.
            div {
                style: "display:flex; flex-direction:row; align-items:center; gap:20px; padding:22px 24px; background:{BAR};",
                button {
                    style: format!("display:flex; justify-content:center; align-items:center; width:64px; height:64px; border-radius:14px; background:{};", FIELD),
                    onmousedown: move |_| {},
                    onclick: move |_| show_profile.set(false),
                    div { style: "color:{TEXT}; font-size:36px;", "‹" }
                }
                div { style: "color:{TEXT}; font-size:36px; font-weight:700;", "My Profile" }
            }
            // Body: avatar, name, phone — centered.
            div {
                style: "display:flex; flex-direction:column; align-items:center; gap:24px; padding:48px 24px;",
                if let Some(uri) = profile.avatar.clone() {
                    img { src: "{uri}", style: "width:320px; height:320px; border-radius:50%;" }
                } else {
                    div {
                        style: "display:flex; align-items:center; justify-content:center; width:320px; height:320px; border-radius:50%; background:{OUT_BUBBLE};",
                        div { style: "color:{TEXT}; font-size:140px; font-weight:700;", "{initial(&name)}" }
                    }
                }
                div { style: "color:{TEXT}; font-size:44px; font-weight:700;", "{name}" }
                // About: bio text with its emoji (skip when both empty).
                if !profile.about.is_empty() || !profile.about_emoji.is_empty() {
                    div {
                        style: "display:flex; flex-direction:row; align-items:center; gap:10px; max-width:90%;",
                        if !profile.about_emoji.is_empty() {
                            div { style: "font-size:32px;", "{profile.about_emoji}" }
                        }
                        if !profile.about.is_empty() {
                            div { style: "color:{TEXT}; font-size:30px; white-space:normal;", "{profile.about}" }
                        }
                    }
                }
                if !profile.phone.is_empty() {
                    div { style: "color:{MUTED}; font-size:30px;", "{profile.phone}" }
                }
            }
        }
    }
}


/// Task 93 Phase 5 — the in-call video screen. The video itself is composited
/// by the HOST (above-ui media surfaces); this screen owns the geometry: it
/// derives the remote + PiP rects from the live surface size and its own
/// layout constants, reports them via `set-video-layout`, and draws the
/// controls strip. Camera toggle + hang up.
#[component]
fn VideoCallScreen(local_on: bool) -> Element {
    // Surface pixels — the same space the video rects use.
    let sw = crate::my::skiko_gfx::canvas::surface_width();
    let sh = crate::my::skiko_gfx::canvas::surface_height();
    // UI-owned call-screen layout policy: a bottom controls strip; remote
    // video fills everything above it; the PiP sits above the strip, right-
    // aligned, at the capture aspect (4:3), a quarter of the width.
    const CONTROLS_H: u32 = 220;
    const MARGIN: u32 = 24;
    let video_h = sh.saturating_sub(CONTROLS_H);
    let pip_w = sw / 4;
    let pip_h = pip_w * 3 / 4;
    chat::set_video_layout(
        0, 0, sw, video_h,
        sw.saturating_sub(pip_w + MARGIN),
        video_h.saturating_sub(pip_h + MARGIN),
        pip_w, pip_h,
    );
    let btn = "display:flex; justify-content:center; align-items:center; width:128px; height:128px; border-radius:28px; color:#FFFFFF; font-size:60px;";
    let cam_bg = if local_on { "#2E7D32" } else { "#555555" };
    rsx! {
        div {
            style: "display:flex; flex-direction:column; height:100%; background:#000000;",
            // The region the video surfaces cover (host-composited).
            div { style: "flex-grow:1;" }
            div {
                style: format!("display:flex; flex-direction:row; justify-content:center; align-items:center; gap:48px; height:{CONTROLS_H}px; background:{BAR};"),
                button {
                    style: format!("{btn} background:{cam_bg};"),
                    onmousedown: move |_| {},
                    onclick: move |_| chat::set_video(!local_on),
                    "📷"
                }
                button {
                    style: format!("{btn} background:#C62828;"),
                    onmousedown: move |_| {},
                    onclick: move |_| chat::hangup_call(),
                    "📵"
                }
            }
        }
    }
}

/// Header for an open conversation: a back arrow (clears `current`) + the title.
#[component]
fn ThreadHeader(
    title: String,
    current: Signal<Option<Thread>>,
    call_status: chat::CallState,
    call_peer: String,
    is_self: bool,
) -> Element {
    let mut current = current;
    // The open thread's id + kind, for the 1:1 call button (Phase 2b-ii).
    let (thread_id, is_group) = current().map(|t| (t.id, t.is_group)).unwrap_or_default();
    let call_tid = thread_id.clone();
    // The handset shows for a real 1:1 peer only — not groups, and not Note-to-Self
    // (you can't call yourself). GREEN to place / RED to hang up (props-driven).
    let can_call = !is_group && !is_self && !thread_id.is_empty();
    let in_call = can_call
        && !matches!(call_status, chat::CallState::Idle | chat::CallState::Ended)
        && call_peer == thread_id;
    let handset_bg = if in_call { "#C62828" } else { "#2E7D32" }; // red end / green call
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; gap:20px; padding:22px 24px; background:{BAR};",
            button {
                style: format!("display:flex; justify-content:center; align-items:center; width:64px; height:64px; border-radius:14px; background:{};", FIELD),
                onclick: move |_| { close_thread(); current.set(None); },
                div { style: "color:{TEXT}; font-size:36px;", "‹" }
            }
            div { style: "color:{TEXT}; font-size:36px; font-weight:700; flex-grow:1;", "{title}" }
            // 1:1 voice call (not groups, not Note-to-Self): big green→place / red→hang-up.
            if in_call {
                button {
                    style: "display:flex; justify-content:center; align-items:center; width:128px; height:128px; border-radius:28px; background:#2E7D32;",
                    onmousedown: move |_| {},
                    onclick: move |_| chat::set_video(true),
                    div { style: "color:#FFFFFF; font-size:64px;", "📷" }
                }
            }
            if can_call {
                button {
                    style: format!("display:flex; justify-content:center; align-items:center; width:128px; height:128px; border-radius:28px; background:{};", handset_bg),
                    onmousedown: move |_| {},
                    onclick: move |_| {
                        if in_call { chat::hangup_call(); } else { let _ = chat::place_call(&call_tid); }
                    },
                    div { style: "color:#FFFFFF; font-size:64px;", "📞" }
                }
            }
        }
    }
}

/// The active-call banner (Phase 2b-ii): shown whenever a 1:1 voice call is in
/// progress. Polls the engine's call state each render (the app re-renders on
/// engine events). Ringing → big Accept/Decline; otherwise → status only (end the
/// call with the red handset in the thread header — no separate End button).
#[component]
fn CallBar(status: chat::CallState) -> Element {
    if matches!(status, chat::CallState::Idle | chat::CallState::Ended) {
        return rsx! {};
    }
    let label = match status {
        chat::CallState::Outgoing => "Calling…",
        chat::CallState::Ringing => "Incoming call",
        chat::CallState::Connecting => "Connecting…",
        chat::CallState::Connected => "On call",
        _ => "",
    };
    let btn = "display:flex; justify-content:center; align-items:center; width:128px; height:128px; border-radius:28px; color:#FFFFFF; font-size:60px;";
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; gap:20px; padding:18px 24px; background:{ACCENT};",
            div { style: "color:#FFFFFF; font-size:32px; font-weight:700; flex-grow:1;", "{label}" }
            // Only incoming calls need controls here: big green Accept + red Decline
            // handsets (active calls are ended via the red handset in the thread).
            if matches!(status, chat::CallState::Ringing) {
                button {
                    style: format!("{btn} background:#2E7D32;"),
                    onmousedown: move |_| {},
                    onclick: move |_| chat::accept_call(),
                    "📞"
                }
                button {
                    style: format!("{btn} background:#C62828;"),
                    onmousedown: move |_| {},
                    onclick: move |_| chat::hangup_call(),
                    "📵"
                }
            }
        }
    }
}

/// In-call audio controls panel (Phase 2b-ii) — shown while a call is active
/// (outgoing/connecting/connected). Mic-mute, earpiece↔loudspeaker route, and a
/// volume slider, driven by `wandr:audio-focus/controls` (the host applies; see
/// project_call_audioserver_crash — routing here does NOT touch setPhoneState). Local
/// signals seed from the host's current applied state via the `get-*` reads.
#[component]
fn CallControls(status: chat::CallState) -> Element {
    use wandr::audio_focus::controls as ctl;
    // Only meaningful during an active call. Ringing shows Accept/Decline in CallBar.
    if !matches!(
        status,
        chat::CallState::Outgoing | chat::CallState::Connecting | chat::CallState::Connected
    ) {
        return rsx! {};
    }
    let mut mic_muted = use_signal(|| ctl::get_mic_mute());
    let mut speaker = use_signal(|| matches!(ctl::get_route(), ctl::AudioRoute::Speaker));
    let mut volume = use_signal(|| ctl::get_volume());

    // Fixed slider-track width (one named UI layout constant): the panel pads the
    // surface, so the track is content-sized, and `element_coordinates().x` is in this
    // same layout-px space → level = x / VOL_TRACK_W.
    const VOL_TRACK_W: f32 = 1000.0;
    let icon = "display:flex; justify-content:center; align-items:center; width:104px; height:104px; border-radius:26px; font-size:48px; color:#FFFFFF;";
    let fill_pct = (volume() * 100.0).clamp(0.0, 100.0) as i32;

    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:18px; padding:20px 24px; background:#101418;",
            // Route label — which output the call audio is on.
            div {
                style: "color:#90A4AE; font-size:26px; text-align:center;",
                if speaker() { "Loudspeaker" } else { "Earpiece" }
            }
            // Mic-mute + earpiece/loudspeaker toggle.
            div {
                style: "display:flex; flex-direction:row; gap:28px; justify-content:center; align-items:center;",
                button {
                    style: format!("{icon} background:{};", if mic_muted() { "#C62828" } else { "#37474F" }),
                    onmousedown: move |_| {},
                    onclick: move |_| { let m = !mic_muted(); mic_muted.set(m); ctl::set_mic_mute(m); },
                    if mic_muted() { "🔇" } else { "🎤" }
                }
                button {
                    style: format!("{icon} background:{};", if speaker() { "#2E7D32" } else { "#37474F" }),
                    onmousedown: move |_| {},
                    onclick: move |_| {
                        let s = !speaker();
                        speaker.set(s);
                        ctl::set_route(if s { ctl::AudioRoute::Speaker } else { ctl::AudioRoute::Earpiece });
                    },
                    if speaker() { "🔊" } else { "📞" }
                }
            }
            // Volume slider — drag/tap the track (onmousedown + onmousemove).
            div {
                style: "display:flex; flex-direction:row; align-items:center; gap:18px;",
                div { style: "color:#90A4AE; font-size:40px;", "🔉" }
                div {
                    style: format!("display:flex; align-items:center; width:{VOL_TRACK_W}px; height:44px; border-radius:22px; background:#37474F;"),
                    onmousedown: move |e| { let l = (e.element_coordinates().x as f32 / VOL_TRACK_W).clamp(0.0, 1.0); volume.set(l); ctl::set_volume(l); },
                    onmousemove: move |e| { let l = (e.element_coordinates().x as f32 / VOL_TRACK_W).clamp(0.0, 1.0); volume.set(l); ctl::set_volume(l); },
                    div { style: format!("height:44px; width:{fill_pct}%; border-radius:22px; background:{ACCENT};") }
                }
            }
        }
    }
}

/// The conversation list: one row per thread that has messages, newest first,
/// showing avatar + title + the last message preview. Tap → open that thread.
#[component]
fn Conversations(
    messages: Vec<UiMsg>,
    contacts: Vec<UiContact>,
    groups: Vec<UiGroup>,
    self_id: String,
    unread: std::collections::HashMap<String, u32>,
    current: Signal<Option<Thread>>,
) -> Element {
    // Last message per thread, in arrival order (history is chronological).
    let mut order: Vec<String> = Vec::new();
    let mut last: std::collections::HashMap<String, UiMsg> = std::collections::HashMap::new();
    for m in &messages {
        if !last.contains_key(&m.thread) {
            order.push(m.thread.clone());
        }
        last.insert(m.thread.clone(), m.clone());
    }
    order.reverse(); // newest-active threads first

    rsx! {
        div {
            style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1; min-height:0; padding:16px; gap:10px;",
            if order.is_empty() {
                div { style: "color:{MUTED}; font-size:28px; padding:24px;", "No conversations yet. Open a contact to start one." }
            }
            for tid in order {
                {
                    let mut current = current;
                    let (title, is_group) = resolve_thread(&tid, &self_id, &contacts, &groups);
                    let avatar = thread_avatar(&tid, &contacts, &groups);
                    let preview = last.get(&tid).map(|m| {
                        let body: String = if m.text.is_empty() && !m.images.is_empty() {
                            "📷 Photo".to_string()
                        } else {
                            m.text.chars().take(48).collect()
                        };
                        if m.outgoing { format!("You: {body}") } else { body }
                    }).unwrap_or_default();
                    let t = Thread { id: tid.clone(), title: title.clone(), is_group };
                    let n = unread.get(&tid).copied().unwrap_or(0);
                    let oid = tid.clone();
                    rsx! {
                        button {
                            key: "{tid}",
                            style: "display:flex; flex-direction:row; align-items:center; gap:20px; padding:14px; border-radius:16px; background:{IN_BUBBLE};",
                            onclick: move |_| { open_thread(&oid); current.set(Some(t.clone())); },
                            Avatar { uri: avatar, letter: initial(&title) }
                            div {
                                style: "display:flex; flex-direction:column; gap:4px; flex-grow:1; min-width:0;",
                                div { style: "color:{TEXT}; font-size:32px;", "{title}" }
                                div { style: "color:{MUTED}; font-size:24px;", "{preview}" }
                            }
                            UnreadBadge { count: n }
                        }
                    }
                }
            }
        }
    }
}

/// First-run linking: the engine emitted a `link-url` — drawn as an in-canvas QR
/// (see `QrView`) for the user to scan directly off the panel.
#[component]
fn LinkPanel(url: String) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; align-items:center; gap:14px; padding:24px; background:{FIELD};",
            div { style: "color:{TEXT}; font-size:30px; font-weight:600;", "Link this device" }
            div { style: "color:{MUTED}; font-size:24px;", "Signal -> Settings -> Linked devices -> scan:" }
            QrView { url }
        }
    }
}

/// The provisioning code as an in-canvas QR. No image primitive needed: each
/// module row is run-length-merged into a few solid divs (a ~45-module code →
/// some hundreds of divs), laid out ONCE when the link-url arrives (the engine
/// emits it a single time, so there's no per-frame relayout). White quiet-zone
/// border so scanners lock on.
#[component]
fn QrView(url: String) -> Element {
    let code = match qrcode::QrCode::new(url.as_bytes()) {
        Ok(c) => c,
        Err(_) => {
            return rsx! { div { style: "color:#FF6B6B; font-size:26px;", "QR encode failed" } };
        }
    };
    let w = code.width();
    let colors = code.to_colors();
    // Logical px/module. The renderer scales by 2× (set_scale), so keep the
    // logical width ~640 → ~1280 physical, fitting the 1440px panel.
    let module = (640 / w as i32).max(4);
    rsx! {
        div {
            style: format!("display:flex; flex-direction:column; padding:{}px; background:#FFFFFF;", module * 3),
            for y in 0..w {
                div {
                    style: "display:flex; flex-direction:row;",
                    for (wpx , dark) in row_runs(&colors, w, y, module) {
                        div {
                            style: format!(
                                "width:{}px; height:{}px; background:{};",
                                wpx, module, if dark { "#000000" } else { "#FFFFFF" }
                            ),
                        }
                    }
                }
            }
        }
    }
}

/// Merge run-lengths of same-colour modules in row `y` → `(width_px, is_dark)`.
fn row_runs(colors: &[qrcode::Color], w: usize, y: usize, module: i32) -> Vec<(i32, bool)> {
    let mut runs = Vec::new();
    let mut x = 0;
    while x < w {
        let dark = colors[y * w + x] == qrcode::Color::Dark;
        let mut k = 1;
        while x + k < w && (colors[y * w + x + k] == qrcode::Color::Dark) == dark {
            k += 1;
        }
        runs.push((k as i32 * module, dark));
        x += k;
    }
    runs
}

#[component]
fn Conversation(messages: Vec<UiMsg>, contacts: Vec<UiContact>, stick_key: String) -> Element {
    // Tag each message with the date label to show above it (once per day, when
    // the local day changes from the previous message).
    let mut rows: Vec<(Option<String>, UiMsg)> = Vec::with_capacity(messages.len());
    let mut prev_day: Option<i64> = None;
    for m in &messages {
        let day = local_day(m.ts);
        let divider = (prev_day != Some(day)).then(|| date_label(day));
        prev_day = Some(day);
        rows.push((divider, m.clone()));
    }

    rsx! {
        div {
            style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1; min-height:0; padding:24px; gap:14px;",
            // Stick this scroll region to the newest message: opening a thread
            // jumps to the end, and new arrivals keep it pinned while at the
            // bottom. Keyed by thread id so switching conversations re-jumps.
            "data-stick-key": "{stick_key}",
            if rows.is_empty() {
                div { style: "color:{MUTED}; font-size:28px; padding:24px;", "No messages yet." }
            }
            for (divider , m) in rows {
                {
                    // Resolve the sender ACI → contact name (fallback: short ACI).
                    let label = sender_label(&m.sender, &contacts);
                    let time = fmt_time(m.ts);
                    let (check, check_col) = check_marks(m.status);
                    rsx! {
                        if let Some(d) = divider {
                            div {
                                style: "display:flex; flex-direction:row; justify-content:center; padding:6px;",
                                div {
                                    style: "padding:8px 22px; border-radius:16px; background:{BAR}; color:{MUTED}; font-size:22px;",
                                    "{d}"
                                }
                            }
                        }
                        if let Some((icon, color)) = m.call.map(call_log_style) {
                            // Call-history entry: a centered pill, not a bubble.
                            div {
                                key: "{m.id}",
                                style: "display:flex; flex-direction:row; justify-content:center; padding:6px;",
                                div {
                                    style: format!("display:flex; flex-direction:row; align-items:center; gap:14px; padding:12px 26px; border-radius:18px; background:{};", BAR),
                                    div { style: format!("color:{}; font-size:30px;", color), "{icon}" }
                                    div { style: format!("color:{}; font-size:26px;", color), "{m.text}" }
                                    div { style: format!("color:{}; font-size:20px;", META), "{time}" }
                                }
                            }
                        } else {
                        div {
                            key: "{m.id}",
                            // Row: align outgoing right, incoming left.
                            style: format!(
                                "display:flex; flex-direction:row; justify-content:{};",
                                if m.outgoing { "flex-end" } else { "flex-start" }
                            ),
                            div {
                                style: format!(
                                    "display:flex; flex-direction:column; gap:6px; max-width:78%; padding:18px; border-radius:18px; background:{};",
                                    if m.outgoing { OUT_BUBBLE } else { IN_BUBBLE }
                                ),
                                if !m.outgoing {
                                    div { style: "color:{SENDER}; font-size:22px; font-weight:600;", "{label}" }
                                }
                                // Image attachments (decrypted by the engine), each
                                // sized to its aspect-fit box.
                                for img in m.images.iter() {
                                    img {
                                        src: "{img.uri}",
                                        style: "width:{img.w}px; height:{img.h}px; border-radius:12px;",
                                    }
                                }
                                if !m.text.is_empty() {
                                    div { style: "color:{TEXT}; font-size:30px; white-space:normal;", "{m.text}" }
                                }
                                // Meta row: local time + (outgoing) delivery checks.
                                div {
                                    style: "display:flex; flex-direction:row; align-items:center; justify-content:flex-end; gap:8px;",
                                    div { style: "color:{META}; font-size:20px;", "{time}" }
                                    if m.outgoing {
                                        div { style: "color:{check_col}; font-size:20px;", "{check}" }
                                    }
                                }
                                // Reaction pill (emoji + count for group repeats),
                                // hugging the start edge via a flex-start wrapper.
                                if !m.reactions.is_empty() {
                                    div {
                                        style: "display:flex; flex-direction:row; justify-content:flex-start;",
                                        div {
                                            style: "padding:4px 14px; border-radius:18px; background:{BAR}; color:{TEXT}; font-size:26px;",
                                            "{m.reactions}"
                                        }
                                    }
                                }
                            }
                        }
                        }
                    }
                }
            }
        }
    }
}

/// Icon + color for a call-log entry (↗ outgoing / ↙ incoming; green answered,
/// red missed, orange declined/busy, muted unanswered-outgoing).
fn call_log_style(c: chat::CallLog) -> (&'static str, &'static str) {
    match c {
        chat::CallLog::OutAnswered => ("↗ 📞", "#3CB043"),
        chat::CallLog::OutMissed => ("↗ 📞", MUTED),
        chat::CallLog::OutBusy => ("↗ 📵", "#E0922A"),
        chat::CallLog::InAnswered => ("↙ 📞", "#3CB043"),
        chat::CallLog::InMissed => ("↙ 📵", "#E04A4A"),
        chat::CallLog::InDeclined => ("↙ 📵", "#E0922A"),
    }
}

/// `<ACI:uuid>` → the matching contact's name, else the short ACI.
fn sender_label(sender: &str, contacts: &[UiContact]) -> String {
    let inner = sender.trim_start_matches('<').trim_end_matches('>');
    let uuid = inner.split_once(':').map(|(_, u)| u).unwrap_or(inner);
    contacts
        .iter()
        .find(|c| c.id == uuid)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| short_sender(sender))
}

/// First letter of a name, uppercased — the avatar placeholder when a contact
/// has no image.
fn initial(name: &str) -> String {
    name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()
}

/// A circular/rounded avatar slot: the decrypted image (`img { src: data-uri }`,
/// drawn by the renderer's image support) or an initial-letter placeholder.
#[component]
fn Avatar(uri: Option<String>, letter: String) -> Element {
    rsx! {
        if let Some(uri) = uri {
            img { src: "{uri}", style: "width:84px; height:84px; flex-shrink:0; border-radius:14px;" }
        } else {
            div {
                style: "display:flex; justify-content:center; align-items:center; width:84px; height:84px; flex-shrink:0; border-radius:14px; background:{OUT_BUBBLE};",
                div { style: "color:{TEXT}; font-size:38px; font-weight:700;", "{letter}" }
            }
        }
    }
}

/// A small accent badge with the unread count; renders nothing when `count` is 0.
#[component]
fn UnreadBadge(count: u32) -> Element {
    rsx! {
        if count > 0 {
            div {
                style: "display:flex; justify-content:center; align-items:center; min-width:44px; height:44px; padding:0 12px; border-radius:22px; background:{ACCENT}; flex-shrink:0;",
                div { style: "color:{TEXT}; font-size:26px; font-weight:700;", "{count}" }
            }
        }
    }
}

/// The people list: groups first (avatar + member preview), then individual
/// contacts (avatar + name + phone). Both share one scroll container so the
/// "Contacts" tab is the single place to see everyone.
#[component]
fn Contacts(
    contacts: Vec<UiContact>,
    groups: Vec<UiGroup>,
    self_id: String,
    unread: std::collections::HashMap<String, u32>,
    current: Signal<Option<Thread>>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1; min-height:0; padding:16px; gap:10px;",
            if contacts.is_empty() && groups.is_empty() {
                div { style: "color:{MUTED}; font-size:28px; padding:24px;", "No contacts yet." }
            }
            for g in groups {
                {
                    let mut current = current;
                    let preview = g.members.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                    let t = Thread { id: g.id.clone(), title: g.title.clone(), is_group: true };
                    let n = unread.get(&g.id).copied().unwrap_or(0);
                    let oid = g.id.clone();
                    rsx! {
                        button {
                            key: "{g.id}",
                            style: "display:flex; flex-direction:row; align-items:center; gap:20px; padding:14px; border-radius:16px; background:{IN_BUBBLE};",
                            onclick: move |_| { open_thread(&oid); current.set(Some(t.clone())); },
                            Avatar { uri: g.avatar_uri, letter: initial(&g.title) }
                            div {
                                style: "display:flex; flex-direction:column; gap:4px; flex-grow:1; min-width:0;",
                                div { style: "color:{TEXT}; font-size:32px;", "{g.title}" }
                                div { style: "color:{MUTED}; font-size:24px;", "{g.members.len()} members · {preview}" }
                            }
                            UnreadBadge { count: n }
                        }
                    }
                }
            }
            for c in contacts {
                {
                    let mut current = current;
                    // The self contact (id == own ACI, usually no profile name) reads
                    // "Note to Self".
                    let is_self = !self_id.is_empty() && c.id == self_id;
                    let name = if is_self {
                        "Note to Self".to_string()
                    } else {
                        c.name.clone()
                    };
                    let t = Thread { id: c.id.clone(), title: name.clone(), is_group: false };
                    let n = unread.get(&c.id).copied().unwrap_or(0);
                    let oid = c.id.clone();
                    rsx! {
                        button {
                            key: "{c.id}",
                            style: "display:flex; flex-direction:row; align-items:center; gap:20px; padding:14px; border-radius:16px; background:{IN_BUBBLE};",
                            onclick: move |_| { open_thread(&oid); current.set(Some(t.clone())); },
                            Avatar { uri: c.avatar_uri, letter: initial(&name) }
                            div {
                                style: "display:flex; flex-direction:column; gap:4px; flex-grow:1; min-width:0;",
                                div { style: "color:{TEXT}; font-size:32px;", "{name}" }
                                if let Some(phone) = c.phone {
                                    div { style: "color:{MUTED}; font-size:24px;", "{phone}" }
                                }
                            }
                            UnreadBadge { count: n }
                        }
                    }
                }
            }
        }
    }
}

/// Message composer: a `data-input` field (the renderer draws value + caret) +
/// a Send button. Enter or Send calls `chat::send`; the engine echoes it back as
/// an outgoing message event, so we don't add it locally.
#[component]
fn Composer(thread: String) -> Element {
    let mut value = use_signal(String::new);
    let mut caret = use_signal(|| 0usize);
    let mut focused = use_signal(|| false);

    // Clones for the two send paths (Enter key + Send button); both handlers are
    // `move` so each needs its own copy of the thread id.
    let thread_key = thread.clone();
    let thread_btn = thread;

    let v = value();
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; gap:16px; padding:20px; background:{BAR};",
            div {
                "data-input": "1",
                "value": "{v}",
                "caret": "{caret}",
                "focused": if focused() { "1" } else { "0" },
                style: format!(
                    "display:flex; flex-grow:1; height:96px; border-radius:20px; font-size:34px; color:{}; background:{};",
                    TEXT, FIELD
                ),
                onmousedown: move |e| {
                    caret.set(caret_at(&value(), e.element_coordinates().x as f32));
                    if !focused() {
                        focused.set(true);
                        let t = value();
                        let n = t.chars().count() as u32;
                        editor_attach("text", "Message", &t, n, n);
                    }
                },
                // An onmousemove listener is what marks this element draggable in
                // dioxus-canvas — WITHOUT it, taps set focus but `onmousedown`
                // (which attaches the IME) is never dispatched, so the keyboard
                // never shows. (It also lets a drag reposition the caret.)
                onmousemove: move |e| {
                    caret.set(caret_at(&value(), e.element_coordinates().x as f32));
                },
                onfocusout: move |_| {
                    if focused() {
                        focused.set(false);
                        editor_detach();
                    }
                },
                onkeydown: move |e| {
                    let k = e.key().to_string();
                    let chars: Vec<char> = value().chars().collect();
                    let c = caret().min(chars.len());
                    match k.as_str() {
                        "Enter" => submit(thread_key.clone(), value, caret),
                        "Escape" => { focused.set(false); editor_detach(); }
                        "Backspace" => {
                            if c > 0 {
                                let mut s: String = chars[..c - 1].iter().collect();
                                s.extend(&chars[c..]);
                                value.set(s);
                                caret.set(c - 1);
                            }
                        }
                        "ArrowLeft" => caret.set(c.saturating_sub(1)),
                        "ArrowRight" => caret.set((c + 1).min(chars.len())),
                        _ if k.chars().count() == 1 => {
                            let mut s: String = chars[..c].iter().collect();
                            s.push_str(&k);
                            s.extend(&chars[c..]);
                            value.set(s);
                            caret.set(c + 1);
                        }
                        _ => {}
                    }
                },
            }
            button {
                style: format!(
                    "display:flex; justify-content:center; align-items:center; width:140px; height:96px; border-radius:20px; background:{};",
                    ACCENT
                ),
                onclick: move |_| submit(thread_btn.clone(), value, caret),
                div { style: "color:{TEXT}; font-size:32px; font-weight:600;", "Send" }
            }
        }
    }
}
