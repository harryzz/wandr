//! wandr.connectivity.test — a dioxus-canvas guest that renders live connectivity
//! from the wandr event bus (task 90). It subscribes to the `net.status` topic via
//! `package.toml [events]` (host-config, the wasi:messaging delivery model) and
//! exports `wandr:events/incoming-handler`; the host calls `handle(msg)` with the
//! retained value on subscribe + every change. No polling — the host forces a frame
//! when it delivers an event, so `app()` (armed via `needs_update`) re-reads the
//! decoded snapshot and repaints.
//!
//! Payload schema for `net.status` (decoded from `msg.data`): the wire string
//! `online wifi <ssid> <ip>` or `offline` (the same form the arbiter's net module
//! uses), published by the wandr-net daemon.

use std::cell::RefCell;

use dioxus::prelude::*;

// One combined generate! for everything this guest talks to: the trimmed
// `my:skiko-gfx` world (canvas/paragraph/ime + renderer/frame-pacing) AND the
// `wandr:events/incoming-handler` EXPORT (see wit/). Same flags as dioxus-canvas's
// `skiko_world!` so `wire!` finds the bindings + export macro.
dioxus_canvas::__wit_bindgen::generate!({
    world: "connectivity-test-app",
    path: "wit",
    generate_all,
    pub_export_macro: true,
    export_macro_name: "__dioxus_canvas_export",
    runtime_path: "::dioxus_canvas::__wit_bindgen::rt",
});

use exports::wandr::events::incoming_handler::{Guest as IncomingHandler, Message};

#[derive(Clone, Default, PartialEq)]
struct NetStatus {
    online: bool,
    transport: String,
    ssid: Option<String>,
    ip: Option<String>,
    /// Number of events received (proves the push path is live).
    events: u32,
    /// Whether we've received any event yet (vs. "waiting…").
    seen: bool,
}

thread_local! {
    static STATUS: RefCell<NetStatus> = RefCell::new(NetStatus::default());
    /// Set by `handle` when an event lands; consumed by `pre_frame` to
    /// `mark_dirty` the renderer (dioxus-canvas only re-runs the component when the
    /// renderer is dirty — `handle` runs outside the render path, so it can't call
    /// `r` directly). Mirrors the Signal guest's `pump`→`mark_dirty`.
    static DIRTY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Parse the `net.status` wire form (`online wifi <ssid> <ip>` / `offline`).
fn parse_status(wire: &str, prev_events: u32) -> NetStatus {
    let mut it = wire.split_whitespace();
    match it.next() {
        Some("online") => {
            let transport = it.next().unwrap_or("wifi").to_string();
            let ssid = it.next().filter(|s| *s != "-").map(str::to_string);
            let ip = it.next().filter(|s| *s != "-").map(str::to_string);
            NetStatus { online: true, transport, ssid, ip, events: prev_events + 1, seen: true }
        }
        _ => NetStatus { online: false, transport: "none".into(), events: prev_events + 1, seen: true, ..Default::default() },
    }
}

// The event-bus receive side. `wire!` below defines `__DioxusCanvasGuest` and
// `export!`s the whole world (which also exports incoming-handler), so we impl the
// handler on the SAME guest type here (trait impls are crate-wide, order-independent
// — `export!` finds it). The host forces a frame on delivery, so just storing the
// decoded snapshot is enough for `app()` to pick it up.
impl IncomingHandler for __DioxusCanvasGuest {
    fn handle(msg: Message) {
        if msg.topic != "net.status" {
            return;
        }
        let wire = String::from_utf8_lossy(&msg.data).to_string();
        STATUS.with(|s| {
            let prev = s.borrow().events;
            *s.borrow_mut() = parse_status(&wire, prev);
        });
        DIRTY.with(|d| d.set(true));
    }
}

dioxus_canvas::wire!(app, pre_frame: |r| {
    r.set_scale(1.5);
    // An event landed since the last frame → re-run + repaint the component.
    if DIRTY.with(|d| d.replace(false)) {
        r.mark_dirty();
    }
    // Push-driven: the host forces a frame when an event lands, so we can idle
    // slowly between events (the gauge is static otherwise).
    r.set_min_frame_delay(2000);
});

// ── UI ───────────────────────────────────────────────────────────────────────

const BG: &str = "#12121A";
const CARD: &str = "#1F1F33";
const TEXT: &str = "#FFFFFF";
const MUTED: &str = "#C7C7D9";
const ONLINE: &str = "#34A853";
const OFFLINE: &str = "#EA4335";

fn app() -> Element {
    // Stay armed so the snapshot stored by `handle` (on a host-forced frame) reaches
    // the tree.
    dioxus::core::needs_update();
    let s = STATUS.with(|s| s.borrow().clone());

    let (dot, status_word) = if !s.seen {
        (MUTED, "waiting for event…".to_string())
    } else if s.online {
        (ONLINE, "Online".to_string())
    } else {
        (OFFLINE, "Offline".to_string())
    };
    let ssid = s.ssid.clone().unwrap_or_else(|| "—".to_string());
    let ip = s.ip.clone().unwrap_or_else(|| "—".to_string());
    let transport = if s.transport.is_empty() { "—".to_string() } else { s.transport.clone() };

    rsx! {
        div {
            style: "display:flex; flex-direction:column; padding:48px; gap:32px; background:{BG}; height:100%;",

            div {
                style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between;",
                div { style: "color:{TEXT}; font-size:56px; font-weight:700;", "Net Monitor" }
                div { style: "color:{MUTED}; font-size:28px;", "{s.events} events" }
            }

            // Big status pill.
            div {
                style: "display:flex; flex-direction:row; align-items:center; gap:24px; background:{CARD}; padding:36px; border-radius:24px;",
                div { style: format!("width:64px; height:64px; border-radius:50%; background:{};", dot) }
                div { style: "color:{TEXT}; font-size:48px; font-weight:700;", "{status_word}" }
            }

            // Detail rows.
            Row { k: "Transport".to_string(), v: transport }
            Row { k: "SSID".to_string(), v: ssid }
            Row { k: "IP address".to_string(), v: ip }

            div {
                style: "color:{MUTED}; font-size:24px; margin-top:8px;",
                "Subscribed to net.status via wandr:events — pushed by the arbiter, no polling."
            }
        }
    }
}

#[component]
fn Row(k: String, v: String) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between; background:{CARD}; padding:28px; border-radius:18px;",
            div { style: "color:{MUTED}; font-size:32px;", "{k}" }
            div { style: "color:{TEXT}; font-size:32px; font-weight:600;", "{v}" }
        }
    }
}
