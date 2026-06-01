//! wart-arbiter-shell — the AMS + IMMS orchestration module (task 74 C).
//!
//! This crate owns the policy the surface/role model was built to untangle: app
//! foreground/background, the IME overlay split, editor focus, IME input
//! routing, and task cycling. It plugs into [`wart_arbiter_core`] as an
//! [`ArbiterModule`]: each command verb reads/writes the core [`Store`] through
//! [`Ctx`] and **declares mechanism as [`Effect`]s** ([`Effect::SetRole`] for a
//! role transition, [`Effect::HostLine`] for a per-host socket push,
//! [`Effect::Kill`] for a process kill) — the binary performs them. It never
//! touches a signal, `/proc`, a socket, or the zygote.
//!
//! ## What stays in the binary (mechanism / zygote coupling)
//! `launch` / `launch-overlay` / `launch-headless` / `preload` (they call the
//! zygote and own the returned pid), `go-home` / `set-home` (they may relaunch
//! the home app), the death-watcher threads, and the [`Effect`] executor. The
//! binary bridges into this module by dispatching `foreground <app-id>` after a
//! launch, and by injecting [`Event::SurfaceRemoved`] when a child dies (this
//! module prunes; the binary then does the home-fallback launch).
//!
//! ## Effect ordering
//! Effects run in emission order *after* the handler returns, so each transition
//! emits its role effects within one handler in the legacy imperative order
//! (demote-behind → promote-overlay, etc.); the store is mutated inline so
//! derivations stay correct mid-handler.

use std::time::{Instant, SystemTime};

use wart_arbiter_core::{
    AppState, ArbiterModule, Ctx, Effect, EditorInfo, Event, Reply, Role, Store, PRIMARY_DISPLAY,
};

/// OOM scores are applied by the binary's `apply_role`; the module only sets roles.
///
/// Chrome app-ids that are overlays/system surfaces, not switchable user apps —
/// excluded from the recents/cycle ring (task 56).
const CHROME_APP_IDS: [&str; 3] = ["war.statusbar", "war.taskbar", "war.ime.keyboard"];

/// The AMS+IMMS orchestration module. Stateless — all state lives in the Store.
#[derive(Default)]
pub struct ShellModule;

impl ShellModule {
    pub fn new() -> Self {
        ShellModule
    }
}

// ── owned-snapshot helpers over the Store (avoid borrow conflicts with ctx) ──

fn visible_app(s: &Store) -> Option<i32> {
    s.display(PRIMARY_DISPLAY).and_then(|d| d.visible_app())
}
fn overlay_desired(s: &Store) -> Option<i32> {
    s.display(PRIMARY_DISPLAY).and_then(|d| d.overlay_desired())
}
fn overlay_engaged(s: &Store) -> bool {
    s.display(PRIMARY_DISPLAY).map(|d| d.overlay_engaged()).unwrap_or(false)
}
fn foreground_slot(s: &Store) -> Option<(i32, String)> {
    s.display(PRIMARY_DISPLAY)
        .and_then(|d| d.foreground_slot())
        .map(|s| (s.pid, s.app_id.clone()))
}
fn role_pid(s: &Store, role: Role) -> Option<(i32, String)> {
    s.display(PRIMARY_DISPLAY)
        .and_then(|d| d.first_with_role(role))
        .map(|s| (s.pid, s.app_id.clone()))
}
fn active_ime(s: &Store) -> Option<(i32, String)> {
    s.display(PRIMARY_DISPLAY)
        .and_then(|d| d.active_ime_surface())
        .map(|s| (s.pid, s.app_id.clone()))
}
fn ime_editor(s: &Store) -> Option<(i32, EditorInfo)> {
    s.display(PRIMARY_DISPLAY)
        .and_then(|d| d.ime_editor().map(|(pid, info)| (pid, info.clone())))
}

// ── role transition: mutate the model + emit the mechanism effect together ──

/// Set a surface's role: emit the `SetRole` effect (the binary maps Role→signal/
/// oom/present) and mirror it into the model. The surface must already exist.
fn apply(ctx: &mut Ctx, pid: i32, role: Role) {
    ctx.request(Effect::SetRole { pid, role });
    ctx.store.display_mut(PRIMARY_DISPLAY).set_role(pid, role);
}

/// Place a surface (create-or-update with app_id) in a role + emit the effect.
/// Used for the promoted target so the surface + app_id are guaranteed present.
fn place(ctx: &mut Ctx, pid: i32, app_id: &str, role: Role) {
    ctx.request(Effect::SetRole { pid, role });
    ctx.store.display_mut(PRIMARY_DISPLAY).put_surface(pid, app_id, role);
}

// ── orchestration (the migrated promote/demote/reconcile helpers) ────────────

/// Promote `(app_id, pid)` to foreground; demote the prior foreground. Idempotent
/// if it is already foreground. Tears down any overlay split first, then re-derives.
pub fn promote_to_foreground(ctx: &mut Ctx, app_id: &str, pid: i32) {
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
pub fn promote_to_overlay(
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
pub fn demote_from_overlay(ctx: &mut Ctx) -> Option<(i32, i32)> {
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
pub fn reconcile_overlay(ctx: &mut Ctx) {
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
pub fn drop_editor_focus_of(ctx: &mut Ctx, pid: i32) {
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
fn on_surface_removed(ctx: &mut Ctx, pid: i32) {
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

/// Symmetric with wart-host/src/ime_inbound.rs::unescape_underscores.
/// Empty → "-"; otherwise spaces → "_". Task 49 step 1a wire format.
fn escape_underscores(s: &str) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.replace(' ', "_")
}

// ── command handlers ─────────────────────────────────────────────────────────

impl ShellModule {
    fn cmd_foreground(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
        if app_id.is_empty() {
            return Reply::err("foreground-empty-app-id");
        }
        let Some(s) = ctx.store.app(app_id).cloned() else {
            return Reply::err(format!("not-tracked {app_id}"));
        };
        let prev = foreground_slot(ctx.store).map(|(_, id)| id);
        promote_to_foreground(ctx, &s.app_id, s.pid);
        let prev_str = prev.unwrap_or_else(|| "(none)".to_string());
        Reply::ok(format!("fg={app_id} prev={prev_str} pid={}", s.pid))
    }

    /// `register-chrome <app-id> <pid>` — a host-spawned chrome overlay
    /// (statusbar/taskbar) self-registers so the arbiter tracks it as a
    /// `Role::Chrome` surface and can fan orientation to its control socket
    /// (chrome-coherence). The pid is the overlay's own; the control socket is
    /// `wart-host-<pid>.sock`. Chrome is excluded from `visible_app`/the cycle
    /// ring by role, so this is inert for AM/IME policy. Death cleanup is the
    /// existing liveness poller → `SurfaceRemoved`.
    fn cmd_register_chrome(&mut self, rest: &str, ctx: &mut Ctx) -> Reply {
        let mut parts = rest.split_whitespace();
        let (Some(app_id), Some(pid_s)) = (parts.next(), parts.next()) else {
            return Reply::err("register-chrome-args: expected <app-id> <pid>");
        };
        let Ok(pid) = pid_s.parse::<i32>() else {
            return Reply::err(format!("register-chrome-bad-pid {pid_s}"));
        };
        ctx.store.insert_app(AppState {
            app_id: app_id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
        ctx.store
            .display_mut(PRIMARY_DISPLAY)
            .put_surface(pid, app_id, Role::Chrome);
        log::info!("arbiter: registered chrome surface app={app_id} pid={pid}");
        Reply::ok(format!("chrome {app_id} pid={pid}"))
    }

    fn cmd_kill(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
        if app_id.is_empty() {
            return Reply::err("kill-empty-app-id");
        }
        let Some(s) = ctx.store.app(app_id).cloned() else {
            return Reply::err(format!("not-tracked {app_id}"));
        };
        // The binary performs the kill; the death watcher will also fire, but we
        // prune the model now so `list` reflects it immediately (matches legacy).
        ctx.request(Effect::Kill { pid: s.pid });
        ctx.store.remove_app(app_id);
        ctx.store.display_mut(PRIMARY_DISPLAY).remove_surface(s.pid);
        log::info!("wart-arbiter: kill {app_id} pid={}", s.pid);
        Reply::ok(format!("killed app={app_id} pid={}", s.pid))
    }

    fn cmd_set_ime(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
        if app_id.is_empty() {
            return Reply::err("set-ime-empty-app-id");
        }
        if app_id == "-" {
            let prev_id = active_ime(ctx.store).map(|(_, id)| id).unwrap_or_else(|| "(none)".into());
            ctx.store.display_mut(PRIMARY_DISPLAY).active_ime = None;
            log::info!("arbiter: cleared active IME (prev={prev_id})");
            return Reply::ok(format!("cleared prev={prev_id}"));
        }
        let Some(s) = ctx.store.app(app_id).cloned() else {
            return Reply::err(format!("not-running {app_id} (launch first)"));
        };
        let prev_id = active_ime(ctx.store).map(|(_, id)| id).unwrap_or_else(|| "(none)".into());
        ctx.store.display_mut(PRIMARY_DISPLAY).active_ime = Some(s.pid);
        log::info!("arbiter: set active IME app={app_id} pid={} (prev={prev_id})", s.pid);
        Reply::ok(format!("ime={app_id} pid={} prev={prev_id}", s.pid))
    }

    fn cmd_ime_overlay_height(&mut self, rest: &str, ctx: &mut Ctx) -> Reply {
        let Ok(px) = rest.trim().parse::<u32>() else {
            return Reply::err(format!("ime-overlay-height-bad-px {rest:?}"));
        };
        ctx.store.set_ime_height(px);
        // Feed the WM module the new keyboard height; it re-pushes geometry to the
        // focused editor (if any). Gated on the WM's own focused-editor cache, so
        // emitting unconditionally is correct.
        ctx.emit(Event::ImeHeightChanged { id: PRIMARY_DISPLAY, px });
        log::info!("arbiter: ime-overlay-height {px}");
        Reply::ok(format!("ime-overlay-height {px}"))
    }

    fn cmd_attach_editor(&mut self, rest: &str, ctx: &mut Ctx) -> Reply {
        // Wire: `attach-editor <pid> [input-type] [hint] [initial-text]`.
        let mut parts = rest.splitn(4, ' ');
        let Some(pid_s) = parts.next() else {
            return Reply::err("attach-editor-missing-pid");
        };
        let Ok(pid) = pid_s.parse::<i32>() else {
            return Reply::err(format!("attach-editor-bad-pid {pid_s}"));
        };
        let Some(owner) = ctx.store.app_by_pid(pid).cloned() else {
            return Reply::err(format!("attach-editor-unknown-pid {pid}"));
        };

        let input_type = parts.next().unwrap_or("text").to_string();
        let hint = parts.next().unwrap_or("").to_string();
        let initial_text = parts.next().unwrap_or("").to_string();

        let prev_pid = ime_editor(ctx.store)
            .map(|(p, _)| p.to_string())
            .unwrap_or_else(|| "-".to_string());
        ctx.store.display_mut(PRIMARY_DISPLAY).set_ime_editor(Some((
            pid,
            EditorInfo {
                input_type: input_type.clone(),
                hint: hint.clone(),
                initial_text: initial_text.clone(),
                initial_selection_start: 0,
                initial_selection_end: 0,
            },
        )));

        let ime = active_ime(ctx.store);
        let ime_dest = ime
            .as_ref()
            .map(|(p, id)| format!("{id} (pid={p})"))
            .unwrap_or_else(|| "(no active IME — set-ime first)".to_string());

        // Re-derive IME-overlay visibility (engages the split when this app is the
        // visible foreground and there's an active IME on another pid).
        reconcile_overlay(ctx);
        let auto_overlay = overlay_engaged(ctx.store) && visible_app(ctx.store) == Some(pid);

        // Deliver editor-attached to the IME host (deferred via Effect::HostLine).
        let routed = if let Some((ime_pid, _)) = &ime {
            let line = format!(
                "editor-attached {input_type} {hint_esc} {text_esc} 0 0\n",
                hint_esc = escape_underscores(&hint),
                text_esc = escape_underscores(&initial_text),
            );
            ctx.deliver_to_host(*ime_pid, line);
            true
        } else {
            false
        };

        // Keyboard will occlude this editor → hand the WM the current keyboard
        // height + mark this editor focused (only when an IME on a different pid
        // is actually shown).
        if let Some((ime_pid, _)) = &ime {
            if *ime_pid != pid {
                let h = ctx.store.ime_height();
                // ImeHeightChanged carries the keyboard-inset push to this editor
                // (the WM reads the focused editor from the Store, set just above);
                // EditorFocusChanged{focused:true} is the focus-gain notice (the WM
                // no-ops it to avoid double-pushing — the height event does it).
                ctx.emit(Event::ImeHeightChanged { id: PRIMARY_DISPLAY, px: h });
                ctx.emit(Event::EditorFocusChanged { editor: pid, focused: true });
            }
        }

        log::info!(
            "arbiter: attach-editor pid={pid} app={} input-type={input_type} → route to {ime_dest} \
             auto-overlay={auto_overlay} routed={routed}",
            owner.app_id,
        );
        Reply::ok(format!(
            "attached editor pid={pid} app={} input-type={input_type} prev-pid={prev_pid} \
             route→{ime_dest} overlay={} routed={routed}",
            owner.app_id,
            if auto_overlay { "engaged" } else { "skipped" },
        ))
    }

    fn cmd_detach_editor(&mut self, rest: &str, ctx: &mut Ctx) -> Reply {
        let pid_s = rest.trim();
        if pid_s.is_empty() {
            return Reply::err("detach-editor-missing-pid");
        }
        let Ok(pid) = pid_s.parse::<i32>() else {
            return Reply::err(format!("detach-editor-bad-pid {pid_s}"));
        };
        let was_focused = ime_editor(ctx.store).map(|(p, _)| p) == Some(pid);
        if !was_focused {
            log::info!("arbiter: detach-editor pid={pid} — was not focused, no-op");
            return Reply::ok(format!("no-op pid={pid} (not focused)"));
        }
        ctx.store.display_mut(PRIMARY_DISPLAY).set_ime_editor(None);
        let ime = active_ime(ctx.store);
        let ime_dest = ime
            .as_ref()
            .map(|(p, id)| format!("{id} (pid={p})"))
            .unwrap_or_else(|| "(no active IME)".to_string());
        // Evaluate engagement before the reconcile (roles unchanged until then).
        let was_engaged = overlay_engaged(ctx.store) && visible_app(ctx.store) == Some(pid);
        reconcile_overlay(ctx);
        let cleared = was_engaged && !overlay_engaged(ctx.store);
        let routed = if let Some((ime_pid, _)) = &ime {
            ctx.deliver_to_host(*ime_pid, "editor-detached\n".to_string());
            true
        } else {
            false
        };
        // Keyboard gone → WM pushes geometry keyboard_px=0 to this editor host.
        ctx.emit(Event::EditorFocusChanged { editor: pid, focused: false });
        log::info!(
            "arbiter: detach-editor pid={pid} → route to {ime_dest} auto-overlay-clear={cleared} \
             routed={routed}"
        );
        Reply::ok(format!(
            "detached pid={pid} route→{ime_dest} overlay={} routed={routed}",
            if cleared { "cleared" } else { "skipped" },
        ))
    }

    /// Shared backend for the five `ime-*` routing verbs (task 47 step 3a).
    fn cmd_ime_route(&mut self, verb: &str, rest: &str, ctx: &mut Ctx) -> Reply {
        let Some((focus_pid, focus_info)) = ime_editor(ctx.store) else {
            return Reply::err("no-focused-editor");
        };
        if verb == "send-key-event" {
            // ime-send-key-event <code-point> <key-id> <down|up> → key-event ... .
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() != 3 {
                return Reply::err("send-key-event-bad-args: expected <code-point> <key-id> <down|up>");
            }
            let line = format!("key-event {} {} {}\n", parts[0], parts[1], parts[2]);
            ctx.deliver_to_host(focus_pid, line);
            log::info!(
                "arbiter: ime-send-key-event → pid={focus_pid} ({} {} {}) queued",
                parts[0], parts[1], parts[2]
            );
            return Reply::ok(format!(
                "route→pid={focus_pid} (key-event {} {} {})",
                parts[0], parts[1], parts[2]
            ));
        }
        // commit-text / set-composing-text / finish-composing-text / set-selection
        // — log only (await step 3b editor-side WIT exports).
        log::info!(
            "arbiter: ime-{verb} → editor pid={focus_pid} app-input-type={} args={rest:?} \
             (step 3b delivers via new editor-side WIT exports)",
            focus_info.input_type,
        );
        Reply::ok(format!(
            "route→pid={focus_pid} (input-type={}) {verb} args={rest:?} [step 3b — log only]",
            focus_info.input_type,
        ))
    }

    fn cmd_back(&mut self, ctx: &mut Ctx) -> Reply {
        // Route ESC to the foreground slot (the IME during a split, so it can
        // dismiss the keyboard — preserving the legacy target).
        let Some((fg_pid, fg)) = foreground_slot(ctx.store) else {
            return Reply::ok("back noop (no foreground app)");
        };
        // ESC: code-point 0, key-id 27. Send down+up so either edge handler fires.
        ctx.deliver_to_host(fg_pid, "key-event 0 27 down\n".to_string());
        ctx.deliver_to_host(fg_pid, "key-event 0 27 up\n".to_string());
        log::info!("arbiter: back → ESC queued to fg={fg} pid={fg_pid}");
        Reply::ok(format!("back → pid={fg_pid} (esc)"))
    }

    fn cmd_cycle_task(&mut self, ctx: &mut Ctx) -> Reply {
        let mut apps: Vec<_> = ctx
            .store
            .apps_snapshot()
            .into_iter()
            .filter(|s| !CHROME_APP_IDS.contains(&s.app_id.as_str()))
            .collect();
        apps.sort_by(|a, b| a.app_id.cmp(&b.app_id));
        if apps.len() < 2 {
            return Reply::ok(format!("cycle-task noop ({} switchable app(s))", apps.len()));
        }
        // Current position = the VISIBLE app (the behind-app under an engaged
        // overlay; never the IME, which would be filtered out as chrome).
        let cur_pid = visible_app(ctx.store);
        let idx = cur_pid
            .and_then(|p| apps.iter().position(|s| s.pid == p))
            .unwrap_or(usize::MAX);
        let next = apps[idx.wrapping_add(1) % apps.len()].clone();
        promote_to_foreground(ctx, &next.app_id, next.pid);
        log::info!("arbiter: cycle-task → fg={} pid={} (ring of {})", next.app_id, next.pid, apps.len());
        Reply::ok(format!("cycle-task → {} pid={}", next.app_id, next.pid))
    }

    fn cmd_overlay(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
        if app_id.is_empty() {
            return Reply::err("overlay-empty-app-id");
        }
        let Some(ime) = ctx.store.app(app_id).cloned() else {
            return Reply::err(format!("overlay-not-tracked {app_id}"));
        };
        let prev_fg = foreground_slot(ctx.store);
        let behind_hint = prev_fg.as_ref().map(|(p, _)| *p);
        let demoted = promote_to_overlay(ctx, &ime.app_id, ime.pid, behind_hint);
        let prev_str = prev_fg.map(|(_, id)| id).unwrap_or_else(|| "(none)".to_string());
        let demoted_str = demoted.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string());
        Reply::ok(format!(
            "overlay={app_id} pid={} prev-fg={prev_str} behind-pid={demoted_str}",
            ime.pid
        ))
    }

    fn cmd_overlay_clear(&mut self, ctx: &mut Ctx) -> Reply {
        match demote_from_overlay(ctx) {
            Some((ime_pid, behind_pid)) => Reply::ok(format!(
                "overlay-cleared ime-pid={ime_pid} repromoted-behind-pid={behind_pid}"
            )),
            None => Reply::ok("overlay-was-not-active"),
        }
    }

    fn cmd_list(&mut self, ctx: &mut Ctx) -> Reply {
        let mut apps = ctx.store.apps_snapshot();
        apps.sort_by(|a, b| a.app_id.cmp(&b.app_id));
        let fg = foreground_slot(ctx.store).map(|(_, id)| id);
        let ime = active_ime(ctx.store).map(|(_, id)| id);
        let focus = ime_editor(ctx.store);
        let mut body = format!("count={}", apps.len());
        for app in apps {
            let elapsed_ms = app.launched_mono.elapsed().as_millis();
            let mut markers = String::new();
            if fg.as_deref() == Some(&app.app_id) {
                markers.push_str(" [fg]");
            }
            if ime.as_deref() == Some(&app.app_id) {
                markers.push_str(" [ime]");
            }
            if focus.as_ref().map(|(p, _)| *p) == Some(app.pid) {
                markers.push_str(&format!(" [editor:{}]", focus.as_ref().unwrap().1.input_type));
            }
            body.push_str(&format!(
                "\n  app={} pid={} elapsed_ms={elapsed_ms}{markers}",
                app.app_id, app.pid
            ));
        }
        Reply::ok(body)
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
            "foreground" => self.cmd_foreground(args, ctx),
            "register-chrome" => self.cmd_register_chrome(args, ctx),
            "kill" => self.cmd_kill(args, ctx),
            "set-ime" => self.cmd_set_ime(args, ctx),
            "attach-editor" => self.cmd_attach_editor(args, ctx),
            "detach-editor" => self.cmd_detach_editor(args, ctx),
            "ime-overlay-height" => self.cmd_ime_overlay_height(args, ctx),
            "ime-commit-text" => self.cmd_ime_route("commit-text", args, ctx),
            "ime-send-key-event" => self.cmd_ime_route("send-key-event", args, ctx),
            "ime-set-composing-text" => self.cmd_ime_route("set-composing-text", args, ctx),
            "ime-finish-composing-text" => self.cmd_ime_route("finish-composing-text", args, ctx),
            "ime-set-selection" => self.cmd_ime_route("set-selection", args, ctx),
            "back" => self.cmd_back(ctx),
            "cycle-task" => self.cmd_cycle_task(ctx),
            "overlay" => self.cmd_overlay(args, ctx),
            "overlay-clear" => self.cmd_overlay_clear(ctx),
            "list" => self.cmd_list(ctx),
            other => Reply::err(format!("shell-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        if let Event::SurfaceRemoved { pid } = ev {
            on_surface_removed(ctx, *pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wart_arbiter_core::{AppState, Registry};
    use std::time::{Instant, SystemTime};

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
        let (reply, _eff) = r.dispatch_command("register-chrome", "war.statusbar 50", &mut store).unwrap();
        assert_eq!(reply.render(), "OK chrome war.statusbar pid=50");
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().role_of(50), Some(Role::Chrome));
        // Inert for AM policy: chrome is never the visible app.
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().visible_app(), Some(10));
        assert!(store.app("war.statusbar").is_some());
    }

    #[test]
    fn cycle_task_rings_off_visible_not_ime() {
        let (mut r, mut store) = reg();
        seed_app(&mut store, "a.app", 10, Role::OverlayBehind); // visible (behind overlay)
        seed_app(&mut store, "war.ime.keyboard", 20, Role::Overlay); // chrome, filtered
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
