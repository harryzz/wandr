//! wandr.settings.wifi — the Wi-Fi settings / picker chrome (task 90 M4).
//!
//! The FIRST privileged guest: `kind=system` + `wifi-control=true` is what the
//! host's `LoadedApp::wifi_privileged` gate requires before it links the
//! `wandr:connectivity/wifi` interface onto this component. So this app exercises
//! the whole M2/M3 stack end-to-end from a real guest: scan (IWificond), join
//! (ISupplicant, passphrase via the IME), radio on/off (IWifi), and the M3
//! WifiConfigManager (saved networks + auto-connect), all host→arbiter→wandr-net.
//!
//! Polling: `pre_frame` refreshes `is-enabled` + `list-saved` every cycle (cheap)
//! and `scan` on a slow cadence (a scan triggers a real IWificond sweep and blocks
//! ~1 s, so we don't do it every frame) or when the Refresh button sets the force
//! flag. Results land in a thread-local `MODEL` + `mark_dirty` on change (the
//! Signal-guest bridge); `app()` reads `MODEL` each render. Transient UI state
//! (which row's passphrase field is open) lives in dioxus signals.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use dioxus::prelude::*;

// One combined generate! for the trimmed `my:skiko-gfx` world (canvas/paragraph/ime
// imports + renderer/frame-pacing exports) AND the privileged
// `wandr:connectivity/wifi` import (see wit/). Same `export_macro_name` +
// `runtime_path` as dioxus-canvas's `skiko_world!`, so `wire!` finds the bindings.
dioxus_canvas::__wit_bindgen::generate!({
    world: "wifi-settings-ui-app",
    path: "wit",
    generate_all,
    pub_export_macro: true,
    export_macro_name: "__dioxus_canvas_export",
    runtime_path: "::dioxus_canvas::__wit_bindgen::rt",
});

dioxus_canvas::wasi_canvas_bindings!();

use wandr::connectivity::wifi;
use wandr::connectivity::wifi::{IpConfig, SecurityKind, WifiConfig};

// ── model (host snapshot, decoupled from the generated types) ────────────────

#[derive(Clone, PartialEq)]
struct Ap {
    ssid: String,
    /// Needs a passphrase to join (wpa-psk / sae / wpa-eap). Open/owe = direct.
    secured: bool,
    rssi: i32,
    connected: bool,
    /// `Some(id)` if this ssid is in the M3 saved store (tap → connect by id, no
    /// passphrase prompt).
    saved_id: Option<u32>,
}

#[derive(Clone, PartialEq, Default)]
struct Saved {
    id: u32,
    ssid: String,
    auto: bool,
}

#[derive(Clone, Default)]
struct Model {
    enabled: bool,
    aps: Vec<Ap>,
    saved: Vec<Saved>,
}

thread_local! {
    static MODEL: RefCell<Model> = RefCell::new(Model::default());
    /// Wall-clock of the last frame — its gap to *now* tells us whether the user is
    /// actively interacting (a fast frame stream from scroll/typing) so we keep
    /// those frames pure-render and never block them on a host call.
    static LAST_FRAME: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Wall-clock of the last host poll — the cadence is TIME-based, not frame-based,
    /// so a high scroll frame rate can't hammer it.
    static LAST_POLL: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Set by the Scan button → forces a poll once interaction settles.
    static FORCE_SCAN: Cell<bool> = const { Cell::new(true) };
}

fn is_secured(s: SecurityKind) -> bool {
    matches!(s, SecurityKind::WpaPsk | SecurityKind::Sae | SecurityKind::WpaEap)
}

/// Pull a fresh snapshot into `MODEL`. `scan` is now non-blocking host-side (the
/// daemon serves cached results + refreshes in the background), so all three reads
/// run together each poll cycle. Returns whether anything changed, so the caller
/// only `mark_dirty`s on a real delta.
fn poll() -> bool {
    let enabled = wifi::is_enabled();
    let saved_raw = wifi::list_saved();
    let saved: Vec<Saved> = saved_raw
        .iter()
        .map(|s| Saved { id: s.id, ssid: s.ssid.clone(), auto: s.auto_connect })
        .collect();

    // Dedup APs by SSID (one row even on 2.4+5 GHz), keep the connected/strongest,
    // tag with the saved id, sort connected-first then by signal.
    let mut aps: Vec<Ap> = Vec::new();
    for r in wifi::scan().unwrap_or_default() {
        if r.ssid.is_empty() {
            continue; // hidden / unnamed — no actionable row
        }
        let saved_id = saved.iter().find(|s| s.ssid == r.ssid).map(|s| s.id);
        let ap = Ap {
            ssid: r.ssid,
            secured: is_secured(r.security),
            rssi: r.rssi,
            connected: r.connected,
            saved_id,
        };
        match aps.iter_mut().find(|e| e.ssid == ap.ssid) {
            Some(e) => {
                if ap.connected || (!e.connected && ap.rssi > e.rssi) {
                    *e = ap;
                }
            }
            None => aps.push(ap),
        }
    }
    aps.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.rssi.cmp(&a.rssi)));

    MODEL.with(|m| {
        let mut m = m.borrow_mut();
        let changed = m.enabled != enabled || m.saved != saved || m.aps != aps;
        m.enabled = enabled;
        m.saved = saved;
        m.aps = aps;
        changed
    })
}

/// How often to poll the host (is-enabled / list-saved / scan), in wall-clock time.
/// All three are non-blocking now (the daemon caches scan), so one cadence covers
/// them. Frame-rate-independent (a scroll stream can't shorten it).
const POLL_INTERVAL: Duration = Duration::from_millis(1500);
/// Frames closer than this are an active input burst (scroll/typing) — keep them
/// pure-render and defer host polls until the user pauses.
const INTERACT_GAP: Duration = Duration::from_millis(400);

dioxus_canvas::wire_wasi_canvas!(app, pre_frame: |r| {
    r.set_scale(1.5); // hi-dpi panel — author px are small; 1.5× is the readable size.
    let now = Instant::now();
    let gap = LAST_FRAME.with(|c| c.replace(Some(now))).map(|t| now.saturating_duration_since(t));
    let interacting = gap.map_or(false, |g| g < INTERACT_GAP);

    // While scrolling/typing, do NOTHING but render — polls resume the moment the
    // user pauses (the next idle frame, ~1 s apart, is not "interacting").
    if !interacting {
        let force = FORCE_SCAN.with(|c| c.replace(false));
        let due = LAST_POLL.with(|c| c.get())
            .map_or(true, |t| now.saturating_duration_since(t) >= POLL_INTERVAL);
        if force || due {
            LAST_POLL.with(|c| c.set(Some(now)));
            if poll() {
                r.mark_dirty();
            }
        }
    }
    // Idle cadence ~1 s — frequent enough to keep the poll timely, cheap enough; input
    // drives its own immediate (pure-render) frames.
    r.set_min_frame_delay(1000);
});

// ── connect helpers ──────────────────────────────────────────────────────────

fn dhcp_config(ssid: &str, security: SecurityKind, passphrase: Option<String>) -> WifiConfig {
    WifiConfig {
        ssid: ssid.to_string(),
        security,
        passphrase,
        auto_connect: true,
        hidden: false,
        ip_config: IpConfig::Dhcp,
    }
}

/// Join an open network directly (no passphrase).
fn join_open(ssid: &str) {
    let _ = wifi::connect_new(&dhcp_config(ssid, SecurityKind::Open, None));
    FORCE_SCAN.with(|c| c.set(true)); // refresh the connected marker
}

/// Join a secured network with a typed passphrase (WPA2/WPA3 personal).
fn join_secured(ssid: &str, passphrase: &str) {
    let _ = wifi::connect_new(&dhcp_config(ssid, SecurityKind::WpaPsk, Some(passphrase.to_string())));
    FORCE_SCAN.with(|c| c.set(true));
}

// ── UI ───────────────────────────────────────────────────────────────────────

const BG: &str = "#12121A";
const CARD: &str = "#1F1F33";
const SUBTLE: &str = "#2A2A44";
const TEXT: &str = "#FFFFFF";
const MUTED: &str = "#C7C7D9";
const ACCENT: &str = "#34A853"; // on / connected
const FIELD: &str = "#0D0D14";
const DANGER: &str = "#EA4335";

/// rssi (dBm) → 0..4 signal bars.
fn bars(rssi: i32) -> u8 {
    match rssi {
        r if r >= -55 => 4,
        r if r >= -67 => 3,
        r if r >= -78 => 2,
        r if r >= -88 => 1,
        _ => 0,
    }
}

/// Map an x within the field to a caret index (chars), like the Signal composer.
fn caret_at(value: &str, _x: f32) -> usize {
    value.chars().count() // simplest: caret at end (passphrases are short, typed sequentially)
}

fn app() -> Element {
    // Stay armed so the host snapshot pumped into MODEL reaches the tree.
    dioxus::core::needs_update();
    let model = MODEL.with(|m| m.borrow().clone());

    // Transient UI state: which secured-unsaved SSID has its passphrase field open.
    let expanded = use_signal(|| None::<String>);
    let pass = use_signal(String::new);
    let caret = use_signal(|| 0usize);
    let focused = use_signal(|| false);

    rsx! {
        div {
            style: "display:flex; flex-direction:column; padding:40px; gap:26px; background:{BG}; height:100%;",

            // ── Header: title + radio toggle ─────────────────────────────────
            div {
                style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between; flex-shrink:0;",
                div { style: "color:{TEXT}; font-size:56px; font-weight:700;", "Wi-Fi" }
                Toggle { on: model.enabled }
            }

            if !model.enabled {
                div { style: "color:{MUTED}; font-size:34px; padding:40px 0;", "Wi-Fi is off." }
            } else {
                // ── Scrollable body: networks + saved ────────────────────────
                div {
                    style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1; gap:14px;",
                    div {
                        style: "display:flex; flex-direction:column; flex-shrink:0; gap:14px;",

                        SectionHeader { title: "Networks".to_string() }

                        for ap in model.aps.iter().cloned() {
                            {
                                let ssid = ap.ssid.clone();
                                let is_open_field = expanded().as_deref() == Some(ssid.as_str());
                                rsx! {
                                    NetRow {
                                        ap: ap.clone(),
                                        expanded,
                                        pass,
                                        caret,
                                        focused,
                                    }
                                    if is_open_field {
                                        PassField { ssid: ssid.clone(), expanded, pass, caret, focused }
                                    }
                                }
                            }
                        }

                        if !model.saved.is_empty() {
                            SectionHeader { title: "Saved networks".to_string() }
                            for sv in model.saved.iter().cloned() {
                                SavedRow { sv }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Toggle(on: bool) -> Element {
    let bg = if on { ACCENT } else { SUBTLE };
    let label = if on { "On" } else { "Off" };
    rsx! {
        button {
            style: format!(
                "display:flex; justify-content:center; align-items:center; width:150px; height:72px; border-radius:36px; background:{};",
                bg
            ),
            onclick: move |_| {
                wifi::set_enabled(!on);
                FORCE_SCAN.with(|c| c.set(true));
            },
            div { style: "color:{TEXT}; font-size:32px; font-weight:600;", "{label}" }
        }
    }
}

#[component]
fn SectionHeader(title: String) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between; margin-top:18px; flex-shrink:0;",
            div { style: "color:{MUTED}; font-size:28px; font-weight:600;", "{title}" }
            if title == "Networks" {
                button {
                    style: "display:flex; justify-content:center; align-items:center; height:56px; padding:0 28px; border-radius:28px; background:{SUBTLE};",
                    onclick: move |_| FORCE_SCAN.with(|c| c.set(true)),
                    div { style: "color:{TEXT}; font-size:26px;", "Scan" }
                }
            }
        }
    }
}

/// One scanned network. Tap behaviour: connected → no-op; open → join directly;
/// secured+saved → connect by id; secured+unsaved → toggle the passphrase field.
#[component]
fn NetRow(
    ap: Ap,
    expanded: Signal<Option<String>>,
    pass: Signal<String>,
    caret: Signal<usize>,
    focused: Signal<bool>,
) -> Element {
    let level = bars(ap.rssi);
    let tag = if ap.connected {
        "Connected".to_string()
    } else if ap.saved_id.is_some() {
        "Saved".to_string()
    } else if ap.secured {
        "Secured".to_string()
    } else {
        "Open".to_string()
    };
    let tag_color = if ap.connected { ACCENT } else { MUTED };
    let ssid = ap.ssid.clone();

    rsx! {
        button {
            style: "display:flex; flex-direction:row; align-items:center; gap:22px; background:{CARD}; padding:24px; border-radius:20px; flex-shrink:0;",
            onclick: move |_| {
                if ap.connected {
                    return;
                }
                if let Some(id) = ap.saved_id {
                    let _ = wifi::connect(id);
                    FORCE_SCAN.with(|c| c.set(true));
                } else if !ap.secured {
                    join_open(&ssid);
                } else {
                    // Toggle the passphrase field for this row.
                    let mut expanded = expanded;
                    let mut focused = focused;
                    if expanded().as_deref() == Some(ssid.as_str()) {
                        expanded.set(None);
                        focused.set(false);
                        editor_detach();
                    } else {
                        expanded.set(Some(ssid.clone()));
                        pass.clone().set(String::new());
                        caret.clone().set(0);
                    }
                }
            },

            // Signal bars.
            div {
                style: "display:flex; flex-direction:row; align-items:flex-end; gap:5px; width:64px; height:56px; flex-shrink:0;",
                for i in 0u8..4 {
                    div {
                        style: format!(
                            "width:12px; height:{}px; border-radius:3px; background:{};",
                            18 + i as u32 * 12,
                            if i < level { TEXT } else { SUBTLE }
                        ),
                    }
                }
            }

            // SSID + tag.
            div {
                style: "display:flex; flex-direction:column; gap:6px; flex-grow:1; min-width:0; overflow:hidden;",
                div { style: "color:{TEXT}; font-size:36px; font-weight:600;", "{ap.ssid}" }
                div {
                    style: "display:flex; flex-direction:row; gap:14px;",
                    div { style: format!("color:{}; font-size:24px;", tag_color), "{tag}" }
                    if ap.secured {
                        div { style: "color:{MUTED}; font-size:24px;", "· locked" }
                    }
                }
            }
        }
    }
}

/// Passphrase entry for an unsaved secured network — a `data-input` field (the
/// renderer draws value + caret, the IME shows the soft keyboard) + a Join button.
/// Mirrors the Signal composer. Value is masked (•) for display.
#[component]
fn PassField(
    ssid: String,
    expanded: Signal<Option<String>>,
    pass: Signal<String>,
    caret: Signal<usize>,
    focused: Signal<bool>,
) -> Element {
    let raw = pass();
    let masked: String = "•".repeat(raw.chars().count());
    let join_ssid = ssid.clone();
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; gap:16px; padding:0 24px 8px 24px; flex-shrink:0;",
            div {
                "data-input": "1",
                "value": "{masked}",
                "caret": "{caret}",
                "focused": if focused() { "1" } else { "0" },
                style: format!(
                    "display:flex; flex-grow:1; height:88px; padding:0 24px; border-radius:18px; font-size:34px; color:{}; background:{};",
                    TEXT, FIELD
                ),
                onmousedown: move |_e| {
                    let mut focused = focused;
                    if !focused() {
                        focused.set(true);
                        let n = pass().chars().count() as u32;
                        editor_attach("password", "Password", "", n, n);
                    }
                },
                // An onmousemove listener is what marks this element focusable in
                // dioxus-canvas (so onmousedown — which attaches the IME — fires).
                onmousemove: move |_e| {
                    caret.clone().set(caret_at(&pass(), 0.0));
                },
                onfocusout: move |_| {
                    let mut focused = focused;
                    if focused() {
                        focused.set(false);
                        editor_detach();
                    }
                },
                onkeydown: move |e| {
                    let k = e.key().to_string();
                    let mut pass = pass;
                    let mut caret = caret;
                    let chars: Vec<char> = pass().chars().collect();
                    match k.as_str() {
                        "Enter" => {
                            join_secured(&join_ssid, &pass());
                            expanded.clone().set(None);
                            focused.clone().set(false);
                            editor_detach();
                        }
                        "Escape" => { expanded.clone().set(None); focused.clone().set(false); editor_detach(); }
                        "Backspace" => {
                            if !chars.is_empty() {
                                let s: String = chars[..chars.len() - 1].iter().collect();
                                pass.set(s);
                                caret.set(pass().chars().count());
                            }
                        }
                        _ if k.chars().count() == 1 => {
                            let mut s = pass();
                            s.push_str(&k);
                            pass.set(s);
                            caret.set(pass().chars().count());
                        }
                        _ => {}
                    }
                },
            }
            button {
                style: format!(
                    "display:flex; justify-content:center; align-items:center; width:150px; height:88px; border-radius:18px; background:{};",
                    ACCENT
                ),
                onclick: move |_| {
                    join_secured(&ssid, &pass());
                    let mut expanded = expanded;
                    expanded.set(None);
                    focused.clone().set(false);
                    editor_detach();
                },
                div { style: "color:{TEXT}; font-size:30px; font-weight:600;", "Join" }
            }
        }
    }
}

/// One saved network: auto-connect toggle + Forget.
#[component]
fn SavedRow(sv: Saved) -> Element {
    let auto = sv.auto;
    let id = sv.id;
    let auto_bg = if auto { ACCENT } else { SUBTLE };
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; gap:20px; background:{CARD}; padding:24px; border-radius:20px; flex-shrink:0;",
            div {
                style: "display:flex; flex-direction:column; gap:6px; flex-grow:1; min-width:0; overflow:hidden;",
                div { style: "color:{TEXT}; font-size:34px; font-weight:600;", "{sv.ssid}" }
                div { style: "color:{MUTED}; font-size:22px;", "saved · id {sv.id}" }
            }
            // Auto-connect toggle.
            button {
                style: format!(
                    "display:flex; flex-direction:row; align-items:center; gap:12px; height:64px; padding:0 22px; border-radius:32px; background:{};",
                    auto_bg
                ),
                onclick: move |_| {
                    wifi::set_auto_connect(id, !auto);
                    FORCE_SCAN.with(|c| c.set(true));
                },
                div { style: "color:{TEXT}; font-size:24px;", "Auto" }
            }
            // Forget.
            button {
                style: format!(
                    "display:flex; justify-content:center; align-items:center; height:64px; padding:0 26px; border-radius:32px; background:{};",
                    DANGER
                ),
                onclick: move |_| {
                    wifi::remove_network(id);
                    FORCE_SCAN.with(|c| c.set(true));
                },
                div { style: "color:{TEXT}; font-size:24px; font-weight:600;", "Forget" }
            }
        }
    }
}
