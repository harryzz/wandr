//! wart-arbiter-notify — the notification module (Signal bg-receipt M3).
//!
//! Owns the active user-notification list in the core [`Store`]. Apps raise
//! notifications through the `war:notify/notifier` host import, which the host
//! forwards as `notify-post`/`notify-cancel`; the status bar reads the list
//! (`notify-list`) and reports taps (`notify-click <nid>`). On a tap this module
//! foregrounds the owner and — if it's alive — delivers `on-notification-click`
//! to its `notify-handler` export; a dead owner is launched (then foregrounded),
//! opening the app fresh.
//!
//! `title`/`body` arrive percent-encoded (the host's one-line control-socket
//! protocol is whitespace-delimited); this module decodes them.

use wart_arbiter_core::{ArbiterModule, Ctx, Effect, LaunchKind, Reply};

#[derive(Default)]
pub struct NotifyModule;

impl NotifyModule {
    pub fn new() -> Self {
        NotifyModule
    }

    /// Resolve an `owner` token: a bare integer ⇒ a pid (the host self-reports
    /// `getpid()`), resolved to its app-id; otherwise the token IS the app-id
    /// (the CLI). App-ids are never bare integers. Mirrors the alarm module.
    fn resolve_owner(token: &str, ctx: &Ctx) -> Option<String> {
        match token.parse::<i32>() {
            Ok(pid) => ctx.store.app_by_pid(pid).map(|a| a.app_id.clone()),
            Err(_) => Some(token.to_string()),
        }
    }

    /// `notify-post <owner-pid-or-app-id> <id> <enc-title> <enc-body>` — raise or
    /// replace (idempotent on `(app, id)`). title/body are percent-encoded.
    fn cmd_post(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() < 3 {
            return Reply::err("notify-post-args: expected <owner> <id> <enc-title> [enc-body]");
        }
        let Some(app_id) = Self::resolve_owner(t[0], ctx) else {
            return Reply::err(format!("notify-post-unknown-owner {}", t[0]));
        };
        let Ok(id) = t[1].parse::<u64>() else {
            return Reply::err(format!("notify-post-bad-id {:?}", t[1]));
        };
        let title = pct_decode(t[2]);
        let body = t.get(3).map(|s| pct_decode(s)).unwrap_or_default();
        let nid = ctx.store.post_notification(&app_id, id, title, body);
        log::info!("arbiter: notify-post app={app_id} id={id} nid={nid}");
        Reply::ok(format!("notify app={app_id} id={id} nid={nid}"))
    }

    /// `notify-cancel <owner-pid-or-app-id> <id>`.
    fn cmd_cancel(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("notify-cancel-args: expected <owner> <id>");
        }
        let Some(app_id) = Self::resolve_owner(t[0], ctx) else {
            return Reply::err(format!("notify-cancel-unknown-owner {}", t[0]));
        };
        let Ok(id) = t[1].parse::<u64>() else {
            return Reply::err(format!("notify-cancel-bad-id {:?}", t[1]));
        };
        let removed = ctx.store.cancel_notification(&app_id, id);
        log::info!("arbiter: notify-cancel app={app_id} id={id} removed={removed}");
        Reply::ok(format!("cancel app={app_id} id={id} removed={removed}"))
    }

    /// `notify-list` — one line per active notification (the status-bar feed +
    /// CLI inspection). `count=N` header, then `nid=… app=… id=… title=…`.
    fn cmd_list(&mut self, ctx: &mut Ctx) -> Reply {
        let ns = ctx.store.notifications();
        let mut out = format!("count={}", ns.len());
        for n in ns {
            out.push_str(&format!(
                "\n  nid={} app={} id={} title={}",
                n.nid,
                n.app_id,
                n.app_notif_id,
                pct_encode(&n.title),
            ));
        }
        Reply::ok(out)
    }

    /// `notify-click <nid>` — the user tapped notification `nid`. Resolve + clear
    /// it, foreground the owner, and deliver `on-notification-click(app-id)` if
    /// the owner is alive (a dead owner is launched, then foregrounded — it opens
    /// fresh, no per-id callback).
    fn cmd_click(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let Ok(nid) = args.trim().parse::<u64>() else {
            return Reply::err(format!("notify-click-bad-nid {args:?}"));
        };
        let Some(n) = ctx.store.take_notification(nid) else {
            return Reply::err(format!("notify-click-unknown-nid {nid}"));
        };
        match ctx.store.app(&n.app_id).map(|a| a.pid) {
            Some(pid) => {
                // Alive (the common case — a backgrounded service): bring it
                // forward and tell it which notification was tapped.
                ctx.request(Effect::Foreground { app_id: n.app_id.clone() });
                ctx.deliver_to_host(pid, format!("notification-clicked {}\n", n.app_notif_id));
            }
            None => {
                // Dead: launch (inserts the app), then foreground (promotes it) —
                // effects run in order, so the app opens visible.
                ctx.request(Effect::Launch {
                    app_id: n.app_id.clone(),
                    kind: LaunchKind::Gui,
                });
                ctx.request(Effect::Foreground { app_id: n.app_id.clone() });
            }
        }
        log::info!("arbiter: notify-click nid={nid} app={} id={}", n.app_id, n.app_notif_id);
        Reply::ok(format!("click nid={nid} app={} id={}", n.app_id, n.app_notif_id))
    }
}

impl ArbiterModule for NotifyModule {
    fn verbs(&self) -> &[&'static str] {
        &["notify-post", "notify-cancel", "notify-list", "notify-click"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "notify-post" => self.cmd_post(args, ctx),
            "notify-cancel" => self.cmd_cancel(args, ctx),
            "notify-list" => self.cmd_list(ctx),
            "notify-click" => self.cmd_click(args, ctx),
            other => Reply::err(format!("notify-unknown-verb {other}")),
        }
    }
}

/// Conservative percent-encoding: keep `A-Za-z0-9-._`, escape everything else as
/// `%XX` (UTF-8 bytes). Round-trips arbitrary text through the whitespace-
/// delimited control-socket line. Mirror of [`pct_decode`].
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex(b >> 4));
            out.push(hex(b & 0xf));
        }
    }
    out
}

fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (unhex(bytes[i + 1]), unhex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(n: u8) -> char {
    (if n < 10 { b'0' + n } else { b'A' + (n - 10) }) as char
}

fn unhex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wart_arbiter_core::{AppState, Registry, Store};
    use std::time::{Instant, SystemTime};

    fn reg() -> (Registry, Store) {
        let mut r = Registry::new();
        r.register(Box::new(NotifyModule::new()));
        (r, Store::new())
    }
    fn add_app(s: &mut Store, id: &str, pid: i32) {
        s.insert_app(AppState {
            app_id: id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
    }

    #[test]
    fn pct_roundtrip() {
        for t in ["Alice", "hi there", "héllo 🌍\nnext", "100% sure", "a/b?c=d"] {
            assert_eq!(pct_decode(&pct_encode(t)), t);
        }
        assert!(!pct_encode("hi there").contains(' '));
    }

    #[test]
    fn post_list_click_alive_delivers_and_foregrounds() {
        let (mut r, mut store) = reg();
        add_app(&mut store, "sig", 30);
        // post with a spacey title (percent-encoded on the wire)
        let (rep, _) = r
            .dispatch_command("notify-post", &format!("30 1 {} {}", pct_encode("Alice"), pct_encode("hi there")), &mut store)
            .unwrap();
        assert!(rep.render().contains("nid=1"), "{}", rep.render());
        // click nid=1 → Foreground{sig} + notification-clicked 1 to pid 30
        let (_, eff) = r.dispatch_command("notify-click", "1", &mut store).unwrap();
        assert!(eff.iter().any(|e| matches!(e, Effect::Foreground { app_id } if app_id == "sig")));
        assert!(eff.iter().any(|e| matches!(e, Effect::HostLine { pid: 30, line } if line == "notification-clicked 1\n")));
        assert!(store.notifications().is_empty(), "click clears the notification");
    }

    #[test]
    fn click_dead_launches_then_foregrounds() {
        let (mut r, mut store) = reg();
        r.dispatch_command("notify-post", &format!("dead 9 {}", pct_encode("t")), &mut store).unwrap();
        let nid = 1u64;
        let (_, eff) = r.dispatch_command("notify-click", &nid.to_string(), &mut store).unwrap();
        assert!(eff.iter().any(|e| matches!(e, Effect::Launch { app_id, kind } if app_id == "dead" && *kind == LaunchKind::Gui)));
        assert!(eff.iter().any(|e| matches!(e, Effect::Foreground { app_id } if app_id == "dead")));
    }
}
