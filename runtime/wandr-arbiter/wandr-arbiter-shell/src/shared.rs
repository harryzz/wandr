//! Cross-cutting helpers shared by the AM ([`crate::am`]) and IME ([`crate::ime`])
//! command halves. These are the Store snapshot accessors, the role-transition
//! primitives (`apply`/`place`), and the promote/demote/reconcile orchestration
//! that both halves call — kept together because the focus-follows-foreground
//! coupling (Android's `finishInput()`) means an AM foreground change drives IME
//! overlay/editor teardown in the same pass. Internal organization only (task 74
//! C left these as one module); the crate is still the single AMS+IMMS module.

use wandr_arbiter_core::{Ctx, Effect, EditorInfo, Event, Role, Store, PRIMARY_DISPLAY};

/// Chrome app-ids that are overlays/system surfaces, not switchable user apps —
/// excluded from the recents/cycle ring (task 56).
pub(crate) const CHROME_APP_IDS: [&str; 4] =
    ["wandr.statusbar", "wandr.taskbar", "wandr.ime.keyboard", "wandr.keyguard"];

// ── owned-snapshot helpers over the Store (avoid borrow conflicts with ctx) ──

pub(crate) fn visible_app(s: &Store) -> Option<i32> {
    s.display(PRIMARY_DISPLAY).and_then(|d| d.visible_app())
}
pub(crate) fn overlay_desired(s: &Store) -> Option<i32> {
    s.display(PRIMARY_DISPLAY).and_then(|d| d.overlay_desired())
}
pub(crate) fn overlay_engaged(s: &Store) -> bool {
    s.display(PRIMARY_DISPLAY).map(|d| d.overlay_engaged()).unwrap_or(false)
}
pub(crate) fn foreground_slot(s: &Store) -> Option<(i32, String)> {
    s.display(PRIMARY_DISPLAY)
        .and_then(|d| d.foreground_slot())
        .map(|s| (s.pid, s.app_id.clone()))
}
pub(crate) fn role_pid(s: &Store, role: Role) -> Option<(i32, String)> {
    s.display(PRIMARY_DISPLAY)
        .and_then(|d| d.first_with_role(role))
        .map(|s| (s.pid, s.app_id.clone()))
}
pub(crate) fn active_ime(s: &Store) -> Option<(i32, String)> {
    s.display(PRIMARY_DISPLAY)
        .and_then(|d| d.active_ime_surface())
        .map(|s| (s.pid, s.app_id.clone()))
}
pub(crate) fn ime_editor(s: &Store) -> Option<(i32, EditorInfo)> {
    s.display(PRIMARY_DISPLAY)
        .and_then(|d| d.ime_editor().map(|(pid, info)| (pid, info.clone())))
}

// ── role transition: mutate the model + emit the mechanism effect together ──

/// Set a surface's role: emit the `SetRole` effect (the binary maps Role→signal/
/// oom/present) and mirror it into the model. The surface must already exist.
pub(crate) fn apply(ctx: &mut Ctx, pid: i32, role: Role) {
    ctx.request(Effect::SetRole { pid, role });
    ctx.store.display_mut(PRIMARY_DISPLAY).set_role(pid, role);
}

/// Place a surface (create-or-update with app_id) in a role + emit the effect.
/// Used for the promoted target so the surface + app_id are guaranteed present.
pub(crate) fn place(ctx: &mut Ctx, pid: i32, app_id: &str, role: Role) {
    ctx.request(Effect::SetRole { pid, role });
    ctx.store.display_mut(PRIMARY_DISPLAY).put_surface(pid, app_id, role);
}

// ── orchestration (the migrated promote/demote/reconcile helpers) ────────────

/// Promote `(app_id, pid)` to foreground; demote the prior foreground. Idempotent
/// if it is already foreground. Tears down any overlay split first, then re-derives.
pub(crate) fn promote_to_foreground(ctx: &mut Ctx, app_id: &str, pid: i32) {
    if overlay_engaged(ctx.store) {
        demote_from_overlay(ctx);
    }
    if let Some((prev_pid, prev_id)) = role_pid(ctx.store, Role::Foreground) {
        if prev_pid != pid {
            log::info!("arbiter: demoting prior foreground app={prev_id} pid={prev_pid}");
            apply(ctx, prev_pid, Role::Background);
            // Focus-follows-foreground (Android finishInput()): the app losing the
            // foreground also loses any editor focus.
            drop_editor_focus_of(ctx, prev_pid);
        }
    }
    log::info!("arbiter: promoting foreground app={app_id} pid={pid}");
    place(ctx, pid, app_id, Role::Foreground);
    reconcile_overlay(ctx);
    ctx.emit(Event::ForegroundChanged {
        app_id: Some(app_id.to_string()),
        pid: Some(pid),
    });
}

/// Promote an overlay (the IME) above the current foreground, demoting it to
/// OverlayBehind. Returns the demoted (behind) pid, if any.
pub(crate) fn promote_to_overlay(
    ctx: &mut Ctx,
    overlay_app_id: &str,
    overlay_pid: i32,
    behind_pid_hint: Option<i32>,
) -> Option<i32> {
    let demoted = match role_pid(ctx.store, Role::Foreground) {
        Some((prev_pid, prev_id)) if prev_pid != overlay_pid => {
            log::info!("arbiter: overlay-demoting prior foreground app={prev_id} pid={prev_pid}");
            apply(ctx, prev_pid, Role::OverlayBehind);
            Some(prev_pid)
        }
        _ => {
            // No distinct foreground; record the caller's behind hint (the focused
            // editor) as the behind surface in the model (no signal — matches the
            // legacy hint path).
            if let Some(h) = behind_pid_hint {
                ctx.store.display_mut(PRIMARY_DISPLAY).set_role(h, Role::OverlayBehind);
            }
            behind_pid_hint
        }
    };
    log::info!("arbiter: overlay-promoting app={overlay_app_id} pid={overlay_pid}");
    place(ctx, overlay_pid, overlay_app_id, Role::Overlay);
    demoted
}

/// Tear down the overlay split: overlay→Background, behind→Foreground. Returns
/// the (overlay_pid, behind_pid) that was active, if any.
pub(crate) fn demote_from_overlay(ctx: &mut Ctx) -> Option<(i32, i32)> {
    let overlay_pid = role_pid(ctx.store, Role::Overlay).map(|(p, _)| p)?;
    let behind_pid = role_pid(ctx.store, Role::OverlayBehind).map(|(p, _)| p);
    log::info!(
        "arbiter: overlay-clearing overlay_pid={overlay_pid} behind_pid={}",
        behind_pid.unwrap_or(0)
    );
    apply(ctx, overlay_pid, Role::Background);
    if let Some(bp) = behind_pid {
        apply(ctx, bp, Role::Foreground);
    }
    Some((overlay_pid, behind_pid.unwrap_or(0)))
}

/// THE single authority for IME-overlay visibility — derived state, reconciled
/// in one place. Idempotent after any fg/focus/ime change.
pub(crate) fn reconcile_overlay(ctx: &mut Ctx) {
    let visible = visible_app(ctx.store);
    match (overlay_desired(ctx.store), overlay_engaged(ctx.store)) {
        (Some(_), false) => {
            let (ime_pid, ime_app) = active_ime(ctx.store).expect("desired implies ime");
            promote_to_overlay(ctx, &ime_app, ime_pid, visible);
        }
        (None, true) => {
            demote_from_overlay(ctx);
        }
        _ => {}
    }
}

/// Clear editor focus if it belongs to `pid` (Android finishInput() on focus
/// loss) and notify the active IME so it drops composing state.
pub(crate) fn drop_editor_focus_of(ctx: &mut Ctx, pid: i32) {
    if ime_editor(ctx.store).map(|(p, _)| p) != Some(pid) {
        return;
    }
    ctx.store.display_mut(PRIMARY_DISPLAY).set_ime_editor(None);
    if let Some((ime_pid, _)) = active_ime(ctx.store) {
        ctx.deliver_to_host(ime_pid, "editor-detached\n".to_string());
    }
    // Keyboard gone for this (now-backgrounded) editor → WM pushes geometry with
    // keyboard_px=0 to `pid` (NOT the visible app — this editor is being demoted).
    ctx.emit(Event::EditorFocusChanged { editor: pid, focused: false });
    log::info!("arbiter: dropped editor focus of pid={pid} (lost foreground)");
}

/// Prune a surface whose process exited. Pairs with the binary's death watcher,
/// which injects [`Event::SurfaceRemoved`] then does the home-fallback launch.
pub(crate) fn on_surface_removed(ctx: &mut Ctx, pid: i32) {
    // Overlay teardown with signal semantics if this pid was either side.
    if overlay_engaged(ctx.store) {
        let involved = role_pid(ctx.store, Role::Overlay).map(|(p, _)| p) == Some(pid)
            || role_pid(ctx.store, Role::OverlayBehind).map(|(p, _)| p) == Some(pid);
        if involved {
            let cleared = demote_from_overlay(ctx);
            log::info!(
                "arbiter: surface-removed pid={pid} tore down overlay split (cleared={:?})",
                cleared.is_some()
            );
        }
    }
    let app_id = ctx.store.app_by_pid(pid).map(|a| a.app_id.clone());
    if let Some(app_id) = app_id {
        ctx.store.remove_app(&app_id);
    }
    ctx.store.display_mut(PRIMARY_DISPLAY).remove_surface(pid);
}
