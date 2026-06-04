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

use wart_arbiter_core::{ArbiterModule, Ctx, Effect, Event, Reply, SensorKind};

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
    /// M3b — pids in an active comms session (a call). They are NEVER dozed
    /// (`cadence 0`) so a call keeps running with the screen off. Driven by
    /// `Event::CommsActive` from the audio module; cleaned on `SurfaceRemoved`.
    comms: HashSet<i32>,
    /// Task 78 — whether we have forced the panel OFF for proximity (phone at the
    /// ear during a call). Tracked so we toggle on transitions only and, crucially,
    /// **always restore** the panel when the call ends / the sensor uncovers (a
    /// stuck-off panel would feel like a bricked device).
    blanked: bool,
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

    /// The cadence (ms) to send a pid while dozing, by its class. A comms-session
    /// pid is never dozed (cadence 0) — a live call must keep running off-screen.
    fn cadence_for(&self, pid: i32) -> u64 {
        if self.comms.contains(&pid) {
            0 // in a call — never doze
        } else if self.bg_service.contains(&pid) {
            DOZE_MAINTENANCE_MS
        } else {
            DOZE_SUSPEND_MS
        }
    }

    /// Apply the panel-blank state: SurfaceFlinger panel power AND touch
    /// suppression move together (task 79) so they can never drift — a blanked
    /// panel always has touch dropped, and restoring the panel always restores
    /// touch. The suppress flag is fanned to EVERY tracked host (a cheek at the
    /// ear can land on chrome too, and the screen is off so nothing needs touch).
    fn set_panel_blanked(&mut self, blank: bool, ctx: &mut Ctx) {
        ctx.request(Effect::SetDisplayPower { on: !blank });
        for app in ctx.store.apps_snapshot() {
            ctx.deliver_to_host(app.pid, format!("input-suppress {}\n", blank as u8));
        }
        self.blanked = blank;
    }

    /// Fail-safe: restore the panel + touch if proximity had blanked them.
    /// Idempotent — safe to call on every call-end / surface-removed. A stuck-off
    /// panel (or dead touch) would feel like a bricked device, so this is the
    /// invariant the policy guarantees.
    fn ensure_unblanked(&mut self, ctx: &mut Ctx) {
        if self.blanked {
            self.set_panel_blanked(false, ctx);
            log::info!("arbiter: proximity blank cleared → panel ON + touch resumed");
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
                self.comms.remove(pid);
                // Task 78 fail-safe: if the call host died while we'd blanked the
                // panel for proximity, restore it (no live call to uncover it).
                if self.comms.is_empty() {
                    self.ensure_unblanked(ctx);
                }
            }
            // M3b — a call started/ended: update the keep-alive set. If we're
            // already dozing, re-fan THIS pid's cadence now (a call starting
            // mid-doze must wake to 0; ending mid-doze re-dozes to its class).
            Event::CommsActive { pid, active } => {
                if *active { self.comms.insert(*pid); } else { self.comms.remove(pid); }
                // Task 78 fail-safe: a call ending must restore the panel if
                // proximity had blanked it (the user can't uncover it post-call).
                if !*active && self.comms.is_empty() {
                    self.ensure_unblanked(ctx);
                }
                // Task 77 — proximity is only worth powering during a call (the
                // screen-off-on-ear use case). Express that intent to the
                // SensorService via the consumer protocol (acquire on call start,
                // release on end); the sensors module ref-counts + drives the HAL.
                // Power never reads the sensor itself — it reacts to
                // `Event::ProximityChanged` below.
                let kind = SensorKind::Proximity;
                if *active {
                    ctx.emit(Event::SensorAcquire { kind, requester: *pid });
                } else {
                    ctx.emit(Event::SensorRelease { kind, requester: *pid });
                }
                if self.dozing {
                    let cadence = self.cadence_for(*pid);
                    ctx.deliver_to_host(*pid, format!("doze {cadence}\n"));
                    log::info!("arbiter: comms {} pid={pid} mid-doze → doze {cadence}",
                        if *active { "start" } else { "end" });
                }
            }
            // Task 78 — proximity screen-off during a call. Only blank while a
            // call is active (phone at the ear); never otherwise. Toggle on the
            // debounced transition and track `blanked` so the fail-safes below can
            // always restore the panel.
            Event::ProximityChanged { near } => {
                let in_call = !self.comms.is_empty();
                if in_call && *near && !self.blanked {
                    self.set_panel_blanked(true, ctx);
                    log::info!("arbiter: proximity near + in-call → panel OFF + touch suppressed");
                } else if !*near && self.blanked {
                    self.ensure_unblanked(ctx);
                } else if !in_call && self.blanked {
                    // Defensive: a call ended between the blank and this event.
                    self.ensure_unblanked(ctx);
                }
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

    fn doze_line(eff: &[Effect], pid: i32) -> Option<String> {
        eff.iter().find_map(|e| match e {
            Effect::HostLine { pid: p, line } if *p == pid => Some(line.trim().to_string()),
            _ => None,
        })
    }

    #[test]
    fn comms_pid_kept_out_of_doze() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        // A call is active on pid 10 (not yet dozing → no immediate fan).
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        // Screen off past grace → ENTER doze: the call host gets `doze 0`, not suspend.
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        assert_eq!(doze_line(&eff, 10).as_deref(), Some("doze 0"));
    }

    #[test]
    fn call_start_and_end_mid_doze_refan() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        // Already dozing (pid 10 = normal → suspend).
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        assert_eq!(doze_line(&eff, 10).as_deref(), Some(&format!("doze {DOZE_SUSPEND_MS}")[..]));
        // Call starts mid-doze → immediate wake to 0.
        let eff = r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        assert_eq!(doze_line(&eff, 10).as_deref(), Some("doze 0"));
        // Call ends mid-doze → re-doze to its class cadence.
        let eff = r.dispatch_event(Event::CommsActive { pid: 10, active: false }, &mut store);
        assert_eq!(doze_line(&eff, 10).as_deref(), Some(&format!("doze {DOZE_SUSPEND_MS}")[..]));
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

    /// Task 77 — the consumer protocol: a call start/end makes power acquire /
    /// release proximity (so the sensor is only on during calls). Asserted via a
    /// sink module that records the emitted intents (keeps power decoupled from
    /// the sensors crate).
    #[test]
    fn comms_acquires_and_releases_proximity() {
        use std::sync::{Arc, Mutex};
        use wart_arbiter_core::{Ctx, SensorKind};

        struct Sink {
            seen: Arc<Mutex<Vec<(bool, SensorKind, i32)>>>, // (acquire?, kind, requester)
        }
        impl ArbiterModule for Sink {
            fn verbs(&self) -> &[&'static str] {
                &[]
            }
            fn on_command(&mut self, _v: &str, _a: &str, _c: &mut Ctx) -> Reply {
                Reply::ok("")
            }
            fn on_event(&mut self, ev: &Event, _c: &mut Ctx) {
                match ev {
                    Event::SensorAcquire { kind, requester } => {
                        self.seen.lock().unwrap().push((true, *kind, *requester))
                    }
                    Event::SensorRelease { kind, requester } => {
                        self.seen.lock().unwrap().push((false, *kind, *requester))
                    }
                    _ => {}
                }
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        r.register(Box::new(Sink { seen: seen.clone() }));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);

        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        r.dispatch_event(Event::CommsActive { pid: 10, active: false }, &mut store);

        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (true, SensorKind::Proximity, 10),
                (false, SensorKind::Proximity, 10),
            ]
        );
    }

    fn display_power(eff: &[Effect]) -> Vec<bool> {
        eff.iter()
            .filter_map(|e| match e {
                Effect::SetDisplayPower { on } => Some(*on),
                _ => None,
            })
            .collect()
    }

    /// Task 78 — proximity blanks the panel only during a call; uncover restores it.
    #[test]
    fn proximity_blanks_only_during_call() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);

        // No call yet: a near reading must NOT blank.
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), Vec::<bool>::new(), "no blank without a call");

        // Call active → near blanks (off), far restores (on).
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), vec![false], "near in-call → OFF");
        // Repeat near = no-op (transition-only).
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), Vec::<bool>::new(), "repeat near = no toggle");
        // Far → restore.
        let eff = r.dispatch_event(Event::ProximityChanged { near: false }, &mut store);
        assert_eq!(display_power(&eff), vec![true], "far → ON");
    }

    /// Task 78 fail-safe — call ending while blanked restores the panel.
    #[test]
    fn call_end_while_blanked_restores_panel() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        r.dispatch_event(Event::ProximityChanged { near: true }, &mut store); // blanked
        // Hang up while still covered → panel must come back on.
        let eff = r.dispatch_event(Event::CommsActive { pid: 10, active: false }, &mut store);
        assert_eq!(display_power(&eff), vec![true], "call end while blanked → ON");
    }

    /// Task 78 fail-safe — call host dying while blanked restores the panel.
    #[test]
    fn surface_removed_while_blanked_restores_panel() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        r.dispatch_event(Event::ProximityChanged { near: true }, &mut store); // blanked
        let eff = r.dispatch_event(Event::SurfaceRemoved { pid: 10 }, &mut store);
        assert_eq!(display_power(&eff), vec![true], "call host death while blanked → ON");
    }

    fn suppress_lines(eff: &[Effect]) -> Vec<String> {
        eff.iter()
            .filter_map(|e| match e {
                Effect::HostLine { line, .. } if line.starts_with("input-suppress") => {
                    Some(line.trim().to_string())
                }
                _ => None,
            })
            .collect()
    }

    /// Task 79 — a proximity blank fans `input-suppress 1` to every tracked host
    /// alongside the panel-off, and unblank fans `input-suppress 0`.
    #[test]
    fn proximity_blank_fans_input_suppress() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        add_app(&mut store, "bar", 20);
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);

        // Near in-call → panel OFF + suppress fanned to BOTH hosts.
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), vec![false]);
        assert_eq!(
            suppress_lines(&eff),
            vec!["input-suppress 1".to_string(), "input-suppress 1".to_string()]
        );

        // Far → panel ON + suppress cleared on both.
        let eff = r.dispatch_event(Event::ProximityChanged { near: false }, &mut store);
        assert_eq!(display_power(&eff), vec![true]);
        assert_eq!(
            suppress_lines(&eff),
            vec!["input-suppress 0".to_string(), "input-suppress 0".to_string()]
        );
    }
}
