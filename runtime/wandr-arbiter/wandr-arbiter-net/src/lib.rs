//! wandr-arbiter-net — the ConnectivityService role (Arbiter Inc., task 88 M1).
//!
//! The arbiter's single owner of *connectivity state* under `--no-art`, mirroring
//! `wandr-arbiter-sensors`: the daemon (`wandr-net`) owns the mechanism (associate /
//! DHCP / route / DNS via `wandr-hal-net`) and reports state up; this module holds
//! that state and notifies guests. Two responsibilities, both pure:
//!
//!   1. **Status of record.** Ingest `report-net-state` from the daemon and keep
//!      the current link snapshot (online?, transport, ssid, ip). `net-status`
//!      answers queries (status bar, debugging, the guest WIT `get-status`).
//!   2. **Change notification** — the single most-used `ConnectivityManager`
//!      feature (`registerDefaultNetworkCallback`): when online↔offline (or the
//!      active network) changes, fan an `on-connectivity-change` line to every
//!      subscribed guest via [`Effect::HostLine`]. Guests subscribe through the
//!      host (`net-subscribe <pid>` when they export the handler) and are dropped
//!      on `SurfaceRemoved` (process exit).
//!
//! Never touches binder/sockets — desktop-testable via the `report-net-state` /
//! `net-subscribe` verbs with no device.

use std::path::PathBuf;

use wandr_arbiter_core::{ArbiterModule, Ctx, Event, Reply};

// ── saved-network store (task 90 M3 — the WifiConfigManager role) ─────────────

/// One persisted network (the `wifi-config` the WIT `saved-network` mirrors). The
/// PSK is held plaintext (root-only file, 0600) — no worse than the framework's
/// own `WifiConfigStore.xml` on this device; keystore2-wrapping is a hardening
/// follow-up (decided 2026-06-09).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedNet {
    pub id: u32,
    pub ssid: String,
    /// `open|owe|wpa-psk|sae|wpa-eap` (the WIT `security-kind` wire token).
    pub security: String,
    pub psk: String,
    pub auto_connect: bool,
    pub hidden: bool,
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// base64 (standard alphabet, padded). SSID/PSK are b64'd both on the wire (so they
/// tokenise across the host→arbiter hops) and in the JSON store (so the values are
/// always JSON-safe — no escaping needed in the hand-rolled serializer).
pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18 & 63) as usize] as char);
        out.push(B64[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// base64 decode (standard alphabet); skips `=` padding + whitespace.
pub fn b64_decode(s: &str) -> Vec<u8> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        let Some(v) = val(c) else { continue };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

/// b64-decode a wire token straight to a `String` (lossy — SSIDs are conventionally
/// UTF-8; a non-UTF-8 SSID is rare and the lossy form is acceptable for display).
fn b64_to_string(s: &str) -> String {
    String::from_utf8_lossy(&b64_decode(s)).into_owned()
}

/// Serialize the saved networks to a JSON array. ssid/psk are b64 so the values
/// never contain `"`/`\`/`,` — the hand-rolled writer (matching the arbiter's
/// no-serde convention) is then trivially correct.
fn saved_to_json(saved: &[SavedNet]) -> String {
    let mut s = String::from("[\n");
    for (i, n) in saved.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        s.push_str(&format!(
            "  {{\"id\":{},\"ssid\":\"{}\",\"sec\":\"{}\",\"psk\":\"{}\",\"auto\":{},\"hidden\":{}}}",
            n.id,
            b64_encode(n.ssid.as_bytes()),
            n.security,
            b64_encode(n.psk.as_bytes()),
            n.auto_connect,
            n.hidden,
        ));
    }
    s.push_str("\n]\n");
    s
}

/// Parse the JSON array written by [`saved_to_json`]. Field-scan per `{…}` object
/// (the values are b64/tokens with no nested delimiters, so a positional scan is
/// robust — same shape as `wandr-hal-net`'s WifiConfigStore parser).
fn saved_from_json(body: &str) -> Vec<SavedNet> {
    fn str_field(obj: &str, key: &str) -> Option<String> {
        let k = format!("\"{key}\":\"");
        let start = obj.find(&k)? + k.len();
        let end = obj[start..].find('"')? + start;
        Some(obj[start..end].to_string())
    }
    fn raw_field<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
        let k = format!("\"{key}\":");
        let start = obj.find(&k)? + k.len();
        let end = obj[start..]
            .find([',', '}'])
            .map(|e| e + start)
            .unwrap_or(obj.len());
        Some(obj[start..end].trim())
    }
    let mut out = Vec::new();
    for obj in body.split('}') {
        let Some(id) = raw_field(obj, "id").and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        let ssid = str_field(obj, "ssid").map(|b| b64_to_string(&b)).unwrap_or_default();
        let security = str_field(obj, "sec").unwrap_or_else(|| "wpa-psk".into());
        let psk = str_field(obj, "psk").map(|b| b64_to_string(&b)).unwrap_or_default();
        let auto_connect = raw_field(obj, "auto") == Some("true");
        let hidden = raw_field(obj, "hidden") == Some("true");
        out.push(SavedNet { id, ssid, security, psk, auto_connect, hidden });
    }
    out
}

/// One connectivity snapshot — what the daemon reports and guests query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetState {
    pub online: bool,
    /// Active transport: "wifi" (M1), later "ethernet"/"cellular"; "none" offline.
    pub transport: String,
    pub ssid: Option<String>,
    pub ip: Option<String>,
}

impl NetState {
    fn offline() -> Self {
        NetState {
            online: false,
            transport: "none".into(),
            ssid: None,
            ip: None,
        }
    }

    /// The wire form fanned to guests / returned by `net-status`:
    /// `online wifi <ssid> <ip>` or `offline`.
    fn wire(&self) -> String {
        if self.online {
            format!(
                "online {} {} {}",
                self.transport,
                self.ssid.as_deref().unwrap_or("-"),
                self.ip.as_deref().unwrap_or("-"),
            )
        } else {
            "offline".to_string()
        }
    }
}

pub struct NetModule {
    state: NetState,
    /// Guest pids that export `on-connectivity-change` (registered via the host).
    subscribers: Vec<i32>,
    /// The WifiConfigManager registry (task 90 M3): persisted saved networks.
    saved: Vec<SavedNet>,
    /// Monotonic id allocator for saved networks (never reused within a session).
    next_id: u32,
    /// Where the registry persists (`None` = in-memory only, for unit tests).
    store_path: Option<PathBuf>,
}

impl Default for NetModule {
    fn default() -> Self {
        Self {
            state: NetState::offline(),
            subscribers: Vec::new(),
            saved: Vec::new(),
            next_id: 1,
            store_path: None,
        }
    }
}

impl NetModule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a persisted saved-network store at `path` (the arbiter binary
    /// passes the on-device path; unit tests use `new()` / a temp path). Loads any
    /// existing registry; `next_id` resumes above the highest stored id.
    pub fn with_store(path: PathBuf) -> Self {
        let saved = std::fs::read_to_string(&path)
            .ok()
            .map(|b| saved_from_json(&b))
            .unwrap_or_default();
        let next_id = saved.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        log::info!("net: loaded {} saved network(s) from {}", saved.len(), path.display());
        Self {
            state: NetState::offline(),
            subscribers: Vec::new(),
            saved,
            next_id,
            store_path: Some(path),
        }
    }

    /// Write the registry to the store file (0600 — it holds plaintext PSKs).
    /// No-op when in-memory (`store_path == None`).
    fn persist(&self) {
        let Some(path) = self.store_path.as_ref() else { return };
        let json = saved_to_json(&self.saved);
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &json) {
            log::warn!("net: persist saved networks → {} failed: {e}", tmp.display());
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            log::warn!("net: persist rename → {} failed: {e}", path.display());
        }
    }

    /// `wifi-saved-list` — one `saved …` line per network (ssid b64'd) + `ok`.
    fn cmd_saved_list(&self) -> Reply {
        let mut body = String::new();
        for n in &self.saved {
            body.push_str(&format!(
                "saved id={} ssid={} sec={} auto={} hidden={}\n",
                n.id,
                b64_encode(n.ssid.as_bytes()),
                n.security,
                n.auto_connect as u8,
                n.hidden as u8,
            ));
        }
        body.push_str(&format!("ok count={}", self.saved.len()));
        Reply::ok(body)
    }

    /// `wifi-saved-add <b64ssid> <sec> <b64psk> <auto> <hidden>` → assign an id,
    /// persist, `ok id=<n>`. A duplicate SSID+security UPDATES the existing entry's
    /// creds/flags (returning its id) rather than creating a second row — the
    /// WifiConfigManager keys a network by (ssid, security).
    fn cmd_saved_add(&mut self, args: &str) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        let [bssid, sec, bpsk, auto, hidden] = t.as_slice() else {
            return Reply::err("wifi-saved-add-usage <b64ssid> <sec> <b64psk> <auto> <hidden>");
        };
        let ssid = b64_to_string(bssid);
        if ssid.is_empty() {
            return Reply::err("wifi-saved-add empty-ssid");
        }
        let psk = b64_to_string(bpsk);
        let auto_connect = *auto == "1" || *auto == "true";
        let hidden = *hidden == "1" || *hidden == "true";
        if let Some(n) = self
            .saved
            .iter_mut()
            .find(|n| n.ssid == ssid && n.security == *sec)
        {
            n.psk = psk;
            n.auto_connect = auto_connect;
            n.hidden = hidden;
            let id = n.id;
            self.persist();
            return Reply::ok(format!("id={id}"));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.saved.push(SavedNet {
            id,
            ssid,
            security: sec.to_string(),
            psk,
            auto_connect,
            hidden,
        });
        self.persist();
        log::info!("net: saved network id={id} added");
        Reply::ok(format!("id={id}"))
    }

    /// `wifi-saved-update <id> <b64ssid> <sec> <b64psk> <auto> <hidden>`.
    fn cmd_saved_update(&mut self, args: &str) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        let [id, bssid, sec, bpsk, auto, hidden] = t.as_slice() else {
            return Reply::err("wifi-saved-update-usage <id> <b64ssid> <sec> <b64psk> <auto> <hidden>");
        };
        let Some(id) = id.parse::<u32>().ok() else {
            return Reply::err("wifi-saved-update bad-id");
        };
        let Some(n) = self.saved.iter_mut().find(|n| n.id == id) else {
            return Reply::err("not-found");
        };
        n.ssid = b64_to_string(bssid);
        n.security = sec.to_string();
        n.psk = b64_to_string(bpsk);
        n.auto_connect = *auto == "1" || *auto == "true";
        n.hidden = *hidden == "1" || *hidden == "true";
        self.persist();
        Reply::ok(format!("id={id}"))
    }

    /// `wifi-saved-remove <id>` — idempotent (removing an absent id is `ok`).
    fn cmd_saved_remove(&mut self, args: &str) -> Reply {
        let Some(id) = args.split_whitespace().next().and_then(|t| t.parse::<u32>().ok()) else {
            return Reply::err("wifi-saved-remove-usage <id>");
        };
        let before = self.saved.len();
        self.saved.retain(|n| n.id != id);
        if self.saved.len() != before {
            self.persist();
            log::info!("net: saved network id={id} removed");
        }
        Reply::ok(format!("id={id}"))
    }

    /// `wifi-saved-auto-connect <id> <0|1>` — toggle auto-join for a saved network.
    fn cmd_saved_auto(&mut self, args: &str) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        let [id, on] = t.as_slice() else {
            return Reply::err("wifi-saved-auto-connect-usage <id> <0|1>");
        };
        let Some(id) = id.parse::<u32>().ok() else {
            return Reply::err("wifi-saved-auto-connect bad-id");
        };
        let Some(n) = self.saved.iter_mut().find(|n| n.id == id) else {
            return Reply::err("not-found");
        };
        n.auto_connect = *on == "1" || *on == "true";
        let auto = n.auto_connect as u8;
        self.persist();
        Reply::ok(format!("id={id} auto={auto}"))
    }

    /// `wifi-saved-creds <id>` → `ssid=<b64> psk=<b64> sec=<s>`. Used by the bin's
    /// `wifi-connect-saved` to resolve a saved id to creds before relaying connect.
    fn cmd_saved_creds(&self, args: &str) -> Reply {
        let Some(id) = args.split_whitespace().next().and_then(|t| t.parse::<u32>().ok()) else {
            return Reply::err("wifi-saved-creds-usage <id>");
        };
        match self.saved.iter().find(|n| n.id == id) {
            Some(n) => Reply::ok(format!(
                "ssid={} psk={} sec={}",
                b64_encode(n.ssid.as_bytes()),
                b64_encode(n.psk.as_bytes()),
                n.security,
            )),
            None => Reply::err("not-found"),
        }
    }

    /// `wifi-auto-network` → the creds of the first auto-connect saved network
    /// (`ssid=<b64> psk=<b64> sec=<s>`), or `none`. The wandr-net daemon queries
    /// this at bring-up to auto-join from the wandr store (replacing its single
    /// WifiConfigStore.xml read).
    fn cmd_auto_network(&self) -> Reply {
        match self.saved.iter().find(|n| n.auto_connect) {
            Some(n) => Reply::ok(format!(
                "ssid={} psk={} sec={}",
                b64_encode(n.ssid.as_bytes()),
                b64_encode(n.psk.as_bytes()),
                n.security,
            )),
            None => Reply::ok("none"),
        }
    }

    /// Push the current state to one host (the guest's `on-connectivity-change`).
    fn notify_one(&self, pid: i32, ctx: &mut Ctx) {
        ctx.deliver_to_host(pid, format!("net-changed {}\n", self.state.wire()));
    }

    /// Fan the current state to every subscribed guest.
    fn notify_all(&self, ctx: &mut Ctx) {
        for &pid in &self.subscribers {
            self.notify_one(pid, ctx);
        }
    }

    /// `report-net-state online wifi <ssid> <ip>` / `report-net-state offline`
    /// — the daemon's state push. Only fans to guests on an actual change.
    fn cmd_report_net_state(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let toks: Vec<&str> = args.split_whitespace().collect();
        let new = match toks.first().copied() {
            Some("online") => {
                let transport = toks.get(1).unwrap_or(&"wifi").to_string();
                let ssid = toks.get(2).filter(|s| **s != "-").map(|s| s.to_string());
                let ip = toks.get(3).filter(|s| **s != "-").map(|s| s.to_string());
                NetState {
                    online: true,
                    transport,
                    ssid,
                    ip,
                }
            }
            Some("offline") => NetState::offline(),
            _ => return Reply::err("report-net-state-usage online <transport> <ssid> <ip> | offline"),
        };
        if new != self.state {
            self.state = new;
            log::info!("net: state → {}", self.state.wire());
            self.notify_all(ctx);
        }
        Reply::ok(format!("report-net-state {}", self.state.wire()))
    }

    /// `net-status` — current link snapshot (status bar / `get-status` / debug).
    fn cmd_net_status(&self) -> Reply {
        Reply::ok(self.state.wire())
    }

    /// `net-subscribe <pid>` — the host registers a guest that exports
    /// `on-connectivity-change`. Immediately delivers the current state so the
    /// guest starts coherent (it learns "online" even if it subscribed late).
    fn cmd_net_subscribe(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let Some(pid) = args.split_whitespace().next().and_then(|t| t.parse::<i32>().ok()) else {
            return Reply::err("net-subscribe-usage <pid>");
        };
        if !self.subscribers.contains(&pid) {
            self.subscribers.push(pid);
        }
        self.notify_one(pid, ctx);
        Reply::ok(format!("net-subscribe {pid}"))
    }
}

impl ArbiterModule for NetModule {
    fn verbs(&self) -> &[&'static str] {
        &[
            "report-net-state",
            "net-status",
            "net-subscribe",
            // Task 90 M3 — WifiConfigManager (saved-network store + auto-connect).
            "wifi-saved-list",
            "wifi-saved-add",
            "wifi-saved-update",
            "wifi-saved-remove",
            "wifi-saved-auto-connect",
            "wifi-saved-creds",
            "wifi-auto-network",
        ]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "report-net-state" => self.cmd_report_net_state(args, ctx),
            "net-status" => self.cmd_net_status(),
            "net-subscribe" => self.cmd_net_subscribe(args, ctx),
            "wifi-saved-list" => self.cmd_saved_list(),
            "wifi-saved-add" => self.cmd_saved_add(args),
            "wifi-saved-update" => self.cmd_saved_update(args),
            "wifi-saved-remove" => self.cmd_saved_remove(args),
            "wifi-saved-auto-connect" => self.cmd_saved_auto(args),
            "wifi-saved-creds" => self.cmd_saved_creds(args),
            "wifi-auto-network" => self.cmd_auto_network(),
            other => Reply::err(format!("net-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut Ctx) {
        // A subscribed guest's process exited — drop it so we stop pushing to a
        // dead pid (mirrors the sensors module's holder cleanup).
        if let Event::SurfaceRemoved { pid } = ev {
            self.subscribers.retain(|&p| p != *pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wandr_arbiter_core::{Effect, Registry, Store};

    fn reg() -> Registry {
        let mut r = Registry::new();
        r.register(Box::new(NetModule::new()));
        r
    }

    /// Collect the `net-changed …` lines from a batch of effects.
    fn host_lines(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::HostLine { pid, line } => Some(format!("{pid}:{}", line.trim())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn net_status_defaults_offline() {
        let mut r = reg();
        let mut store = Store::new();
        let (reply, _) = r.dispatch_command("net-status", "", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(ref s) if s == "offline"), "{reply:?}");
    }

    #[test]
    fn report_online_then_status() {
        let mut r = reg();
        let mut store = Store::new();
        r.dispatch_command("report-net-state", "online wifi HomeNet 192.168.1.50", &mut store)
            .unwrap();
        let (reply, _) = r.dispatch_command("net-status", "", &mut store).unwrap();
        assert!(
            matches!(reply, Reply::Ok(ref s) if s == "online wifi HomeNet 192.168.1.50"),
            "{reply:?}"
        );
    }

    #[test]
    fn subscriber_notified_on_subscribe_and_change() {
        let mut r = reg();
        let mut store = Store::new();
        // Subscribe while offline → immediate "offline" push.
        let (_, eff) = r.dispatch_command("net-subscribe", "100", &mut store).unwrap();
        assert_eq!(host_lines(&eff), vec!["100:net-changed offline"]);
        // Going online fans the change to the subscriber.
        let (_, eff) = r
            .dispatch_command("report-net-state", "online wifi Net 10.0.0.5", &mut store)
            .unwrap();
        assert_eq!(host_lines(&eff), vec!["100:net-changed online wifi Net 10.0.0.5"]);
    }

    #[test]
    fn no_notify_without_change() {
        let mut r = reg();
        let mut store = Store::new();
        r.dispatch_command("net-subscribe", "100", &mut store).unwrap();
        r.dispatch_command("report-net-state", "online wifi Net 10.0.0.5", &mut store)
            .unwrap();
        // Re-reporting the identical state must not re-notify.
        let (_, eff) = r
            .dispatch_command("report-net-state", "online wifi Net 10.0.0.5", &mut store)
            .unwrap();
        assert!(host_lines(&eff).is_empty(), "duplicate state should not re-notify");
    }

    #[test]
    fn surface_removed_drops_subscriber() {
        let mut r = reg();
        let mut store = Store::new();
        r.dispatch_command("net-subscribe", "100", &mut store).unwrap();
        r.dispatch_event(Event::SurfaceRemoved { pid: 100 }, &mut store);
        // After the guest dies, a state change fans to nobody.
        let (_, eff) = r
            .dispatch_command("report-net-state", "online wifi Net 10.0.0.5", &mut store)
            .unwrap();
        assert!(host_lines(&eff).is_empty(), "dead subscriber should be dropped");
    }

    // ── WifiConfigManager (task 90 M3) ────────────────────────────────────────

    fn ok_body(r: &Reply) -> String {
        match r {
            Reply::Ok(s) => s.clone(),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    /// Drive the module directly (no Ctx needed for the registry commands).
    fn add(m: &mut NetModule, ssid: &str, sec: &str, psk: &str, auto: bool) -> u32 {
        let r = m.cmd_saved_add(&format!(
            "{} {sec} {} {} 0",
            b64_encode(ssid.as_bytes()),
            b64_encode(psk.as_bytes()),
            auto as u8
        ));
        ok_body(&r).strip_prefix("id=").unwrap().parse().unwrap()
    }

    #[test]
    fn saved_add_list_assigns_monotonic_ids() {
        let mut m = NetModule::new();
        let a = add(&mut m, "HomeNet", "wpa-psk", "secret1", true);
        let b = add(&mut m, "Cafe", "wpa-psk", "latte", false);
        assert_eq!((a, b), (1, 2));
        let body = ok_body(&m.cmd_saved_list());
        assert!(body.contains(&format!("saved id=1 ssid={} sec=wpa-psk auto=1 hidden=0", b64_encode(b"HomeNet"))), "{body}");
        assert!(body.contains("ok count=2"), "{body}");
    }

    #[test]
    fn saved_add_same_ssid_updates_not_duplicates() {
        let mut m = NetModule::new();
        let a = add(&mut m, "HomeNet", "wpa-psk", "old", true);
        let b = add(&mut m, "HomeNet", "wpa-psk", "new", false);
        assert_eq!(a, b, "same ssid+security updates the existing entry");
        assert_eq!(m.saved.len(), 1);
        assert_eq!(m.saved[0].psk, "new");
        assert!(!m.saved[0].auto_connect);
    }

    #[test]
    fn saved_creds_and_auto_network() {
        let mut m = NetModule::new();
        add(&mut m, "HomeNet", "wpa-psk", "secret1", false);
        let id2 = add(&mut m, "Auto", "sae", "autopass", true);
        // creds resolves an id to b64 ssid/psk.
        let creds = ok_body(&m.cmd_saved_creds(&id2.to_string()));
        assert_eq!(
            creds,
            format!("ssid={} psk={} sec=sae", b64_encode(b"Auto"), b64_encode(b"autopass"))
        );
        // auto-network returns the FIRST auto-connect network's creds.
        let auto = ok_body(&m.cmd_auto_network());
        assert!(auto.contains(&format!("ssid={}", b64_encode(b"Auto"))), "{auto}");
        // Flip auto off → auto-network is none.
        m.cmd_saved_auto(&format!("{id2} 0"));
        assert_eq!(ok_body(&m.cmd_auto_network()), "none");
    }

    #[test]
    fn saved_remove_is_idempotent() {
        let mut m = NetModule::new();
        let id = add(&mut m, "Gone", "wpa-psk", "x", false);
        assert!(matches!(m.cmd_saved_remove(&id.to_string()), Reply::Ok(_)));
        assert!(m.saved.is_empty());
        // Removing again is still ok (idempotent).
        assert!(matches!(m.cmd_saved_remove(&id.to_string()), Reply::Ok(_)));
        assert!(matches!(m.cmd_saved_creds(&id.to_string()), Reply::Err(ref e) if e == "not-found"));
    }

    #[test]
    fn json_roundtrip_preserves_everything() {
        let nets = vec![
            SavedNet { id: 1, ssid: "My Net".into(), security: "wpa-psk".into(), psk: "p@ss,\"w0rd".into(), auto_connect: true, hidden: false },
            SavedNet { id: 7, ssid: "Hidden".into(), security: "sae".into(), psk: "x".into(), auto_connect: false, hidden: true },
        ];
        let json = saved_to_json(&nets);
        assert_eq!(saved_from_json(&json), nets, "b64 fields survive special chars + commas/quotes");
    }
}
