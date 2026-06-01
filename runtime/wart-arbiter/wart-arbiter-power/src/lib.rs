//! wart-arbiter-power — the PowerManager module (doze policy).
//!
//! Owns the doze POLICY so the arbiter decides and the host applies (the
//! project's central rule, which the v0 host-local doze diverged from). It
//! consumes [`Event::ScreenState`] (the binary's screen poller — On/Vr = live),
//! applies a screen-off **grace**, and on a doze transition fans a
//! `doze <cadence-ms>` line to every tracked host. Each host then slows its own
//! render/bg-tick loop to that cadence (the mechanism), exactly like it applies
//! arbiter-decided geometry/orientation/roles. `cadence=0` = "not dozing".
//!
//! ## Class-based policy
//! The cadence is **per-app**, keyed on the app's power class — the arbiter is
//! the single authority, so it can treat apps differently:
//!   * **background-service** (e.g. Signal): a lenient *maintenance* cadence so it
//!     keeps receiving in a timely way while the screen is off (a Doze exemption).
//!   * everyone else (normal / chrome): a longer *suspend* cadence — nothing to
//!     show off-screen, so back off harder for battery.
//! The class is reported up by the loader (`report-power-class <pid> <class>` —
//! the host parses the manifest `background` flag; "host reads/reports, arbiter
//! owns/decides", like `report-panel`). Unreported pids default to normal.
//!
//! Future home for user-set per-app profiles (restricted/optimized/unrestricted),
//! wakelocks (suppress doze), and maintenance-window alarm batching.

use std::collections::HashSet;
use std::time::Instant;

use wart_arbiter_core::{ArbiterModule, Ctx, Event, Reply};

/// Screen-off grace before dozing (ms) — the "when to doze" policy.
const DOZE_GRACE_MS: u128 = 60_000;
/// Cadence for a background-service while dozing (ms): keeps receiving.
const DOZE_MAINTENANCE_MS: u64 = 10_000;
/// Cadence for everyone else while dozing (ms): back off harder (off-screen).
const DOZE_SUSPEND_MS: u64 = 60_000;

#[derive(Default)]
pub struct PowerModule {
    /// When the screen last went non-live (`None` = live). Transient module state.
    screen_off_at: Option<Instant>,
    dozing: bool,
    /// Pids the loader reported as background-services (get the maintenance
    /// cadence). Everyone else is normal. Cleaned on `SurfaceRemoved`.
    bg_service: HashSet<i32>,
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

    /// The cadence (ms) to send a pid while dozing, by its class.
    fn cadence_for(&self, pid: i32) -> u64 {
        if self.bg_service.contains(&pid) {
            DOZE_MAINTENANCE_MS
        } else {
            DOZE_SUSPEND_MS
        }
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
        // Fan the per-app cadence to every tracked host (dumb appliers). On EXIT
        // everyone gets `doze 0`; on ENTER each gets its class's cadence. A dead
        // pid's socket just fails silently in the executor.
        for app in ctx.store.apps_snapshot() {
            let cadence = if new_dozing { self.cadence_for(app.pid) } else { 0 };
            ctx.deliver_to_host(app.pid, format!("doze {cadence}\n"));
        }
        log::info!(
            "arbiter: doze {} — fanned per-class to hosts (maintenance={DOZE_MAINTENANCE_MS}ms / suspend={DOZE_SUSPEND_MS}ms, {} bg-services)",
            if new_dozing { "ENTER" } else { "EXIT" },
            self.bg_service.len()
        );
    }

    /// `report-power-class <pid> <bg-service|normal>` — the loader reports an
    /// app's power class at startup (host reads the manifest; arbiter owns it).
    fn cmd_report_class(&mut self, args: &str, _ctx: &mut Ctx) -> Reply {
        let mut t = args.split_whitespace();
        let (Some(pid_s), Some(class)) = (t.next(), t.next()) else {
            return Reply::err("report-power-class-args: expected <pid> <bg-service|normal>");
        };
        let Ok(pid) = pid_s.parse::<i32>() else {
            return Reply::err(format!("report-power-class-bad-pid {pid_s}"));
        };
        match class {
            "bg-service" => {
                self.bg_service.insert(pid);
            }
            _ => {
                self.bg_service.remove(&pid);
            }
        }
        log::info!("arbiter: power-class pid={pid} class={class}");
        Reply::ok(format!("power-class pid={pid} class={class}"))
    }
}

impl ArbiterModule for PowerModule {
    fn verbs(&self) -> &[&'static str] {
        &["report-power-class"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "report-power-class" => self.cmd_report_class(args, ctx),
            other => Reply::err(format!("power-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        match ev {
            Event::ScreenState { live } => self.on_screen_state(*live, ctx),
            Event::SurfaceRemoved { pid } => {
                self.bg_service.remove(pid);
            }
            _ => {}
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
        assert_eq!(decide(None, false), None);
        assert_eq!(decide(Some(59_000), false), None);
        assert_eq!(decide(Some(60_000), false), Some(true));
        assert_eq!(decide(Some(120_000), true), None);
        assert_eq!(decide(None, true), Some(false));
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
    fn doze_fans_per_class_cadence() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        add_app(&mut store, "game", 20);

        // sig reports bg-service; game stays normal.
        r.dispatch_command("report-power-class", "10 bg-service", &mut store).unwrap();

        // Screen off past grace → ENTER: sig gets maintenance (10s), game suspend (60s).
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        let cad = |pid: i32| {
            eff.iter().find_map(|e| match e {
                Effect::HostLine { pid: p, line } if *p == pid => Some(line.clone()),
                _ => None,
            })
        };
        assert_eq!(cad(10).as_deref(), Some("doze 10000\n")); // bg-service → maintenance
        assert_eq!(cad(20).as_deref(), Some("doze 60000\n")); // normal → suspend

        // Screen on → EXIT: both get doze 0.
        let eff = r.dispatch_event(Event::ScreenState { live: true }, &mut store);
        assert_eq!(
            eff.iter()
                .filter(|e| matches!(e, Effect::HostLine { line, .. } if line == "doze 0\n"))
                .count(),
            2
        );
    }

    #[test]
    fn surface_removed_forgets_class() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        r.dispatch_command("report-power-class", "10 bg-service", &mut store).unwrap();
        // App dies → its class is forgotten (a recycled pid mustn't inherit it).
        r.dispatch_event(Event::SurfaceRemoved { pid: 10 }, &mut store);
        // ENTER doze: pid 10 now defaults to normal (suspend), not maintenance.
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        let line = eff.iter().find_map(|e| match e {
            Effect::HostLine { pid: 10, line } => Some(line.clone()),
            _ => None,
        });
        assert_eq!(line.as_deref(), Some("doze 60000\n"));
    }
}
