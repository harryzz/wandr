//! Timed-wake alarms (Arbiter Inc. 3c — AlarmManager/JobScheduler).
//!
//! A guest schedules an [`Alarm`] (via the `war:alarm/scheduler` host import → the
//! arbiter's `schedule-alarm` verb); the binary's timer thread emits
//! [`crate::Event::AlarmTick`] and the `wart-arbiter-alarm` module fires the due
//! ones. One delivery mechanism — the guest's `on-alarm(id)` export — serves both
//! wake paths: an alive app gets it immediately; a dead app is relaunched
//! (`Effect::Launch`) and delivered on the next tick once its control socket binds.
//!
//! Alarms are keyed by `(app_id, alarm_id)` and live in the [`Store`] so they
//! survive the guest's death/relaunch AND an arbiter restart (persisted with the
//! registry). `schedule` is idempotent on the key (re-arming replaces), so a
//! relaunched guest re-scheduling the same id is a no-op.

use crate::{LaunchKind, Store};

/// One scheduled wake.
#[derive(Clone, Debug, PartialEq)]
pub struct Alarm {
    pub app_id: String,
    pub alarm_id: u64,
    /// Next fire time, unix epoch milliseconds.
    pub next_fire_ms: u64,
    /// Repeat period in ms; `0` = one-shot (dropped after firing).
    pub repeat_ms: u64,
    /// How to relaunch the owner if it's dead when the alarm fires.
    pub wake_kind: LaunchKind,
    /// Set when the alarm fired while the owner was dead and we relaunched it;
    /// the next tick delivers `on-alarm` once its control socket is up.
    pub pending_deliver: bool,
}

impl Store {
    /// Schedule or replace an alarm (idempotent on `(app_id, alarm_id)`).
    pub fn upsert_alarm(&mut self, alarm: Alarm) {
        if let Some(slot) = self
            .alarms
            .iter_mut()
            .find(|a| a.app_id == alarm.app_id && a.alarm_id == alarm.alarm_id)
        {
            *slot = alarm;
        } else {
            self.alarms.push(alarm);
        }
    }

    /// Cancel an alarm. Returns whether one was removed.
    pub fn cancel_alarm(&mut self, app_id: &str, alarm_id: u64) -> bool {
        let before = self.alarms.len();
        self.alarms
            .retain(|a| !(a.app_id == app_id && a.alarm_id == alarm_id));
        self.alarms.len() != before
    }

    pub fn alarms(&self) -> &[Alarm] {
        &self.alarms
    }

    pub fn alarms_mut(&mut self) -> &mut Vec<Alarm> {
        &mut self.alarms
    }

    /// Whether any alarms exist (the binary's timer thread skips ticking when none).
    pub fn has_alarms(&self) -> bool {
        !self.alarms.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alarm(app: &str, id: u64, next: u64, repeat: u64) -> Alarm {
        Alarm {
            app_id: app.to_string(),
            alarm_id: id,
            next_fire_ms: next,
            repeat_ms: repeat,
            wake_kind: LaunchKind::Headless,
            pending_deliver: false,
        }
    }

    #[test]
    fn upsert_is_idempotent_on_key() {
        let mut s = Store::new();
        s.upsert_alarm(alarm("a", 1, 1000, 5000));
        s.upsert_alarm(alarm("a", 1, 2000, 5000)); // same key → replace
        s.upsert_alarm(alarm("a", 2, 3000, 0));
        s.upsert_alarm(alarm("b", 1, 4000, 0));
        assert_eq!(s.alarms().len(), 3);
        assert_eq!(s.alarms().iter().find(|a| a.app_id == "a" && a.alarm_id == 1).unwrap().next_fire_ms, 2000);
        assert!(s.cancel_alarm("a", 1));
        assert!(!s.cancel_alarm("a", 1));
        assert_eq!(s.alarms().len(), 2);
        assert!(s.has_alarms());
    }
}
