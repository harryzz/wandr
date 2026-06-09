//! wandr-arbiter-shell — the AMS + IMMS orchestration module (task 74 C).
//!
//! This crate owns the policy the surface/role model was built to untangle: app
//! foreground/background, the IME overlay split, editor focus, IME input
//! routing, and task cycling. It plugs into [`wandr_arbiter_core`] as an
//! [`ArbiterModule`]: each command verb reads/writes the core `Store` through
//! `Ctx` and **declares mechanism as `Effect`s** (`Effect::SetRole` for a role
//! transition, `Effect::HostLine` for a per-host socket push, `Effect::Kill` for
//! a process kill) — the binary performs them. It never touches a signal,
//! `/proc`, a socket, or the zygote.
//!
//! ## Internal layout (by Android responsibility)
//! The two halves Android's `system_server` keeps coordinating are split into
//! submodules for readability — but kept ONE [`ArbiterModule`] (one registration)
//! because focus-follows-foreground (`finishInput()`) couples them in a single
//! synchronous pass, so they share direct calls rather than an event cascade:
//!   * [`am`]     — ActivityManager: foreground / kill / back / task-cycle / list
//!                  + chrome-surface registration.
//!   * [`ime`]    — InputMethodManager: active IME, editor attach/detach + focus,
//!                  the overlay split, `ime-*` routing.
//!   * [`shared`] — the snapshot accessors + promote/demote/reconcile orchestration
//!                  both halves call.
//!
//! ## What stays in the binary (mechanism / zygote coupling)
//! `launch` / `launch-overlay` / `launch-headless` / `preload` (they call the
//! zygote and own the returned pid), `go-home` / `set-home` (they may relaunch
//! the home app), the death-watcher threads, and the `Effect` executor. The
//! binary bridges into this module by dispatching `foreground <app-id>` after a
//! launch, and by injecting `Event::SurfaceRemoved` when a child dies (this
//! module prunes; the binary then does the home-fallback launch).

use wandr_arbiter_core::{ArbiterModule, Ctx, Event, Reply};

mod am;
mod ime;
mod shared;

/// The AMS+IMMS orchestration module. Stateless — all state lives in the Store.
#[derive(Default)]
pub struct ShellModule;

impl ShellModule {
    pub fn new() -> Self {
        ShellModule
    }
}

impl ArbiterModule for ShellModule {
    fn verbs(&self) -> &[&'static str] {
        &[
            "foreground",
            "register-chrome",
            "kill",
            "set-ime",
            "attach-editor",
            "detach-editor",
            "ime-overlay-height",
            "ime-commit-text",
            "ime-send-key-event",
            "ime-set-composing-text",
            "ime-finish-composing-text",
            "ime-set-selection",
            "back",
            "cycle-task",
            "overlay",
            "overlay-clear",
            "list",
        ]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            // AM (see `am`)
            "foreground" => self.cmd_foreground(args, ctx),
            "register-chrome" => self.cmd_register_chrome(args, ctx),
            "kill" => self.cmd_kill(args, ctx),
            "back" => self.cmd_back(ctx),
            "cycle-task" => self.cmd_cycle_task(ctx),
            "list" => self.cmd_list(ctx),
            // IME (see `ime`)
            "set-ime" => self.cmd_set_ime(args, ctx),
            "attach-editor" => self.cmd_attach_editor(args, ctx),
            "detach-editor" => self.cmd_detach_editor(args, ctx),
            "ime-overlay-height" => self.cmd_ime_overlay_height(args, ctx),
            "ime-commit-text" => self.cmd_ime_route("commit-text", args, ctx),
            "ime-send-key-event" => self.cmd_ime_route("send-key-event", args, ctx),
            "ime-set-composing-text" => self.cmd_ime_route("set-composing-text", args, ctx),
            "ime-finish-composing-text" => self.cmd_ime_route("finish-composing-text", args, ctx),
            "ime-set-selection" => self.cmd_ime_route("set-selection", args, ctx),
            "overlay" => self.cmd_overlay(args, ctx),
            "overlay-clear" => self.cmd_overlay_clear(ctx),
            other => Reply::err(format!("shell-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        if let Event::SurfaceRemoved { pid } = ev {
            shared::on_surface_removed(ctx, *pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ShellModule;
    use std::time::{Instant, SystemTime};
    use wandr_arbiter_core::{
        AppState, EditorInfo, Effect, Event, Registry, Reply, Role, Store, PRIMARY_DISPLAY,
    };

    fn reg() -> (Registry, Store) {
        let mut r = Registry::new();
        r.register(Box::new(ShellModule::new()));
        (r, Store::new())
    }

    fn seed_app(store: &mut Store, id: &str, pid: i32, role: Role) {
        store.insert_app(AppState {
            app_id: id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
        store.display_mut(PRIMARY_DISPLAY).put_surface(pid, id, role);
    }

    #[test]
    fn attach_editor_engages_overlay_in_order() {
        let (mut r, mut store) = reg();
        seed_app(&mut store, "app", 10, Role::Foreground);
        seed_app(&mut store, "ime", 20, Role::Background);
        store.display_mut(PRIMARY_DISPLAY).active_ime = Some(20);

        let (reply, effects) = r.dispatch_command("attach-editor", "10 text", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        // The split is engaged in the model: app behind, ime overlay, visible=app.
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().visible_app(), Some(10));
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(10), Some(Role::OverlayBehind));
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(20), Some(Role::Overlay));
        // Effect order: demote behind (OverlayBehind) → promote overlay (Overlay)
        // → editor-attached host line. SetRole(OverlayBehind, 10) precedes
        // SetRole(Overlay, 20).
        let roles: Vec<_> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SetRole { pid, role } => Some((*pid, *role)),
                _ => None,
            })
            .collect();
        assert_eq!(roles, vec![(10, Role::OverlayBehind), (20, Role::Overlay)]);
        assert!(effects.iter().any(|e| matches!(e, Effect::HostLine { pid: 20, .. })));
    }

    #[test]
    fn register_chrome_adds_chrome_surface_inert_to_policy() {
        let (mut r, mut store) = reg();
        seed_app(&mut store, "app", 10, Role::Foreground);
        // Density known → the reply carries the dp×density strip height (top → 133).
        store.geometry_mut(PRIMARY_DISPLAY).density = 560.0 / 160.0;
        let (reply, _eff) = r.dispatch_command("register-chrome", "wandr.statusbar 50 top", &mut store).unwrap();
        assert_eq!(reply.render(), "OK chrome wandr.statusbar pid=50 height=133");
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(50), Some(Role::Chrome));
        // Inert for AM policy: chrome is never the visible app.
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().visible_app(), Some(10));
        assert!(store.app("wandr.statusbar").is_some());
    }

    #[test]
    fn cycle_task_rings_off_visible_not_ime() {
        let (mut r, mut store) = reg();
        seed_app(&mut store, "a.app", 10, Role::OverlayBehind); // visible (behind overlay)
        seed_app(&mut store, "wandr.ime.keyboard", 20, Role::Overlay); // chrome, filtered
        seed_app(&mut store, "b.app", 30, Role::Background);
        store.display_mut(PRIMARY_DISPLAY).active_ime = Some(20);
        store.display_mut(PRIMARY_DISPLAY).set_ime_editor(Some((10, EditorInfo::default())));

        let (reply, _eff) = r.dispatch_command("cycle-task", "", &mut store).unwrap();
        // Ring = {a.app, b.app}; visible=a.app(10) → next=b.app(30).
        assert_eq!(reply.render(), "OK cycle-task → b.app pid=30");
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().visible_app(), Some(30));
    }

    #[test]
    fn surface_removed_prunes_and_tears_down_split() {
        let (mut r, mut store) = reg();
        seed_app(&mut store, "app", 10, Role::OverlayBehind);
        seed_app(&mut store, "ime", 20, Role::Overlay);
        store.display_mut(PRIMARY_DISPLAY).active_ime = Some(20);
        // The behind app dies.
        let _ = r.dispatch_event(Event::SurfaceRemoved { pid: 10 }, &mut store);
        assert!(store.app("app").is_none());
        assert!(store.display(PRIMARY_DISPLAY).unwrap().role_of(10).is_none());
        // Overlay torn down: ime back to Background, no foreground left.
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(20), Some(Role::Background));
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().visible_app(), None);
    }
}
