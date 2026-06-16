//! wandr-arbiter-keyguard — the Keyguard/lockscreen module (v1: swipe-to-unlock).
//!
//! Owns the lock state + policy. Auto-locks on screen-off (reacts to
//! [`Event::ScreenState`] `{ live: false }`, the same signal the power module
//! consumes) and exposes `lock` / `unlock` verbs (boot locks via `lock`; the
//! `unlock` verb is driven by the keyguard guest's swipe gesture → host → arbiter).
//!
//! Locking **demotes the visible app to `Background`** (so it stops fighting for
//! focus via the host's `focus_refresh` gate, and goes lifecycle-Paused) and shows
//! the keyguard surface as [`Role::Lockscreen`] (the binary maps that to the
//! foreground mechanism — show + focus + present — at the keyguard's high launch
//! layer). Unlocking hides the keyguard and restores the covered app to the
//! foreground via [`Effect::Foreground`] (the proper shell promote). The keyguard
//! is excluded from `visible_app`/the task-cycle ring (it's not a switchable app).

use wandr_arbiter_core::{ArbiterModule, Ctx, Effect, Event, Reply, Role, PRIMARY_DISPLAY};

/// The keyguard guest's app-id (launched as a `--standalone-overlay-lock` overlay).
const KEYGUARD_APP_ID: &str = "wandr.keyguard";
/// The power-menu overlay guest (task 110) — same modal-overlay machinery.
const POWERMENU_APP_ID: &str = "wandr.powermenu";

#[derive(Default)]
pub struct KeyguardModule {
    locked: bool,
    /// The app-id covered by the keyguard (to restore on unlock). `None` if
    /// nothing was foreground at lock time.
    saved_fg: Option<String>,
    /// Task 80 Step 2 — the covered app's pid, suppressed for input while locked
    /// so the keyguard is modal (a fullscreen lock overlay shares the content
    /// region with the app behind it; rect-filtering alone wouldn't exclude it).
    /// Re-enabled on unlock.
    suppressed_pid: Option<i32>,
    // ── Power menu (task 110) — a second modal overlay, same mechanism. ──
    menu_shown: bool,
    menu_saved_fg: Option<String>,
    menu_suppressed_pid: Option<i32>,
    /// True when the menu was opened *over the lock screen*: the keyguard was
    /// demoted out of the input window list (so the power menu — not the
    /// keyguard's swipe area — receives touch) and must be restored on dismiss.
    menu_covered_keyguard: bool,
}

impl KeyguardModule {
    pub fn new() -> Self {
        Self::default()
    }

    fn keyguard_pid(ctx: &Ctx) -> Option<i32> {
        ctx.store.app(KEYGUARD_APP_ID).map(|a| a.pid)
    }

    fn do_lock(&mut self, ctx: &mut Ctx) -> Reply {
        if self.locked {
            return Reply::ok("already-locked");
        }
        let Some(kg) = Self::keyguard_pid(ctx) else {
            return Reply::err("keyguard-not-running (launch wandr.keyguard first)");
        };
        // Record + demote the visible app so it stops fighting for focus (Paused).
        let vis = ctx.store.display(PRIMARY_DISPLAY).and_then(|d| d.visible_app());
        self.saved_fg = vis.and_then(|pid| ctx.store.app_by_pid(pid).map(|a| a.app_id.clone()));
        if let Some(pid) = vis {
            ctx.request(Effect::SetRole { pid, role: Role::Background });
            ctx.store.display_mut(PRIMARY_DISPLAY).set_role(pid, Role::Background);
            // Task 80 Step 2 — suppress the covered app's touch so the keyguard is
            // modal (it overlaps the app's content region; rect-filtering can't
            // exclude a fullscreen overlay). Re-enabled on unlock.
            ctx.deliver_to_host(pid, "input-suppress 1\n");
            self.suppressed_pid = Some(pid);
        }
        // Show the keyguard topmost + focused (the binary maps Lockscreen → the
        // foreground mechanism). put_surface create-or-updates it.
        ctx.request(Effect::SetRole { pid: kg, role: Role::Lockscreen });
        ctx.store
            .display_mut(PRIMARY_DISPLAY)
            .put_surface(kg, KEYGUARD_APP_ID, Role::Lockscreen);
        self.locked = true;
        // The focused app was just demoted under the lock screen. Announce the
        // foreground change to the keyguard so the shell `finishInput()`s the old
        // editor + hides any soft keyboard that was up — otherwise it floats on top
        // of the lock screen (the keyguard has no text field). Mirrors what
        // `promote_to_foreground` emits on a normal app switch.
        ctx.emit(Event::ForegroundChanged {
            app_id: Some(KEYGUARD_APP_ID.to_string()),
            pid: Some(kg),
        });
        // Re-push system orientation so the lock screen + chrome snap PORTRAIT (the
        // lock screen never rotates — like an orientation-locked launcher). wm's
        // OrientationChanged handler re-fans; effective_orient now returns 0 because
        // a Role::Lockscreen surface is present. (orient value is unchanged — this
        // just forces the re-fan; the lock, not this value, decides portrait.)
        let orient = ctx.store.geometry(PRIMARY_DISPLAY).map(|g| g.orientation).unwrap_or(0);
        ctx.emit(Event::OrientationChanged { id: PRIMARY_DISPLAY, orient });
        log::info!("arbiter: keyguard LOCKED (covering {:?}, kg pid={kg})", self.saved_fg);
        Reply::ok(format!("locked covering={:?}", self.saved_fg))
    }

    fn powermenu_pid(ctx: &Ctx) -> Option<i32> {
        ctx.store.app(POWERMENU_APP_ID).map(|a| a.pid)
    }

    /// Show the power menu as a modal overlay (same mechanism as locking: demote
    /// + input-suppress the visible app, show the overlay topmost+focused).
    fn do_show_menu(&mut self, ctx: &mut Ctx) -> Reply {
        if self.menu_shown {
            return Reply::ok("power-menu-already-shown");
        }
        let Some(pm) = Self::powermenu_pid(ctx) else {
            return Reply::err("powermenu-not-running (launch wandr.powermenu first)");
        };
        let vis = ctx.store.display(PRIMARY_DISPLAY).and_then(|d| d.visible_app());
        self.menu_saved_fg = vis.and_then(|pid| ctx.store.app_by_pid(pid).map(|a| a.app_id.clone()));
        if let Some(pid) = vis {
            ctx.request(Effect::SetRole { pid, role: Role::Background });
            ctx.store.display_mut(PRIMARY_DISPLAY).set_role(pid, Role::Background);
            ctx.deliver_to_host(pid, "input-suppress 1\n");
            self.menu_suppressed_pid = Some(pid);
        }
        // Opened over the lock screen: the keyguard is also a fullscreen
        // `Role::Lockscreen` surface, so at equal input-z the dispatcher would
        // route touch to it (the swipe-to-unlock area) instead of the menu.
        // Demote it to `Background` so it drops out of the input window list and
        // the power menu becomes the sole focusable `Lockscreen` surface. Restored
        // on dismiss. (`visible_app()` excludes the keyguard, so the block above
        // never covers this — it must be handled explicitly.)
        if self.locked {
            if let Some(kg) = Self::keyguard_pid(ctx) {
                ctx.request(Effect::SetRole { pid: kg, role: Role::Background });
                ctx.store.display_mut(PRIMARY_DISPLAY).set_role(kg, Role::Background);
                self.menu_covered_keyguard = true;
            }
        }
        ctx.request(Effect::SetRole { pid: pm, role: Role::Lockscreen });
        ctx.store
            .display_mut(PRIMARY_DISPLAY)
            .put_surface(pm, POWERMENU_APP_ID, Role::Lockscreen);
        self.menu_shown = true;
        ctx.emit(Event::ForegroundChanged {
            app_id: Some(POWERMENU_APP_ID.to_string()),
            pid: Some(pm),
        });
        let orient = ctx.store.geometry(PRIMARY_DISPLAY).map(|g| g.orientation).unwrap_or(0);
        ctx.emit(Event::OrientationChanged { id: PRIMARY_DISPLAY, orient });
        log::info!("arbiter: power-menu SHOWN (covering {:?}, pm pid={pm})", self.menu_saved_fg);
        Reply::ok("power-menu shown")
    }

    /// Hide the power menu + restore the covered app (mirrors unlock). Always
    /// backgrounds the powermenu surface — this also covers the boot case where
    /// `pm-dismiss` hides the just-registered (visible Chrome) overlay.
    fn do_hide_menu(&mut self, ctx: &mut Ctx) -> Reply {
        if let Some(pm) = Self::powermenu_pid(ctx) {
            ctx.request(Effect::SetRole { pid: pm, role: Role::Background });
            ctx.store.display_mut(PRIMARY_DISPLAY).set_role(pm, Role::Background);
        }
        if self.menu_shown {
            if let Some(pid) = self.menu_suppressed_pid.take() {
                ctx.deliver_to_host(pid, "input-suppress 0\n");
            }
            // Restore the lock screen we demoted on show (back to the input list +
            // focus). Done before re-foregrounding any covered app so the keyguard
            // stays the modal top surface while locked.
            if self.menu_covered_keyguard {
                if let Some(kg) = Self::keyguard_pid(ctx) {
                    ctx.request(Effect::SetRole { pid: kg, role: Role::Lockscreen });
                    ctx.store
                        .display_mut(PRIMARY_DISPLAY)
                        .put_surface(kg, KEYGUARD_APP_ID, Role::Lockscreen);
                }
                self.menu_covered_keyguard = false;
            }
            if let Some(app_id) = self.menu_saved_fg.take() {
                ctx.request(Effect::Foreground { app_id });
            }
            self.menu_shown = false;
            let orient = ctx.store.geometry(PRIMARY_DISPLAY).map(|g| g.orientation).unwrap_or(0);
            ctx.emit(Event::OrientationChanged { id: PRIMARY_DISPLAY, orient });
            log::info!("arbiter: power-menu HIDDEN");
        }
        Reply::ok("power-menu hidden")
    }

    fn do_unlock(&mut self, ctx: &mut Ctx) -> Reply {
        if !self.locked {
            return Reply::ok("already-unlocked");
        }
        if let Some(kg) = Self::keyguard_pid(ctx) {
            ctx.request(Effect::SetRole { pid: kg, role: Role::Background });
            ctx.store.display_mut(PRIMARY_DISPLAY).set_role(kg, Role::Background);
        }
        // Task 80 Step 2 — re-enable the covered app's touch (it was suppressed for
        // modality while locked).
        if let Some(pid) = self.suppressed_pid.take() {
            ctx.deliver_to_host(pid, "input-suppress 0\n");
        }
        // Restore the covered app to foreground (proper shell promote: demotes
        // others, reconciles overlay, emits ForegroundChanged). No-op if it died.
        if let Some(app_id) = self.saved_fg.take() {
            ctx.request(Effect::Foreground { app_id });
        }
        self.locked = false;
        // Re-push system orientation now the lockscreen surface is gone, so chrome +
        // the restored app return to the decided device orient (effective_orient no
        // longer forces portrait).
        let orient = ctx.store.geometry(PRIMARY_DISPLAY).map(|g| g.orientation).unwrap_or(0);
        ctx.emit(Event::OrientationChanged { id: PRIMARY_DISPLAY, orient });
        log::info!("arbiter: keyguard UNLOCKED");
        Reply::ok("unlocked")
    }
}

impl ArbiterModule for KeyguardModule {
    fn verbs(&self) -> &[&'static str] {
        &["lock", "unlock", "power-menu", "pm-dismiss"]
    }

    fn on_command(&mut self, verb: &str, _args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "lock" => self.do_lock(ctx),
            "unlock" => self.do_unlock(ctx),
            "power-menu" => self.do_show_menu(ctx),
            "pm-dismiss" => self.do_hide_menu(ctx),
            other => Reply::err(format!("keyguard-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        match ev {
            // Auto-lock the moment the screen turns off.
            Event::ScreenState { live: false } => {
                self.do_lock(ctx);
            }
            // The covered app exited while locked → unlock reveals home, not it.
            Event::SurfaceRemoved { pid } => {
                let gone = ctx.store.app_by_pid(*pid).map(|a| a.app_id.clone());
                if gone.as_ref() == self.saved_fg.as_ref() {
                    self.saved_fg = None;
                }
                if gone.as_ref() == self.menu_saved_fg.as_ref() {
                    self.menu_saved_fg = None;
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
    use wandr_arbiter_core::{AppState, Registry, Store};

    fn seed(s: &mut Store, id: &str, pid: i32, role: Role) {
        s.insert_app(AppState {
            app_id: id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
        s.display_mut(PRIMARY_DISPLAY).put_surface(pid, id, role);
    }

    fn reg() -> (Registry, Store) {
        let mut r = Registry::new();
        r.register(Box::new(KeyguardModule::new()));
        (r, Store::new())
    }

    #[test]
    fn lock_demotes_app_and_shows_keyguard() {
        let (mut r, mut store) = reg();
        seed(&mut store, "wandr.dioxus.demo", 10, Role::Foreground);
        seed(&mut store, KEYGUARD_APP_ID, 99, Role::Background);

        let (reply, eff) = r.dispatch_command("lock", "", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        // App demoted, keyguard shown.
        assert!(eff.iter().any(|e| matches!(e, Effect::SetRole { pid: 10, role: Role::Background })));
        assert!(eff.iter().any(|e| matches!(e, Effect::SetRole { pid: 99, role: Role::Lockscreen })));
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(99), Some(Role::Lockscreen));
        // visible_app excludes the keyguard (not a switchable surface).
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().visible_app(), None);
    }

    #[test]
    fn unlock_restores_the_covered_app() {
        let (mut r, mut store) = reg();
        seed(&mut store, "wandr.dioxus.demo", 10, Role::Foreground);
        seed(&mut store, KEYGUARD_APP_ID, 99, Role::Background);
        r.dispatch_command("lock", "", &mut store).unwrap();
        let (_reply, eff) = r.dispatch_command("unlock", "", &mut store).unwrap();
        // Keyguard hidden + the covered app re-foregrounded (via the shell verb).
        assert!(eff.iter().any(|e| matches!(e, Effect::SetRole { pid: 99, role: Role::Background })));
        assert!(eff.iter().any(|e| matches!(e, Effect::Foreground { app_id } if app_id == "wandr.dioxus.demo")));
    }

    #[test]
    fn power_menu_over_lock_demotes_keyguard_for_input() {
        let (mut r, mut store) = reg();
        seed(&mut store, "wandr.dioxus.demo", 10, Role::Foreground);
        seed(&mut store, KEYGUARD_APP_ID, 99, Role::Background);
        seed(&mut store, POWERMENU_APP_ID, 77, Role::Background);
        r.dispatch_command("lock", "", &mut store).unwrap();
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(99), Some(Role::Lockscreen));

        // Show the menu over the lock screen: keyguard must drop out of the input
        // list (Background) so the power menu becomes the sole Lockscreen surface.
        let (_reply, eff) = r.dispatch_command("power-menu", "", &mut store).unwrap();
        assert!(eff.iter().any(|e| matches!(e, Effect::SetRole { pid: 99, role: Role::Background })));
        assert!(eff.iter().any(|e| matches!(e, Effect::SetRole { pid: 77, role: Role::Lockscreen })));
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(99), Some(Role::Background));
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(77), Some(Role::Lockscreen));

        // Dismiss restores the lock screen as the focusable Lockscreen surface.
        let (_reply, eff) = r.dispatch_command("pm-dismiss", "", &mut store).unwrap();
        assert!(eff.iter().any(|e| matches!(e, Effect::SetRole { pid: 99, role: Role::Lockscreen })));
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(99), Some(Role::Lockscreen));
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(77), Some(Role::Background));
    }

    #[test]
    fn screen_off_auto_locks() {
        let (mut r, mut store) = reg();
        seed(&mut store, "wandr.dioxus.demo", 10, Role::Foreground);
        seed(&mut store, KEYGUARD_APP_ID, 99, Role::Background);
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        assert!(eff.iter().any(|e| matches!(e, Effect::SetRole { pid: 99, role: Role::Lockscreen })));
        // Idempotent — a second off tick doesn't re-lock/re-demote.
        let eff2 = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        assert!(eff2.is_empty());
    }
}
