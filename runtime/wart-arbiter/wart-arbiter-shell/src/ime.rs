//! IME half (Android's InputMethodManagerService): the active IME, editor
//! attach/detach + focus, the IME-overlay split engage/clear, and `ime-*` input
//! routing. Command handlers on [`crate::ShellModule`]; the shared promote/demote/
//! reconcile orchestration lives in [`crate::shared`].

use wart_arbiter_core::{Ctx, EditorInfo, Event, Reply, PRIMARY_DISPLAY};

use crate::shared::{
    active_ime, demote_from_overlay, foreground_slot, ime_editor, overlay_engaged,
    promote_to_overlay, reconcile_overlay, visible_app,
};

/// Symmetric with wart-host/src/ime_inbound.rs::unescape_underscores.
/// Empty → "-"; otherwise spaces → "_". Task 49 step 1a wire format.
fn escape_underscores(s: &str) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.replace(' ', "_")
}

impl crate::ShellModule {
    pub(crate) fn cmd_set_ime(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
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

    pub(crate) fn cmd_ime_overlay_height(&mut self, rest: &str, ctx: &mut Ctx) -> Reply {
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

    pub(crate) fn cmd_attach_editor(&mut self, rest: &str, ctx: &mut Ctx) -> Reply {
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

    pub(crate) fn cmd_detach_editor(&mut self, rest: &str, ctx: &mut Ctx) -> Reply {
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
    pub(crate) fn cmd_ime_route(&mut self, verb: &str, rest: &str, ctx: &mut Ctx) -> Reply {
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

    pub(crate) fn cmd_overlay(&mut self, app_id: &str, ctx: &mut Ctx) -> Reply {
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

    pub(crate) fn cmd_overlay_clear(&mut self, ctx: &mut Ctx) -> Reply {
        match demote_from_overlay(ctx) {
            Some((ime_pid, behind_pid)) => Reply::ok(format!(
                "overlay-cleared ime-pid={ime_pid} repromoted-behind-pid={behind_pid}"
            )),
            None => Reply::ok("overlay-was-not-active"),
        }
    }
}
