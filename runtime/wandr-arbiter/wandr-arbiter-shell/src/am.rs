//! AM half (Android's ActivityManager): app foreground/kill/back/task-cycle/list
//! + chrome-surface registration. Command handlers on [`crate::ShellModule`];
//! the shared promote/reconcile orchestration lives in [`crate::shared`].

use std::time::{Instant, SystemTime};

use wandr_arbiter_core::{
    AppState, ChromeAnchor, Ctx, Effect, Reply, Role, PRIMARY_DISPLAY, STATUS_BAR_DP, TASKBAR_DP,
};

use crate::shared::{
    active_ime, foreground_slot, ime_editor, promote_to_foreground, visible_app, CHROME_APP_IDS,
};

impl crate::ShellModule {
    pub(crate) fn cmd_foreground(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
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

    /// `register-chrome <app-id> <pid> <anchor>` — a host-spawned chrome overlay
    /// (statusbar/taskbar) self-registers so the arbiter tracks it as a
    /// `Role::Chrome` surface and can fan orientation to its control socket
    /// (chrome-coherence). `anchor` ∈ `top|bottom-bar`. The pid is the overlay's
    /// own; the control socket is `wandr-host-<pid>.sock`. Chrome is excluded from
    /// `visible_app`/the cycle ring by role, so this is inert for AM/IME policy.
    ///
    /// True-dp: the REPLY carries the strip `height` (dp×density for the anchor)
    /// so the overlay sizes its surface to the arbiter-authored value — the
    /// arbiter is the single source of the chrome heights. `height=0` if the
    /// density isn't known yet (host falls back); density is fixed per device.
    pub(crate) fn cmd_register_chrome(&mut self, rest: &str, ctx: &mut Ctx) -> Reply {
        let mut parts = rest.split_whitespace();
        let (Some(app_id), Some(pid_s)) = (parts.next(), parts.next()) else {
            return Reply::err("register-chrome-args: expected <app-id> <pid> <anchor>");
        };
        let Ok(pid) = pid_s.parse::<i32>() else {
            return Reply::err(format!("register-chrome-bad-pid {pid_s}"));
        };
        let anchor = parts.next().unwrap_or("");
        ctx.store.insert_app(AppState {
            app_id: app_id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
        let display = ctx.store.display_mut(PRIMARY_DISPLAY);
        display.put_surface(pid, app_id, Role::Chrome);
        // Record the anchor edge on the surface so the WMS can place this
        // chrome's input strip from the live insets (top→inset_top strip,
        // bottom→inset_bottom strip).
        let (dp, chrome_anchor) = match anchor {
            "top" => (STATUS_BAR_DP, Some(ChromeAnchor::Top)),
            "bottom-bar" => (TASKBAR_DP, Some(ChromeAnchor::Bottom)),
            _ => (0, None),
        };
        if let Some(a) = chrome_anchor {
            display.set_chrome_anchor(pid, a);
        }
        let height = ctx
            .store
            .geometry(PRIMARY_DISPLAY)
            .map(|g| g.dp_to_px(dp))
            .unwrap_or(0);
        log::info!("arbiter: registered chrome surface app={app_id} pid={pid} anchor={anchor} height={height}");
        Reply::ok(format!("chrome {app_id} pid={pid} height={height}"))
    }

    pub(crate) fn cmd_kill(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
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
        log::info!("wandr-arbiter: kill {app_id} pid={}", s.pid);
        Reply::ok(format!("killed app={app_id} pid={}", s.pid))
    }

    pub(crate) fn cmd_back(&mut self, ctx: &mut Ctx) -> Reply {
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

    pub(crate) fn cmd_cycle_task(&mut self, ctx: &mut Ctx) -> Reply {
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

    /// `task-list` (task 92) — machine-parseable running-app snapshot for the
    /// host's `wandr:task-manager/list-apps` impl. One line per app:
    /// `app=<id> pid=<n> state=<foreground|background|overlay|headless> uptime_ms=<n>`,
    /// ordered by app-id (stable across polls so a UI list doesn't jump). The host
    /// enriches each row with `/proc/<pid>` (CPU/mem) + the install-class `kind` +
    /// `label`; the arbiter is the authority only for the set + pid + role + uptime.
    pub(crate) fn cmd_task_list(&mut self, ctx: &mut Ctx) -> Reply {
        let mut apps = ctx.store.apps_snapshot();
        apps.sort_by(|a, b| a.app_id.cmp(&b.app_id));
        let mut body = format!("count={}", apps.len());
        for app in apps {
            let uptime_ms = app.launched_mono.elapsed().as_millis();
            let state = ctx
                .store
                .display(PRIMARY_DISPLAY)
                .and_then(|d| d.role_of(app.pid))
                .map(role_to_state)
                .unwrap_or("background");
            body.push_str(&format!(
                "\napp={} pid={} state={state} uptime_ms={uptime_ms}",
                app.app_id, app.pid
            ));
        }
        Reply::ok(body)
    }

    /// `task-kill <app-id>` (task 92) — the host's `kill-app` path. Distinct error
    /// tokens the host maps to `kill-error`: `not-found` (not tracked) / `protected`
    /// (a runtime-managed surface the arbiter would immediately relaunch:
    /// chrome/keyguard/launcher). Plain `system` apps are killable. On success
    /// mirrors `cmd_kill` (Effect::Kill + prune the model now so the next poll
    /// reflects it).
    pub(crate) fn cmd_task_kill(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
        if app_id.is_empty() {
            return Reply::err("not-found");
        }
        let Some(s) = ctx.store.app(app_id).cloned() else {
            return Reply::err("not-found");
        };
        let is_protected =
            CHROME_APP_IDS.contains(&app_id) || ctx.store.home() == Some(app_id);
        if is_protected {
            return Reply::err("protected");
        }
        ctx.request(Effect::Kill { pid: s.pid });
        ctx.store.remove_app(app_id);
        ctx.store.display_mut(PRIMARY_DISPLAY).remove_surface(s.pid);
        log::info!("arbiter: task-kill {app_id} pid={}", s.pid);
        Reply::ok(format!("killed app={app_id} pid={}", s.pid))
    }

    pub(crate) fn cmd_list(&mut self, ctx: &mut Ctx) -> Reply {
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

/// Flatten the internal [`Role`] to the guest-facing `app-state` token (task 92).
/// `Foreground`/`OverlayBehind` are both visible app slots → `foreground`;
/// `Overlay`/`Chrome`/`Lockscreen` are system overlay surfaces → `overlay`;
/// `Headless` (no SF surface) → `headless`; everything else → `background`.
fn role_to_state(role: Role) -> &'static str {
    match role {
        Role::Foreground | Role::OverlayBehind => "foreground",
        Role::Overlay | Role::Chrome | Role::Lockscreen => "overlay",
        Role::Headless => "headless",
        Role::Background => "background",
    }
}
