//! wart-arbiter-alarm — the AlarmManager/JobScheduler module (Arbiter Inc. 3c).
//!
//! Owns the timed-wake policy. Guests schedule via the `war:alarm/scheduler` host
//! import, which the host forwards as the `schedule-alarm` verb; the binary's
//! timer thread emits [`Event::AlarmTick`] and this module fires the due alarms.
//! One delivery mechanism — the guest's `on-alarm(id)` export — serves both wake
//! paths:
//!   • owner alive → `Effect::HostLine{ "alarm-fired <id>" }` → host calls on-alarm.
//!   • owner dead  → `Effect::Launch{ wake_kind }`, mark the alarm pending; the
//!     next tick (owner now up, socket bound) delivers `alarm-fired`.
//! Repeats reschedule to `now + repeat_ms` (no burst catch-up); one-shots drop.

use wart_arbiter_core::{Alarm, ArbiterModule, Ctx, Effect, Event, LaunchKind, Reply};

#[derive(Default)]
pub struct AlarmModule;

impl AlarmModule {
    pub fn new() -> Self {
        AlarmModule
    }

    /// `schedule-alarm <app-id> <id> <when-unix-ms> <repeat-ms> [wake-kind]` —
    /// schedule or replace (idempotent on `(app-id, id)`). `when` is an ABSOLUTE
    /// unix-ms fire time: the host computes `now + delay` from its own (same
    /// device) wall clock, so the arbiter stays clockless except for the tick.
    fn cmd_schedule(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() < 4 {
            return Reply::err("schedule-alarm-args: expected <app-id> <id> <when-unix-ms> <repeat-ms> [kind]");
        }
        let (Ok(alarm_id), Ok(when_ms), Ok(repeat_ms)) =
            (t[1].parse::<u64>(), t[2].parse::<u64>(), t[3].parse::<u64>())
        else {
            return Reply::err(format!("schedule-alarm-bad-args {args:?}"));
        };
        let wake_kind = t.get(4).map(|s| LaunchKind::from_wire(s)).unwrap_or(LaunchKind::Headless);
        ctx.store.upsert_alarm(Alarm {
            app_id: t[0].to_string(),
            alarm_id,
            next_fire_ms: when_ms,
            repeat_ms,
            wake_kind,
            pending_deliver: false,
        });
        log::info!(
            "arbiter: schedule-alarm app={} id={alarm_id} when={when_ms} repeat={repeat_ms}ms kind={}",
            t[0], wake_kind.as_wire()
        );
        Reply::ok(format!("alarm app={} id={alarm_id} when={when_ms} repeat={repeat_ms}ms", t[0]))
    }

    fn cmd_cancel(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("cancel-alarm-args: expected <app-id> <id>");
        }
        let Ok(alarm_id) = t[1].parse::<u64>() else {
            return Reply::err(format!("cancel-alarm-bad-id {:?}", t[1]));
        };
        let removed = ctx.store.cancel_alarm(t[0], alarm_id);
        log::info!("arbiter: cancel-alarm app={} id={alarm_id} removed={removed}", t[0]);
        Reply::ok(format!("cancel app={} id={alarm_id} removed={removed}", t[0]))
    }

    /// Fire every due alarm (`next_fire_ms <= now`).
    fn on_tick(&mut self, now_ms: u64, ctx: &mut Ctx) {
        // Pass 1 (immutable): decide per-due-alarm. Reading alarms() + app() are
        // both immutable borrows — fine together.
        let mut delivers: Vec<(i32, u64)> = Vec::new(); // (pid, alarm_id)
        let mut launches: Vec<(String, LaunchKind)> = Vec::new();
        let mut reschedule: Vec<(String, u64)> = Vec::new(); // (app_id, alarm_id) → now+repeat
        let mut remove: Vec<(String, u64)> = Vec::new();
        let mut set_pending: Vec<(String, u64)> = Vec::new();
        for a in ctx.store.alarms() {
            if a.next_fire_ms > now_ms {
                continue;
            }
            match ctx.store.app(&a.app_id) {
                Some(app) => {
                    // Owner alive → deliver now; reschedule or drop; clear pending.
                    delivers.push((app.pid, a.alarm_id));
                    if a.repeat_ms > 0 {
                        reschedule.push((a.app_id.clone(), a.alarm_id));
                    } else {
                        remove.push((a.app_id.clone(), a.alarm_id));
                    }
                }
                None => {
                    // Owner dead → relaunch once (guard on pending) + keep due so a
                    // later tick (owner up) delivers.
                    if !a.pending_deliver {
                        launches.push((a.app_id.clone(), a.wake_kind));
                    }
                    set_pending.push((a.app_id.clone(), a.alarm_id));
                }
            }
        }

        // Pass 2 (mutable): apply the state transitions.
        let alarms = ctx.store.alarms_mut();
        for (app_id, id) in &reschedule {
            if let Some(a) = alarms.iter_mut().find(|a| a.app_id == *app_id && a.alarm_id == *id) {
                a.next_fire_ms = now_ms + a.repeat_ms;
                a.pending_deliver = false;
            }
        }
        for (app_id, id) in &set_pending {
            if let Some(a) = alarms.iter_mut().find(|a| a.app_id == *app_id && a.alarm_id == *id) {
                a.pending_deliver = true;
            }
        }
        alarms.retain(|a| !remove.iter().any(|(app_id, id)| a.app_id == *app_id && a.alarm_id == *id));

        // Pass 3: emit effects (relaunch dead owners, deliver to alive ones).
        for (app_id, kind) in launches {
            log::info!("arbiter: alarm waking dead app={app_id} via relaunch kind={}", kind.as_wire());
            ctx.request(Effect::Launch { app_id, kind });
        }
        for (pid, id) in delivers {
            ctx.deliver_to_host(pid, format!("alarm-fired {id}\n"));
        }
    }
}

impl ArbiterModule for AlarmModule {
    fn verbs(&self) -> &[&'static str] {
        &["schedule-alarm", "cancel-alarm"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "schedule-alarm" => self.cmd_schedule(args, ctx),
            "cancel-alarm" => self.cmd_cancel(args, ctx),
            other => Reply::err(format!("alarm-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        if let Event::AlarmTick { now_ms } = ev {
            self.on_tick(*now_ms, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wart_arbiter_core::{AppState, Registry, Store};
    use std::time::{Instant, SystemTime};

    fn reg() -> (Registry, Store) {
        let mut r = Registry::new();
        r.register(Box::new(AlarmModule::new()));
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
    fn host_line(eff: &[Effect], pid: i32) -> Option<String> {
        eff.iter().find_map(|e| match e {
            Effect::HostLine { pid: p, line } if *p == pid => Some(line.clone()),
            _ => None,
        })
    }

    #[test]
    fn schedule_then_fire_alive_delivers() {
        let (mut r, mut store) = reg();
        add_app(&mut store, "a", 10);
        // fire at when=1500 (absolute), repeat=5000
        r.dispatch_command("schedule-alarm", "a 7 1500 5000", &mut store).unwrap();
        // tick before due → nothing
        let e = r.dispatch_event(Event::AlarmTick { now_ms: 1400 }, &mut store);
        assert!(host_line(&e, 10).is_none());
        // tick at due (1500) → deliver alarm-fired 7 to pid 10
        let e = r.dispatch_event(Event::AlarmTick { now_ms: 1500 }, &mut store);
        assert_eq!(host_line(&e, 10).as_deref(), Some("alarm-fired 7\n"));
        // rescheduled to 1500+5000
        assert_eq!(store.alarms()[0].next_fire_ms, 6500);
    }

    #[test]
    fn fire_dead_relaunches_then_delivers_next_tick() {
        let (mut r, mut store) = reg();
        // app "b" not running; one-shot at when=100
        r.dispatch_command("schedule-alarm", "b 1 100 0", &mut store).unwrap();
        // due tick → no app → Effect::Launch + pending
        let e = r.dispatch_event(Event::AlarmTick { now_ms: 200 }, &mut store);
        assert!(e.iter().any(|x| matches!(x, Effect::Launch { app_id, kind } if app_id == "b" && *kind == LaunchKind::Headless)));
        assert!(store.alarms()[0].pending_deliver);
        // owner comes up; next tick delivers + drops the one-shot
        add_app(&mut store, "b", 20);
        let e = r.dispatch_event(Event::AlarmTick { now_ms: 300 }, &mut store);
        assert_eq!(host_line(&e, 20).as_deref(), Some("alarm-fired 1\n"));
        assert!(store.alarms().is_empty(), "one-shot dropped after delivery");
    }

    #[test]
    fn cancel_removes() {
        let (mut r, mut store) = reg();
        r.dispatch_command("schedule-alarm", "a 5 100 100", &mut store).unwrap();
        assert!(store.has_alarms());
        r.dispatch_command("cancel-alarm", "a 5", &mut store).unwrap();
        assert!(!store.has_alarms());
    }
}
