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

use wandr_arbiter_core::{ArbiterModule, Ctx, Event, Reply};

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
}

impl Default for NetModule {
    fn default() -> Self {
        Self {
            state: NetState::offline(),
            subscribers: Vec::new(),
        }
    }
}

impl NetModule {
    pub fn new() -> Self {
        Self::default()
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
        &["report-net-state", "net-status", "net-subscribe"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "report-net-state" => self.cmd_report_net_state(args, ctx),
            "net-status" => self.cmd_net_status(),
            "net-subscribe" => self.cmd_net_subscribe(args, ctx),
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
}
