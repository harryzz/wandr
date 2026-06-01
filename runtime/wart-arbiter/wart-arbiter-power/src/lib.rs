//! wart-arbiter-power — the PowerManager module (doze policy).
//!
//! Owns the doze POLICY so the arbiter decides and the host applies (the
//! project's central rule, which the v0 host-local doze diverged from). It
//! consumes [`Event::ScreenState`] (the binary's screen poller — On/Vr = live),
//! applies a screen-off **grace**, and on a doze transition fans a
//! `doze <cadence-ms>` line to every tracked host. Each host then slows its own
//! render/bg-tick loop to that cadence (the mechanism), exactly like it applies
//! arbiter-decided geometry/orientation/roles. `cadence=0` means "not dozing,
//! resume normal pacing".
//!
//! This is the home for future power policy that genuinely needs the arbiter as
//! the single authority: wakelocks (an app asks to stay awake → suppress doze),
//! idle stages, and batching alarms into maintenance windows.

use std::time::Instant;

use wart_arbiter_core::{ArbiterModule, Ctx, Event, Reply};

/// Screen-off grace before dozing (ms). Catches a message arriving right after
/// the screen sleeps, before backing off — the "when to doze" policy.
const DOZE_GRACE_MS: u128 = 60_000;
/// Coarse cadence (ms) hosts slow their loop to while dozing. Pushed to hosts,
/// which apply it; `0` = not dozing.
const DOZE_CADENCE_MS: u64 = 10_000;

#[derive(Default)]
pub struct PowerModule {
    /// When the screen last went non-live (`None` = live). Transient working
    /// state owned by the module (not persisted, not shared).
    screen_off_at: Option<Instant>,
    dozing: bool,
}

/// Pure doze decision: given how long the screen has been off (`off_ms`, `None`
/// = live) and the current dozing flag, return `Some(new)` on a transition else
/// `None`. Split out so the grace boundary is unit-testable without real time.
fn decide(off_ms: Option<u128>, dozing: bool) -> Option<bool> {
    let dozing_now = off_ms.map(|ms| ms >= DOZE_GRACE_MS).unwrap_or(false);
    (dozing_now != dozing).then_some(dozing_now)
}

impl PowerModule {
    pub fn new() -> Self {
        Self::default()
    }

    fn on_screen_state(&mut self, live: bool, ctx: &mut Ctx) {
        if live {
            self.screen_off_at = None;
        } else if self.screen_off_at.is_none() {
            self.screen_off_at = Some(Instant::now());
        }
        let off_ms = self.screen_off_at.map(|t| t.elapsed().as_millis());
        let Some(new_dozing) = decide(off_ms, self.dozing) else {
            return;
        };
        self.dozing = new_dozing;
        let cadence = if new_dozing { DOZE_CADENCE_MS } else { 0 };
        // Fan the doze cadence to every tracked host — they apply it (dumb
        // appliers). A dead pid's socket just fails silently in the executor.
        for app in ctx.store.apps_snapshot() {
            ctx.deliver_to_host(app.pid, format!("doze {cadence}\n"));
        }
        log::info!(
            "arbiter: doze {} (cadence={cadence}ms) — fanned to hosts",
            if new_dozing { "ENTER" } else { "EXIT" }
        );
    }
}

impl ArbiterModule for PowerModule {
    fn verbs(&self) -> &[&'static str] {
        &[] // event-only module — no command verbs
    }

    fn on_command(&mut self, verb: &str, _args: &str, _ctx: &mut Ctx) -> Reply {
        Reply::err(format!("power-unknown-verb {verb}"))
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        if let Event::ScreenState { live } = ev {
            self.on_screen_state(*live, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime};
    use wart_arbiter_core::{AppState, Effect, Registry, Store};

    #[test]
    fn grace_boundary_decision() {
        assert_eq!(decide(None, false), None); // live, not dozing → no change
        assert_eq!(decide(Some(0), false), None); // just off → within grace
        assert_eq!(decide(Some(59_000), false), None); // still in grace
        assert_eq!(decide(Some(60_000), false), Some(true)); // grace elapsed → ENTER
        assert_eq!(decide(Some(120_000), true), None); // already dozing → no change
        assert_eq!(decide(None, true), Some(false)); // live again → EXIT
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
    fn screen_off_past_grace_fans_doze_to_all_hosts() {
        let mut r = Registry::new();
        // Pre-age the module's screen-off clock past the grace.
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "a", 10);
        add_app(&mut store, "b", 20);

        // A non-live tick now → grace already elapsed → ENTER, fanned to both pids.
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        let mut doze_pids: Vec<i32> = eff
            .iter()
            .filter_map(|e| match e {
                Effect::HostLine { pid, line } if line == "doze 10000\n" => Some(*pid),
                _ => None,
            })
            .collect();
        doze_pids.sort();
        assert_eq!(doze_pids, vec![10, 20]);

        // A live tick → EXIT (doze 0) to both.
        let eff = r.dispatch_event(Event::ScreenState { live: true }, &mut store);
        assert_eq!(
            eff.iter()
                .filter(|e| matches!(e, Effect::HostLine { line, .. } if line == "doze 0\n"))
                .count(),
            2
        );
    }
}
