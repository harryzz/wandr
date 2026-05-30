//! signal-ui — a dioxus-canvas guest that drives the Signal engine purely through
//! the `wart:signal/chat` contract (task-67 Phase 2 item 2). It renders the
//! conversation (history + live events from `poll-events`), sends typed messages
//! via `send`, and surfaces link/connection state. Composed with signal-engine
//! through `wac plug` (item 3); standalone it imports `wart:signal/chat`, which
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

use dioxus::prelude::*;

// One generate! for everything this guest talks to: the skiko-gfx world
// (canvas/paragraph/ime imports + renderer/frame-pacing exports) AND the engine's
// `wart:signal/chat` import (see wit/). A second generate! would conflict on
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

use wart::signal::chat;

// Renderer/sink/IME wiring from dioxus-canvas, over the bindings generated above.
// The pre_frame hook is where we pump the engine each tick.
//
// NOTE: making the composer ride above the soft keyboard is deferred to task 68
// (host-driven keyboard inset) — the app must NOT hard-code the overlay height
// (it's 1200px today but resizable via request-overlay-height). For now the
// composer is a plain bottom bar; while typing it sits behind the keyboard.
dioxus_canvas::wire!(app, pre_frame: |r| {
    r.set_scale(2.0); // hi-dpi panel — author px are small; 2× for readability
    r.set_min_frame_delay(120); // ~8 polls/sec so the engine keeps making progress
    if pump() {
        r.mark_dirty();
    }
});

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

// ── the model the components read, updated by `pump` from the engine ──────────
#[derive(Clone, PartialEq, Default)]
struct UiMsg {
    id: u64,
    sender: String,
    text: String,
    outgoing: bool,
}

#[derive(Clone, PartialEq, Default)]
struct UiContact {
    id: String,
    name: String,
    phone: Option<String>,
    /// `data:…;base64,…` URI for `img { src }`, encoded once when contacts load.
    avatar_uri: Option<String>,
}

#[derive(Default)]
struct Model {
    state: String,
    link_url: Option<String>,
    messages: Vec<UiMsg>,
    contacts: Vec<UiContact>,
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
            // Backfill persisted history (messages from before this UI started;
            // live ones arrive as events below).
            let hist = chat::history();
            let contacts = load_contacts();
            MODEL.with(|m| {
                let mut m = m.borrow_mut();
                m.contacts = contacts;
                for msg in hist {
                    m.messages.push(UiMsg {
                        id: msg.id,
                        sender: msg.sender,
                        text: msg.text,
                        outgoing: msg.outgoing,
                    });
                }
            });
        }
    });

    let mut changed = false;
    let mut refresh = false;
    let events = chat::poll_events();
    if !events.is_empty() {
        MODEL.with(|m| {
            let mut m = m.borrow_mut();
            for e in events {
                match e {
                    chat::Event::Message(msg) => {
                        // Dedup by id (a live message is also in history()).
                        if !m.messages.iter().any(|x| x.id == msg.id) {
                            m.messages.push(UiMsg {
                                id: msg.id,
                                sender: msg.sender,
                                text: msg.text,
                                outgoing: msg.outgoing,
                            });
                        }
                    }
                    chat::Event::LinkUrl(url) => m.link_url = Some(url),
                    chat::Event::Linked(_) | chat::Event::Connected => {
                        m.link_url = None;
                    }
                    chat::Event::Disconnected => {}
                    // Re-fetched below (load_contacts needs no &mut m borrow).
                    chat::Event::ContactsUpdated(_) => refresh = true,
                }
            }
        });
        changed = true;
    }

    if refresh {
        let contacts = load_contacts();
        MODEL.with(|m| m.borrow_mut().contacts = contacts);
    }

    let state = chat::state();
    MODEL.with(|m| {
        let mut m = m.borrow_mut();
        if m.state != state {
            m.state = state;
            changed = true;
        }
    });
    changed
}

fn snapshot() -> (String, Option<String>, Vec<UiMsg>, Vec<UiContact>) {
    MODEL.with(|m| {
        let m = m.borrow();
        (m.state.clone(), m.link_url.clone(), m.messages.clone(), m.contacts.clone())
    })
}

/// Send the composer's current text through the engine, then clear it. Signals
/// are `Copy`, passed by value. The engine echoes the message back as an outgoing
/// event, so we don't append it locally.
fn submit(mut value: Signal<String>, mut caret: Signal<usize>) {
    let t = value().trim().to_string();
    if !t.is_empty() {
        let _ = chat::send(&t);
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

// ── components ────────────────────────────────────────────────────────────────
fn app() -> Element {
    // Stay armed: re-schedule this scope every render so engine updates (pushed
    // into MODEL by `pump` + flagged via `mark_dirty`) re-run this body and reach
    // the tree.
    dioxus::core::needs_update();

    // 0 = chat, 1 = contacts.
    let mut tab = use_signal(|| 0u8);

    let (state, link_url, messages, contacts) = snapshot();
    let n = contacts.len();
    let chat_bg = if tab() == 0 { ACCENT } else { BAR };
    let people_bg = if tab() == 1 { ACCENT } else { BAR };
    rsx! {
        div {
            style: "display:flex; flex-direction:column; height:100%; background:{BG};",
            // Title bar + tabs.
            div {
                style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between; padding:28px; background:{BAR};",
                div { style: "color:{TEXT}; font-size:44px; font-weight:700;", "Signal" }
                div { style: "color:{MUTED}; font-size:26px;", "{state}" }
            }
            div {
                style: "display:flex; flex-direction:row; gap:12px; padding:14px 24px; background:{BAR};",
                button {
                    style: format!("display:flex; justify-content:center; flex-grow:1; padding:14px; border-radius:14px; background:{};", chat_bg),
                    onclick: move |_| tab.set(0),
                    div { style: "color:{TEXT}; font-size:28px;", "Chat" }
                }
                button {
                    style: format!("display:flex; justify-content:center; flex-grow:1; padding:14px; border-radius:14px; background:{};", people_bg),
                    onclick: move |_| tab.set(1),
                    div { style: "color:{TEXT}; font-size:28px;", "Contacts ({n})" }
                }
            }
            if tab() == 0 {
                if let Some(url) = link_url {
                    LinkPanel { url }
                }
                Conversation { messages, contacts: contacts.clone() }
                Composer {}
            } else {
                Contacts { contacts }
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
fn Conversation(messages: Vec<UiMsg>, contacts: Vec<UiContact>) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1; min-height:0; padding:24px; gap:14px;",
            if messages.is_empty() {
                div { style: "color:{MUTED}; font-size:28px; padding:24px;", "No messages yet." }
            }
            for m in messages {
                {
                    // Resolve the sender ACI → contact name (fallback: short ACI).
                    let label = sender_label(&m.sender, &contacts);
                    rsx! {
                        div {
                            key: "{m.id}",
                            // Row: align outgoing right, incoming left.
                            style: format!(
                                "display:flex; flex-direction:row; justify-content:{};",
                                if m.outgoing { "flex-end" } else { "flex-start" }
                            ),
                            div {
                                style: format!(
                                    "display:flex; flex-direction:column; gap:6px; padding:18px; border-radius:18px; background:{};",
                                    if m.outgoing { OUT_BUBBLE } else { IN_BUBBLE }
                                ),
                                if !m.outgoing {
                                    div { style: "color:{SENDER}; font-size:22px; font-weight:600;", "{label}" }
                                }
                                div { style: "color:{TEXT}; font-size:30px;", "{m.text}" }
                            }
                        }
                    }
                }
            }
        }
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

/// The contact list: each row is an avatar (`img { src: data-uri }`, drawn by the
/// renderer's image support) or an initial placeholder, plus name + phone.
#[component]
fn Contacts(contacts: Vec<UiContact>) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1; min-height:0; padding:16px; gap:10px;",
            if contacts.is_empty() {
                div { style: "color:{MUTED}; font-size:28px; padding:24px;", "No contacts yet." }
            }
            for c in contacts {
                div {
                    key: "{c.id}",
                    style: "display:flex; flex-direction:row; align-items:center; gap:20px; padding:14px; border-radius:16px; background:{IN_BUBBLE};",
                    if let Some(uri) = c.avatar_uri {
                        img { src: "{uri}", style: "width:84px; height:84px; border-radius:14px;" }
                    } else {
                        div {
                            style: "display:flex; justify-content:center; align-items:center; width:84px; height:84px; border-radius:14px; background:{OUT_BUBBLE};",
                            div { style: "color:{TEXT}; font-size:38px; font-weight:700;", "{initial(&c.name)}" }
                        }
                    }
                    div {
                        style: "display:flex; flex-direction:column; gap:4px;",
                        div { style: "color:{TEXT}; font-size:32px;", "{c.name}" }
                        if let Some(phone) = c.phone {
                            div { style: "color:{MUTED}; font-size:24px;", "{phone}" }
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
fn Composer() -> Element {
    let mut value = use_signal(String::new);
    let mut caret = use_signal(|| 0usize);
    let mut focused = use_signal(|| false);

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
                        "Enter" => submit(value, caret),
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
                onclick: move |_| submit(value, caret),
                div { style: "color:{TEXT}; font-size:32px; font-weight:600;", "Send" }
            }
        }
    }
}
