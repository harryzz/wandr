//! User-facing notifications (Signal bg-receipt M3 — generic notification
//! primitive). An app raises one via the `wandr:notify/notifier` host import → the
//! arbiter's `notify-post` verb; the arbiter keeps the active list (surfaced in
//! the status bar), and when the user taps one it foregrounds the owner and
//! delivers `on-notification-click(id)` to the app's `notify-handler` export.
//!
//! Keyed by `(app_id, app_notif_id)` (re-posting the same id replaces); a stable
//! arbiter-assigned `nid` is the global handle the status-bar feed + click use.
//! In-memory (not persisted — notifications are transient; an app re-posts on its
//! next activity, and a stale one across an arbiter restart is undesirable).

use crate::Store;

/// One active notification.
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    /// Global, arbiter-assigned stable handle (what the status-bar feed + click use).
    pub nid: u64,
    /// Owning app.
    pub app_id: String,
    /// The id the app posted with — delivered back via `on-notification-click`,
    /// and the key (with `app_id`) for `cancel`.
    pub app_notif_id: u64,
    pub title: String,
    pub body: String,
}

impl Store {
    /// Raise or replace a notification (idempotent on `(app_id, app_notif_id)`).
    /// Returns the global `nid`.
    pub fn post_notification(
        &mut self,
        app_id: &str,
        app_notif_id: u64,
        title: String,
        body: String,
    ) -> u64 {
        if let Some(n) = self
            .notifications
            .iter_mut()
            .find(|n| n.app_id == app_id && n.app_notif_id == app_notif_id)
        {
            n.title = title;
            n.body = body;
            return n.nid;
        }
        self.next_nid += 1;
        let nid = self.next_nid;
        self.notifications.push(Notification {
            nid,
            app_id: app_id.to_string(),
            app_notif_id,
            title,
            body,
        });
        nid
    }

    /// Clear a notification by the app's own id. Returns whether one was removed.
    pub fn cancel_notification(&mut self, app_id: &str, app_notif_id: u64) -> bool {
        let before = self.notifications.len();
        self.notifications
            .retain(|n| !(n.app_id == app_id && n.app_notif_id == app_notif_id));
        self.notifications.len() != before
    }

    /// Resolve + remove a notification by its global `nid` (on click/dismiss).
    pub fn take_notification(&mut self, nid: u64) -> Option<Notification> {
        let i = self.notifications.iter().position(|n| n.nid == nid)?;
        Some(self.notifications.remove(i))
    }

    /// Drop every notification owned by `app_id` (e.g. on app exit). Returns the count.
    pub fn clear_app_notifications(&mut self, app_id: &str) -> usize {
        let before = self.notifications.len();
        self.notifications.retain(|n| n.app_id != app_id);
        before - self.notifications.len()
    }

    pub fn notifications(&self) -> &[Notification] {
        &self.notifications
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_is_idempotent_and_nid_stable() {
        let mut s = Store::new();
        let a = s.post_notification("sig", 1, "Alice".into(), "hi".into());
        let b = s.post_notification("sig", 1, "Alice".into(), "hello".into()); // replace
        assert_eq!(a, b, "same (app,id) keeps its nid");
        assert_eq!(s.notifications().len(), 1);
        assert_eq!(s.notifications()[0].body, "hello");
        let c = s.post_notification("sig", 2, "Bob".into(), "yo".into());
        assert_ne!(a, c);
        assert_eq!(s.notifications().len(), 2);
    }

    #[test]
    fn take_by_nid_and_cancel_and_clear() {
        let mut s = Store::new();
        let n1 = s.post_notification("sig", 1, "t".into(), "b".into());
        s.post_notification("sig", 2, "t".into(), "b".into());
        s.post_notification("mail", 9, "t".into(), "b".into());
        let taken = s.take_notification(n1).unwrap();
        assert_eq!(taken.app_notif_id, 1);
        assert_eq!(s.notifications().len(), 2);
        assert!(s.cancel_notification("sig", 2));
        assert!(!s.cancel_notification("sig", 2));
        assert_eq!(s.clear_app_notifications("mail"), 1);
        assert!(s.notifications().is_empty());
    }
}
