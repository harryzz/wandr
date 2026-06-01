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

use wart_arbiter_core::{ArbiterModule, Ctx, Event, Reply};

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

/// The audio-focus arbiter. `stack.last()` is the current owner.
#[derive(Default)]
pub struct AudioModule {
    stack: Vec<FocusEntry>,
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
        Reply::ok(format!("granted pid={pid} app={app_id} kind={}", kind.as_wire()))
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
        &["audio-focus-request", "audio-focus-abandon", "audio-focus-list"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "audio-focus-request" => self.cmd_request(args, ctx),
            "audio-focus-abandon" => self.cmd_abandon(args, ctx),
            "audio-focus-list"    => self.cmd_list(),
            other => Reply::err(format!("audio-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        // A focus owner's process died — drop it and restore the next owner.
        if let Event::SurfaceRemoved { pid } = ev {
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
