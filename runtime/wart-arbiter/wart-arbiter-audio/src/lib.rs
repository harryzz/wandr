//! wart-arbiter-audio — the AudioService role (Arbiter Inc.).
//!
//! **M1: the audio-focus stack.** Audio focus is *the* canonical cross-app
//! arbitration: a guest cannot decide to pause another guest, so a central
//! authority must. This module owns the focus stack and decides, on each
//! request, what happens to the previous owner — permanent **loss** (a `gain`
//! request evicts it), **loss-transient** (a `gain-transient` pauses it), or
//! **duck** (a `gain-transient-may-duck` lowers its volume) — and restores the
//! prior owner with **gain** when the top entry abandons. Each transition is an
//! `on-focus-changed <kind>` push to the affected host (`deliver_to_host`); the
//! guest pauses / ducks / resumes. The host/guest WIT wiring is M2; the
//! comms-session (IN_COMMUNICATION) + routing appliers are M3.
//!
//! Focus is runtime-only (it dies with a reboot), so the stack lives in this
//! module, not the durable core Store.

use wart_arbiter_core::{ArbiterModule, Ctx, Effect, Event, Reply};

/// What a guest requests (Android AUDIOFOCUS_GAIN family).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusKind {
    /// Permanent — evict every prior owner (they release).
    Gain,
    /// Transient — the prior owner pauses, resumes when we abandon.
    GainTransient,
    /// Transient may-duck — the prior owner lowers its volume, keeps playing.
    GainTransientMayDuck,
}

impl FocusKind {
    fn from_wire(s: &str) -> Option<Self> {
        match s {
            "gain"                                      => Some(Self::Gain),
            "gain-transient"           | "transient"    => Some(Self::GainTransient),
            "gain-transient-may-duck"  | "duck"         => Some(Self::GainTransientMayDuck),
            _ => None,
        }
    }
    fn as_wire(&self) -> &'static str {
        match self {
            Self::Gain                 => "gain",
            Self::GainTransient        => "gain-transient",
            Self::GainTransientMayDuck => "gain-transient-may-duck",
        }
    }
}

/// What an affected owner is told (Android AUDIOFOCUS_LOSS family), delivered as
/// `on-focus-changed <change>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusChange { Loss, LossTransient, Duck, Gain }

impl FocusChange {
    fn as_wire(&self) -> &'static str {
        match self {
            Self::Loss          => "loss",
            Self::LossTransient => "loss-transient",
            Self::Duck          => "duck",
            Self::Gain          => "gain",
        }
    }
}

#[derive(Clone, Debug)]
struct FocusEntry {
    pid:    i32,
    app_id: String,
    kind:   FocusKind,
}

/// App-assigned notification id the audio module uses for the ongoing-call
/// badge (its own id space, keyed per owner app like any notifier).
const CALL_NOTIF_ID: u64 = 0xCA11;

/// The audio-focus arbiter. `stack.last()` is the current owner.
#[derive(Default)]
pub struct AudioModule {
    stack: Vec<FocusEntry>,
    /// M3 — the pid currently in a comms session (a VoIP call), if any. One call
    /// at a time; it holds permanent focus + drives the global audio mode.
    comms: Option<i32>,
}

impl AudioModule {
    pub fn new() -> Self {
        AudioModule::default()
    }

    /// Resolve an owner token to `(pid, app_id)`. A bare integer is a pid (the
    /// host self-reports `getpid()`); resolve its app-id best-effort ("?" if
    /// untracked — focus still works on the raw pid). Otherwise the token is an
    /// app-id (the CLI), resolved to its pid; unknown app-id ⇒ None. Mirrors the
    /// notify/alarm modules.
    fn resolve(token: &str, ctx: &Ctx) -> Option<(i32, String)> {
        match token.parse::<i32>() {
            Ok(pid) => {
                let app_id = ctx.store.app_by_pid(pid)
                    .map(|a| a.app_id.clone())
                    .unwrap_or_else(|| "?".to_string());
                Some((pid, app_id))
            }
            Err(_) => ctx.store.app(token).map(|a| (a.pid, a.app_id.clone())),
        }
    }

    fn push_change(ctx: &mut Ctx, pid: i32, change: FocusChange) {
        ctx.deliver_to_host(pid, format!("on-focus-changed {}\n", change.as_wire()));
    }

    /// `audio-focus-request <pid|app-id> <kind>` — grant focus, demoting the
    /// previous owner per `kind`.
    fn cmd_request(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("audio-focus-request-args: expected <pid|app-id> <kind>");
        }
        let Some(kind) = FocusKind::from_wire(t[1]) else {
            return Reply::err(format!("audio-focus-bad-kind {:?}", t[1]));
        };
        let Some((pid, app_id)) = Self::resolve(t[0], ctx) else {
            return Reply::err(format!("audio-focus-unknown-owner {}", t[0]));
        };
        self.grant(pid, app_id.clone(), kind, ctx);
        Reply::ok(format!("granted pid={pid} app={app_id} kind={}", kind.as_wire()))
    }

    /// Push `(pid, app_id)` onto the focus stack as the new owner, demoting the
    /// previous owner per `kind` (loss / loss-transient / duck). Shared by the
    /// focus-request verb and the comms-session start (which grabs `Gain`).
    fn grant(&mut self, pid: i32, app_id: String, kind: FocusKind, ctx: &mut Ctx) {
        // Re-request from the same owner: drop its stale entry first so it
        // doesn't get a spurious loss push or a duplicate stack slot.
        self.stack.retain(|e| e.pid != pid);
        let prev = self.stack.last().cloned();

        match kind {
            FocusKind::Gain => {
                // Permanent grab: every prior owner releases.
                for e in &self.stack {
                    Self::push_change(ctx, e.pid, FocusChange::Loss);
                }
                self.stack.clear();
            }
            FocusKind::GainTransient => {
                if let Some(p) = &prev {
                    Self::push_change(ctx, p.pid, FocusChange::LossTransient);
                }
            }
            FocusKind::GainTransientMayDuck => {
                if let Some(p) = &prev {
                    Self::push_change(ctx, p.pid, FocusChange::Duck);
                }
            }
        }

        self.stack.push(FocusEntry { pid, app_id: app_id.clone(), kind });
        log::info!("arbiter: audio-focus granted pid={pid} app={app_id} kind={}", kind.as_wire());
    }

    /// `audio-focus-abandon <pid|app-id>` — release focus; if it was the owner,
    /// the prior (transient/duck) owner underneath regains.
    fn cmd_abandon(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let token = args.trim();
        if token.is_empty() {
            return Reply::err("audio-focus-abandon-args: expected <pid|app-id>");
        }
        let Some((pid, _app)) = Self::resolve(token, ctx) else {
            return Reply::err(format!("audio-focus-unknown-owner {token}"));
        };
        self.drop_pid(pid, ctx);
        Reply::ok(format!("abandoned pid={pid}"))
    }

    /// Remove `pid` from the stack; if it was the current owner, the new top
    /// regains focus. Shared by abandon + the death-watch (SurfaceRemoved).
    fn drop_pid(&mut self, pid: i32, ctx: &mut Ctx) -> bool {
        let was_owner = self.stack.last().map(|e| e.pid) == Some(pid);
        let existed = self.stack.iter().any(|e| e.pid == pid);
        if !existed {
            return false;
        }
        self.stack.retain(|e| e.pid != pid);
        if was_owner {
            if let Some(top) = self.stack.last() {
                Self::push_change(ctx, top.pid, FocusChange::Gain);
                log::info!("arbiter: audio-focus restored pid={} app={}", top.pid, top.app_id);
            }
        }
        true
    }

    /// `audio-call-start <pid|app-id>` — begin a comms session (a VoIP call):
    /// grab transient focus (music pauses, resuming after the call), switch the
    /// global audio mode to IN_COMMUNICATION (the owner host applies it), and
    /// raise an ongoing-call notification. One call at a time.
    fn cmd_call_start(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let token = args.trim();
        let Some((pid, app_id)) = Self::resolve(token, ctx) else {
            return Reply::err(format!("audio-call-unknown-owner {token}"));
        };
        // Transient focus → music pauses now and resumes when the call ends
        // (Android telephony uses GAIN_TRANSIENT for exactly this).
        self.grant(pid, app_id.clone(), FocusKind::GainTransient, ctx);
        self.comms = Some(pid);
        // The owner host (it holds the binder connection) applies the mode.
        ctx.deliver_to_host(pid, "audio-policy set-mode comm\n");
        // Ongoing-call badge in the status bar (notify module's Store).
        ctx.store.post_notification(&app_id, CALL_NOTIF_ID, "Ongoing call".into(), app_id.clone());
        // Keep the call host out of doze (the power module reacts). M3b.
        ctx.emit(Event::CommsActive { pid, active: true });
        ctx.request(Effect::Persist);
        log::info!("arbiter: audio-call-start pid={pid} app={app_id} → IN_COMMUNICATION");
        Reply::ok(format!("call-start pid={pid} app={app_id}"))
    }

    /// `audio-call-end <pid|app-id>` — end the comms session: restore NORMAL
    /// mode, release focus (the prior owner regains), clear the badge.
    fn cmd_call_end(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let token = args.trim();
        let Some((pid, app_id)) = Self::resolve(token, ctx) else {
            return Reply::err(format!("audio-call-unknown-owner {token}"));
        };
        if self.comms != Some(pid) {
            return Reply::err(format!("audio-call-not-in-session pid={pid}"));
        }
        self.comms = None;
        ctx.deliver_to_host(pid, "audio-policy set-mode normal\n");
        self.drop_pid(pid, ctx); // release focus → prior owner regains
        ctx.store.cancel_notification(&app_id, CALL_NOTIF_ID);
        // Release the doze keep-alive (the power module reacts). M3b.
        ctx.emit(Event::CommsActive { pid, active: false });
        ctx.request(Effect::Persist);
        log::info!("arbiter: audio-call-end pid={pid} app={app_id} → NORMAL");
        Reply::ok(format!("call-end pid={pid} app={app_id}"))
    }

    /// `audio-route <pid|app-id> <speaker|earpiece>` — the speaker toggle: the
    /// owner host applies `setForceUse(COMMUNICATION, …)`.
    fn cmd_route(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("audio-route-args: expected <pid|app-id> <speaker|earpiece>");
        }
        let route = match t[1] {
            "speaker" | "earpiece" => t[1],
            other => return Reply::err(format!("audio-route-bad-target {other:?}")),
        };
        let Some((pid, _app)) = Self::resolve(t[0], ctx) else {
            return Reply::err(format!("audio-route-unknown-owner {}", t[0]));
        };
        ctx.deliver_to_host(pid, format!("audio-policy set-route {route}\n"));
        log::info!("arbiter: audio-route pid={pid} → {route}");
        Reply::ok(format!("route pid={pid} {route}"))
    }

    /// `audio-focus-list` — the stack, owner last. CLI + inspection.
    fn cmd_list(&self) -> Reply {
        let mut out = format!("count={}", self.stack.len());
        for (i, e) in self.stack.iter().enumerate() {
            let marker = if i + 1 == self.stack.len() { " [owner]" } else { "" };
            out.push_str(&format!(
                "\n  pid={} app={} kind={}{marker}",
                e.pid, e.app_id, e.kind.as_wire(),
            ));
        }
        Reply::ok(out)
    }
}

impl ArbiterModule for AudioModule {
    fn verbs(&self) -> &[&'static str] {
        &[
            "audio-focus-request", "audio-focus-abandon", "audio-focus-list",
            "audio-call-start", "audio-call-end", "audio-route",
        ]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "audio-focus-request" => self.cmd_request(args, ctx),
            "audio-focus-abandon" => self.cmd_abandon(args, ctx),
            "audio-focus-list"    => self.cmd_list(),
            "audio-call-start"    => self.cmd_call_start(args, ctx),
            "audio-call-end"      => self.cmd_call_end(args, ctx),
            "audio-route"         => self.cmd_route(args, ctx),
            other => Reply::err(format!("audio-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        // A focus owner's process died — drop it and restore the next owner.
        if let Event::SurfaceRemoved { pid } = ev {
            // If the comms-session owner died, end the session: clear the mode
            // marker + the ongoing-call badge (no set-mode push — the host is
            // gone; the next host to set a mode will correct it).
            if self.comms == Some(*pid) {
                self.comms = None;
                if let Some(app) = ctx.store.app_by_pid(*pid).map(|a| a.app_id.clone()) {
                    ctx.store.cancel_notification(&app, CALL_NOTIF_ID);
                }
                log::info!("arbiter: audio-call owner pid={pid} died — session ended");
            }
            if self.drop_pid(*pid, ctx) {
                log::info!("arbiter: audio-focus dropped dead pid={pid}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime};
    use wart_arbiter_core::{AppState, Effect, Registry, Store};

    fn reg() -> (Registry, Store) {
        let mut r = Registry::new();
        r.register(Box::new(AudioModule::new()));
        (r, Store::new())
    }

    fn seed(s: &mut Store, id: &str, pid: i32) {
        s.insert_app(AppState {
            app_id: id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
    }

    /// Collect the `on-focus-changed <kind>` pushes to a given pid.
    fn changes_for(eff: &[Effect], pid: i32) -> Vec<String> {
        eff.iter().filter_map(|e| match e {
            Effect::HostLine { pid: p, line } if *p == pid =>
                Some(line.trim().to_string()),
            _ => None,
        }).collect()
    }

    #[test]
    fn transient_pauses_then_restores_owner() {
        let (mut r, mut store) = reg();
        // Music app takes permanent focus.
        let (reply, _) = r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        // A navigation prompt takes transient focus → music gets loss-transient.
        let (_r, eff) = r.dispatch_command("audio-focus-request", "222 gain-transient", &mut store).unwrap();
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed loss-transient"]);
        // Prompt abandons → music regains.
        let (_r, eff) = r.dispatch_command("audio-focus-abandon", "222", &mut store).unwrap();
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed gain"]);
    }

    #[test]
    fn permanent_gain_evicts_everyone() {
        let (mut r, mut store) = reg();
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        r.dispatch_command("audio-focus-request", "222 gain-transient", &mut store).unwrap();
        // A call grabs permanent focus → both prior owners get loss.
        let (_r, eff) = r.dispatch_command("audio-focus-request", "333 gain", &mut store).unwrap();
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed loss"]);
        assert_eq!(changes_for(&eff, 222), vec!["on-focus-changed loss"]);
        // Only the call remains; abandoning it restores nothing.
        let (_r, eff) = r.dispatch_command("audio-focus-abandon", "333", &mut store).unwrap();
        assert!(eff.is_empty());
    }

    #[test]
    fn duck_does_not_clear_stack() {
        let (mut r, mut store) = reg();
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        let (_r, eff) = r.dispatch_command("audio-focus-request", "222 duck", &mut store).unwrap();
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed duck"]);
        // Owner is 222 with 111 still under it.
        let (reply, _) = r.dispatch_command("audio-focus-list", "", &mut store).unwrap();
        let Reply::Ok(body) = reply else { panic!() };
        assert!(body.contains("count=2"));
    }

    #[test]
    fn app_id_resolves_to_pid() {
        let (mut r, mut store) = reg();
        seed(&mut store, "war.signal", 4321);
        let (reply, _) = r.dispatch_command("audio-focus-request", "war.signal gain", &mut store).unwrap();
        let Reply::Ok(body) = reply else { panic!() };
        assert!(body.contains("pid=4321"));
        assert!(body.contains("app=war.signal"));
    }

    #[test]
    fn dead_owner_is_dropped_and_next_restored() {
        let (mut r, mut store) = reg();
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        r.dispatch_command("audio-focus-request", "222 gain-transient", &mut store).unwrap();
        // Owner 222 dies → 111 regains.
        let eff = r.dispatch_event(Event::SurfaceRemoved { pid: 222 }, &mut store);
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed gain"]);
        // A non-owner death just prunes (no restore push).
        r.dispatch_command("audio-focus-request", "333 gain-transient", &mut store).unwrap();
        let eff = r.dispatch_event(Event::SurfaceRemoved { pid: 111 }, &mut store);
        assert!(eff.is_empty());
    }

    #[test]
    fn call_start_pauses_music_sets_mode_and_badges() {
        let (mut r, mut store) = reg();
        seed(&mut store, "war.signal", 500);
        // Music playing (permanent focus).
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        let (reply, eff) = r.dispatch_command("audio-call-start", "war.signal", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        // Music pauses (transient), not permanently evicted.
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed loss-transient"]);
        // The owner host is told to switch the global mode.
        assert!(changes_for(&eff, 500).iter().any(|l| l == "audio-policy set-mode comm"));
        // Ongoing-call badge raised.
        assert!(store.notifications().iter().any(|n| n.app_id == "war.signal"));
    }

    #[test]
    fn call_end_resumes_music_restores_mode_clears_badge() {
        let (mut r, mut store) = reg();
        seed(&mut store, "war.signal", 500);
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        r.dispatch_command("audio-call-start", "war.signal", &mut store).unwrap();
        let (_r, eff) = r.dispatch_command("audio-call-end", "war.signal", &mut store).unwrap();
        // Mode restored on the owner + music regains focus.
        assert!(changes_for(&eff, 500).iter().any(|l| l == "audio-policy set-mode normal"));
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed gain"]);
        // Badge cleared.
        assert!(store.notifications().iter().all(|n| n.app_id != "war.signal"));
    }

    #[test]
    fn route_targets_the_owner_host() {
        let (mut r, mut store) = reg();
        let (reply, eff) = r.dispatch_command("audio-route", "500 speaker", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        assert!(changes_for(&eff, 500).iter().any(|l| l == "audio-policy set-route speaker"));
    }

    #[test]
    fn comms_owner_death_ends_session() {
        let (mut r, mut store) = reg();
        seed(&mut store, "war.signal", 500);
        r.dispatch_command("audio-call-start", "war.signal", &mut store).unwrap();
        assert!(store.notifications().iter().any(|n| n.app_id == "war.signal"));
        // Owner dies → session ends, badge cleared.
        r.dispatch_event(Event::SurfaceRemoved { pid: 500 }, &mut store);
        assert!(store.notifications().iter().all(|n| n.app_id != "war.signal"));
    }

    #[test]
    fn rerequest_same_owner_no_self_loss() {
        let (mut r, mut store) = reg();
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        // Same pid re-requests (e.g. upgrade transient→gain): no loss to itself.
        let (_r, eff) = r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        assert!(changes_for(&eff, 111).is_empty());
        let (reply, _) = r.dispatch_command("audio-focus-list", "", &mut store).unwrap();
        let Reply::Ok(body) = reply else { panic!() };
        assert!(body.contains("count=1"));
    }
}
