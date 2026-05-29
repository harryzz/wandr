//! wart-arbiter — Hybrid runtime policy daemon (task 46 step 3).
//!
//! Two modes:
//!
//! - **Daemon** (`wart-arbiter --daemon`): binds
//!   `/data/local/tmp/wart-arbiter.sock`, accepts text commands
//!   from clients, dispatches to wart-host's zygote socket.
//! - **Client** (`wart-arbiter <cmd> [args]`): connects to the
//!   above socket, sends one command, prints the response.
//!
//! Commands (both arbiter↔client and arbiter→zygote use text +
//! newline-delimited, matching the AOSP-zygote precedent locked by
//! D2 in task-45):
//!
//!   launch <app-id>     — forward to zygote as LAUNCH_GUI; record in
//!                         state; auto-promote to foreground (demoting
//!                         the previous fg if any).
//!   launch-headless <id>— forward as LAUNCH (wasi:cli/command consumer)
//!   list                — print running apps + pids; current fg is
//!                         marked with [fg]; current IME marked [ime].
//!   kill <app-id>       — KILL the zygote-tracked pid for this app-id
//!   preload <app-id>    — PRELOAD on the zygote (post-install refresh)
//!   foreground <id>     — promote <id> to foreground; SIGUSR1 the
//!                         previous fg, SIGUSR2 the new fg, write
//!                         /proc/<pid>/oom_score_adj on both.
//!   set-ime <app-id>    — designate <app-id> as the active IME. Must
//!                         already be running (launch first). Future:
//!                         auto-launch + set in one go.
//!   attach-editor <pid> [input-type] [hint] [initial-text]
//!                       — focused app's host reports editor focus.
//!                         Records EditorFocus; logs the route to the
//!                         active IME. Cross-process dispatch lands
//!                         in step 2.
//!   detach-editor <pid> — reverse of attach-editor.
//!   ime-commit-text <text>          — IME → focused-app text commit.
//!   ime-send-key-event <code-point> <key-id> <action>
//!                                   — IME → focused-app key event.
//!   ime-set-composing-text <text>   — IME → composing-state update.
//!   ime-finish-composing-text       — IME → finalize composing region.
//!   ime-set-selection <start> <end> — IME → editor cursor/selection.

mod state;
mod zygote_client;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context, Result};

use crate::state::AppState;

const ARBITER_SOCK_PATH: &str = "/data/local/tmp/wart-arbiter.sock";

/// Where the running-apps state is persisted between daemon restarts.
/// Children of the zygote outlive arbiter restarts; this lets us
/// reattach to them via `kill(pid, 0)` liveness checks rather than
/// starting with an empty map every time.
const ARBITER_STATE_PATH: &str = "/data/local/tmp/wart-arbiter-state.json";

/// Panic-hook destination. The arbiter is small + mostly synchronous,
/// but if a Mutex gets poisoned or some socket I/O panics, we want a
/// trail visible on the next startup.
const ARBITER_CRASH_PATH: &str = "/data/local/tmp/wart-arbiter-crash.json";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Initialize logger up front — both daemon and client modes log
    // to logcat on Android. `--daemon` mode tags as info-level by
    // default; client mode logs are debug-level (most output goes
    // via stdout for the user).
    let max_level = if matches!(args.first(), Some(s) if s == "--daemon") {
        log::LevelFilter::Info
    } else {
        log::LevelFilter::Debug
    };
    android_logger::init_once(
        android_logger::Config::default().with_max_level(max_level),
    );

    let result = match args.first().map(|s| s.as_str()) {
        Some("--daemon") => run_daemon(),
        Some("launch") => run_client("launch", args.get(1).cloned()),
        Some("launch-headless") => run_client("launch-headless", args.get(1).cloned()),
        // Task 47 step 3c — overlay launch for IME apps.
        Some("launch-overlay") => run_client("launch-overlay", args.get(1).cloned()),
        Some("list") => run_client("list", None),
        Some("kill") => run_client("kill", args.get(1).cloned()),
        Some("preload") => run_client("preload", args.get(1).cloned()),
        Some("foreground") => run_client("foreground", args.get(1).cloned()),
        // Task 57 — launcher / home app.
        Some("set-home") => run_client("set-home", args.get(1).cloned()),
        Some("go-home")  => run_client("go-home", None),
        // Task 47 step 3c — manual overlay engage/clear.
        Some("overlay") => run_client("overlay", args.get(1).cloned()),
        Some("overlay-clear") => run_client("overlay-clear", None),
        // Task 47 step 1 — IME routing commands. The client side
        // passes the verb's tail through unchanged; the daemon
        // parses arg-shapes per command.
        Some(verb @ ("set-ime" | "attach-editor" | "detach-editor"
                    | "ime-commit-text" | "ime-send-key-event"
                    | "ime-set-composing-text" | "ime-finish-composing-text"
                    | "ime-set-selection")) => {
            run_client_multi(verb, &args[1..])
        }
        Some(other) => {
            eprintln!("wart-arbiter: unknown command: {other}");
            print_usage();
            std::process::exit(2);
        }
        None => {
            print_usage();
            std::process::exit(2);
        }
    };

    if let Err(e) = result {
        eprintln!("wart-arbiter: {e:#}");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n\
         \n\
         Daemon mode:\n\
           wart-arbiter --daemon\n\
         \n\
         Client commands (require the daemon running):\n\
           wart-arbiter launch          <app-id>\n\
           wart-arbiter launch-headless <app-id>\n\
           wart-arbiter launch-overlay  <app-id>          (IME-shaped surface, task 47 step 3c)\n\
           wart-arbiter list\n\
           wart-arbiter kill            <app-id>\n\
           wart-arbiter preload         <app-id>\n\
           wart-arbiter foreground      <app-id>\n\
           wart-arbiter set-home        <app-id>          (designate the home/launcher app; '-' clears)\n\
           wart-arbiter go-home                           (foreground the home app)\n\
           wart-arbiter overlay         <app-id>          (engage IME overlay split, task 47 step 3c)\n\
           wart-arbiter overlay-clear                     (tear it down)\n\
         \n\
         IME routing (task 47):\n\
           wart-arbiter set-ime         <app-id>\n\
           wart-arbiter attach-editor   <pid> [input-type] [hint] [initial-text]\n\
           wart-arbiter detach-editor   <pid>\n\
           wart-arbiter ime-commit-text         <text>\n\
           wart-arbiter ime-send-key-event      <code-point> <key-id> <down|up>\n\
           wart-arbiter ime-set-composing-text  <text>\n\
           wart-arbiter ime-finish-composing-text\n\
           wart-arbiter ime-set-selection       <start> <end>\n",
    );
}

// ─── Client mode ──────────────────────────────────────────────────────

fn run_client(verb: &str, arg: Option<String>) -> Result<()> {
    // Build the wire command. Verb-only commands (`list`) send just
    // the verb; arg-bearing commands send `<verb> <arg>`.
    let needs_arg = matches!(
        verb,
        "launch" | "launch-headless" | "launch-overlay"
        | "kill" | "preload" | "foreground" | "overlay" | "set-home"
    );
    let line = match (needs_arg, arg) {
        (true, Some(a))  => format!("{verb} {a}\n"),
        (true, None)     => {
            return Err(anyhow!("wart-arbiter {verb}: requires <app-id>"));
        }
        (false, _)       => format!("{verb}\n"),
    };
    send_and_print(&line)
}

/// Multi-arg client form used by the task-47 IME commands. Joins all
/// CLI args after the verb into the wire command's single-line tail.
/// `attach-editor <pid> [input-type] [hint] [initial-text]` becomes
/// `attach-editor <pid> <input-type> <hint> <initial-text>\n`.
///
/// Per-arg spaces are NOT supported by this minimal serializer — IME
/// commit-text strings with spaces work because everything after the
/// verb is concatenated with single spaces and the daemon's parser
/// splits on the first N spaces only (so trailing args containing
/// spaces survive). See `parse_ime_tail_*` on the daemon side.
fn run_client_multi(verb: &str, rest: &[String]) -> Result<()> {
    let mut line = verb.to_string();
    for arg in rest {
        line.push(' ');
        line.push_str(arg);
    }
    line.push('\n');
    send_and_print(&line)
}

fn send_and_print(line: &str) -> Result<()> {
    let mut stream = UnixStream::connect(ARBITER_SOCK_PATH)
        .with_context(|| format!("connect {ARBITER_SOCK_PATH} — is the daemon running?"))?;
    stream.write_all(line.as_bytes())?;
    stream.flush().ok();
    // Half-close write so the server's read_to_string-like loop sees
    // EOF cleanly for multi-line replies (LIST). The Linux UnixStream
    // shutdown(Write) is the closest equivalent to "I'm done sending."
    stream.shutdown(std::net::Shutdown::Write).ok();

    let reader = BufReader::new(&stream);
    for line in reader.lines() {
        let line = line?;
        println!("{line}");
    }
    Ok(())
}

// ─── Daemon mode ──────────────────────────────────────────────────────

fn run_daemon() -> Result<()> {
    log::info!("wart-arbiter: starting daemon — sock={ARBITER_SOCK_PATH}");
    log::info!("wart-arbiter: zygote sock = {}", zygote_client::zygote_sock_path());

    // Task 46 crash-marker — panic-hook drops a JSON file on the way
    // out, drained + logged on next startup.
    install_panic_hook();
    drain_prior_crash_marker();

    // Task 46 crash-marker — restore in-memory state from disk. Each
    // persisted pid is liveness-checked via `kill(pid, 0)`; survivors
    // are re-inserted, dead ones are dropped + logged.
    match state::restore_from(Path::new(ARBITER_STATE_PATH)) {
        Ok((alive, dead)) => {
            log::info!(
                "wart-arbiter: state restore — {alive} alive app(s) re-attached, {dead} dropped"
            );
        }
        Err(e) => {
            log::warn!("wart-arbiter: state restore failed: {e:#} (continuing with empty state)");
        }
    }

    let sock_path = Path::new(ARBITER_SOCK_PATH);
    if sock_path.exists() {
        std::fs::remove_file(sock_path)
            .with_context(|| format!("removing stale socket {ARBITER_SOCK_PATH}"))?;
    }
    let listener = UnixListener::bind(ARBITER_SOCK_PATH)
        .with_context(|| format!("UnixListener::bind {ARBITER_SOCK_PATH}"))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(
        ARBITER_SOCK_PATH,
        std::fs::Permissions::from_mode(0o666),
    );
    log::info!("wart-arbiter: listening");

    // Task 54 — start the death watchers. The subscriber thread is the
    // primary (event-driven) path; the poller is the ≤5 s backstop that
    // also covers a dropped subscriber connection or a crashed zygote.
    spawn_death_watchers();

    // Task 57 — boot to home. If a home app was designated in a previous
    // session (restored above), foreground it now (launching it if it
    // didn't survive the restart) so the device comes up to a usable
    // home screen with no adb command needed. No-op if no home is set
    // (the pre-launcher default). The zygote is started before the
    // arbiter by run-hybrid-stack.sh, so `launch_gui` can reach it.
    {
        let _g = arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());
        if state::current_home().is_some() {
            log::info!("wart-arbiter: boot — foregrounding home app");
            ensure_home_foreground();
            if let Err(e) = state::save_to(Path::new(ARBITER_STATE_PATH)) {
                log::warn!("wart-arbiter: boot home-foreground state save failed: {e:#}");
            }
        }
    }

    loop {
        let (stream, _addr) = match listener.accept() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("wart-arbiter: accept failed: {e}");
                continue;
            }
        };
        if let Err(e) = handle_client(stream) {
            log::warn!("wart-arbiter: client error: {e:#}");
        }
    }
}

fn handle_client(mut stream: UnixStream) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Err(anyhow!("client closed without sending a command"));
    }
    let line = line.trim_end_matches('\n').trim_end_matches('\r');
    log::info!("wart-arbiter: cmd={line:?}");

    let (verb, rest) = match line.split_once(' ') {
        Some((v, r)) => (v, r.trim().to_string()),
        None => (line, String::new()),
    };

    // Task 54 — serialize command handling against the death-notification
    // / polling watcher threads. The accept loop is single-threaded so
    // commands were implicitly serialized before; the new background
    // threads (subscriber + poller) also mutate state + send signals, so
    // one coarse process-wide lock keeps the two paths from interleaving
    // a state mutation with a signal cascade. Low frequency — fine.
    let _guard = arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());

    let result = match verb {
        "launch"          => cmd_launch(&mut stream, &rest, LaunchKind::Gui),
        "launch-headless" => cmd_launch(&mut stream, &rest, LaunchKind::Headless),
        // Task 47 step 3c — overlay launch (e.g. an IME app). Same
        // shape as `launch` but the child gets a bottom-strip
        // overlay SurfaceControl. Does NOT auto-promote to fg here
        // — that's the `overlay` command's job, or the auto-tie in
        // `attach-editor` when an editor focuses.
        "launch-overlay"  => cmd_launch(&mut stream, &rest, LaunchKind::GuiOverlay),
        "kill"            => cmd_kill(&mut stream, &rest),
        "list"            => cmd_list(&mut stream),
        "preload"         => cmd_preload(&mut stream, &rest),
        "foreground"      => cmd_foreground(&mut stream, &rest),
        // Task 57 — launcher / home app.
        "set-home"        => cmd_set_home(&mut stream, &rest),
        "go-home"         => cmd_go_home(&mut stream),
        // Task 47 step 3c — manual overlay handles.
        "overlay"         => cmd_overlay(&mut stream, &rest),
        "overlay-clear"   => cmd_overlay_clear(&mut stream),
        // Task 47 step 1 — IME routing. State maintenance + logging
        // only; cross-process delivery lands in step 2.
        "set-ime"                    => cmd_set_ime(&mut stream, &rest),
        "attach-editor"              => cmd_attach_editor(&mut stream, &rest),
        "detach-editor"              => cmd_detach_editor(&mut stream, &rest),
        "ime-commit-text"            => cmd_ime_route(&mut stream, "commit-text", &rest),
        "ime-send-key-event"         => cmd_ime_route(&mut stream, "send-key-event", &rest),
        "ime-set-composing-text"     => cmd_ime_route(&mut stream, "set-composing-text", &rest),
        "ime-finish-composing-text"  => cmd_ime_route(&mut stream, "finish-composing-text", &rest),
        "ime-set-selection"          => cmd_ime_route(&mut stream, "set-selection", &rest),
        other => {
            writeln!(stream, "ERR unknown-command {other}")?;
            Ok(())
        }
    };

    // Task 46 crash-marker — persist state after every command. The
    // mutating commands (launch / kill / foreground) need this so
    // post-restart we see the right apps + fg; non-mutating (list /
    // preload) save anyway because it's cheap (one ~1 KB write) and
    // the simpler code path is worth the micro-cost.
    if let Err(e) = state::save_to(Path::new(ARBITER_STATE_PATH)) {
        log::warn!("wart-arbiter: state save failed: {e:#}");
    }

    result
}

// ─── IME routing (task 47 step 1) ──────────────────────────────────────
//
// Step 1 scope: state maintenance + structured logging of routing
// intent. Cross-process delivery (per-host control sockets, WIT-proxy
// invocation) is step 2+. The commands here are how the focused-app's
// host (via skiko's WasiInputMethod adapter, step 2) and the IME
// app's host will eventually talk to the arbiter.

fn cmd_set_ime(stream: &mut UnixStream, rest: &str) -> Result<()> {
    let app_id = rest.trim();
    if app_id.is_empty() {
        writeln!(stream, "ERR set-ime-empty-app-id")?;
        return Ok(());
    }
    // Special form: "set-ime -" clears the active IME without naming a
    // replacement. Useful for tests + future "stop using any IME" flow.
    if app_id == "-" {
        let prev = state::set_active_ime(None);
        let prev_id = prev.map(|p| p.app_id).unwrap_or_else(|| "(none)".to_string());
        writeln!(stream, "OK cleared prev={prev_id}")?;
        log::info!("arbiter: cleared active IME (prev={prev_id})");
        return Ok(());
    }
    let Some(s) = state::get(app_id) else {
        writeln!(stream, "ERR not-running {app_id} (launch first)")?;
        return Ok(());
    };
    let prev = state::set_active_ime(Some(state::ActiveIme {
        app_id: s.app_id.clone(),
        pid: s.pid,
    }));
    let prev_id = prev.map(|p| p.app_id).unwrap_or_else(|| "(none)".to_string());
    writeln!(stream, "OK ime={app_id} pid={} prev={prev_id}", s.pid)?;
    log::info!(
        "arbiter: set active IME app={app_id} pid={} (prev={prev_id})",
        s.pid
    );
    Ok(())
}

fn cmd_attach_editor(stream: &mut UnixStream, rest: &str) -> Result<()> {
    // Wire shape: `attach-editor <pid> [input-type] [hint] [initial-text]`.
    // input-type defaults to `text`; hint + initial-text default to "".
    // The CLI doesn't currently support spaces in hint / initial-text;
    // that comes through skiko in step 2 over the per-host control
    // socket where we control the serialization.
    let mut parts = rest.splitn(4, ' ');
    let Some(pid_s) = parts.next() else {
        writeln!(stream, "ERR attach-editor-missing-pid")?;
        return Ok(());
    };
    let Ok(pid) = pid_s.parse::<i32>() else {
        writeln!(stream, "ERR attach-editor-bad-pid {pid_s}")?;
        return Ok(());
    };
    // Validate the pid is one of our tracked apps.
    let owner = state::snapshot().into_iter().find(|a| a.pid == pid);
    let Some(owner) = owner else {
        writeln!(stream, "ERR attach-editor-unknown-pid {pid}")?;
        return Ok(());
    };

    let input_type = parts.next().unwrap_or("text").to_string();
    let hint = parts.next().unwrap_or("").to_string();
    let initial_text = parts.next().unwrap_or("").to_string();

    let info = state::EditorInfo {
        input_type: input_type.clone(),
        hint: hint.clone(),
        initial_text: initial_text.clone(),
        initial_selection_start: 0,
        initial_selection_end: 0,
    };
    let prev = state::set_editor_focus(Some(state::EditorFocus {
        pid,
        editor_info: info,
    }));
    let prev_pid = prev.map(|p| p.pid).map(|p| p.to_string()).unwrap_or_else(|| "-".to_string());

    let ime = state::current_active_ime();
    let ime_dest = ime
        .as_ref()
        .map(|i| format!("{} (pid={})", i.app_id, i.pid))
        .unwrap_or_else(|| "(no active IME — set-ime first)".to_string());

    // Task 47 step 3c — auto-tie. If there's an active IME and its
    // pid differs from the focused editor's, engage the overlay
    // split: IME → foreground, editor → OverlayBehind. The IME's SF
    // surface flips visible at MAX-layer; the editor stays visible
    // at layer 0 (lifecycle Resumed so the cursor keeps blinking).
    // Skip if the IME pid IS the editor pid (same process owning
    // both — implausible in practice but defensive).
    let auto_overlay = match &ime {
        Some(i) if i.pid != pid => {
            let already = state::current_overlay()
                .map(|ov| ov.ime_pid == i.pid && ov.behind_pid == pid)
                .unwrap_or(false);
            if !already {
                promote_to_overlay(&i.app_id, i.pid, Some(pid));
                true
            } else {
                false
            }
        }
        _ => false,
    };

    // Task 49 step 1a — deliver editor-attached to the IME's wart-host
    // via its per-host control socket. The IME's render-loop drain
    // will call into the guest's exported `war:ime/ime.on-editor-attached`
    // (step 1b). Fire-and-forget; failure to deliver doesn't roll back
    // the attach-editor state on the arbiter side.
    let delivered = match &ime {
        Some(i) => {
            let host_sock = format!("/data/local/tmp/wart-host-{}.sock", i.pid);
            let line = format!(
                "editor-attached {input_type} {hint_esc} {text_esc} {sel_start} {sel_end}\n",
                hint_esc   = escape_underscores(&hint),
                text_esc   = escape_underscores(&initial_text),
                sel_start  = 0u32,
                sel_end    = 0u32,
            );
            match deliver_to_host(&host_sock, &line) {
                Ok(_) => true,
                Err(e) => {
                    log::warn!(
                        "arbiter: editor-attached delivery to {host_sock} failed: {e:#}"
                    );
                    false
                }
            }
        }
        None => false,
    };

    writeln!(
        stream,
        "OK attached editor pid={pid} app={} input-type={input_type} \
         prev-pid={prev_pid} route→{ime_dest} overlay={overlay} delivered={delivered}",
        owner.app_id,
        overlay = if auto_overlay { "engaged" } else { "skipped" },
    )?;
    log::info!(
        "arbiter: attach-editor pid={pid} app={} input-type={input_type} hint={hint:?} \
         initial-text-len={} → route to {ime_dest} auto-overlay={auto_overlay} \
         delivered={delivered}",
        owner.app_id,
        initial_text.len(),
    );
    Ok(())
}

/// Symmetric with wart-host/src/ime_inbound.rs::unescape_underscores.
/// Empty → "-"; otherwise spaces → "_". Task 49 step 1a wire format.
fn escape_underscores(s: &str) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.replace(' ', "_")
}

fn cmd_detach_editor(stream: &mut UnixStream, rest: &str) -> Result<()> {
    let pid_s = rest.trim();
    if pid_s.is_empty() {
        writeln!(stream, "ERR detach-editor-missing-pid")?;
        return Ok(());
    }
    let Ok(pid) = pid_s.parse::<i32>() else {
        writeln!(stream, "ERR detach-editor-bad-pid {pid_s}")?;
        return Ok(());
    };
    let prev = state::current_editor_focus();
    let was_focused = prev.as_ref().map(|p| p.pid) == Some(pid);
    if was_focused {
        let _ = state::set_editor_focus(None);
        let ime = state::current_active_ime();
        let ime_dest = ime
            .as_ref()
            .map(|i| format!("{} (pid={})", i.app_id, i.pid))
            .unwrap_or_else(|| "(no active IME)".to_string());
        // Task 47 step 3c — auto-tie. If we engaged the overlay
        // split for this editor on attach, tear it down now. The
        // IME goes back to Background; the editor (this pid) gets
        // repromoted to Foreground.
        let cleared = match state::current_overlay() {
            Some(ov) if ov.behind_pid == pid => demote_from_overlay().is_some(),
            _ => false,
        };
        // Task 49 step 1a — deliver editor-detached to the IME's
        // wart-host via its per-host control socket. Symmetric with
        // cmd_attach_editor's delivery above. Fire-and-forget.
        let delivered = match &ime {
            Some(i) => {
                let host_sock = format!("/data/local/tmp/wart-host-{}.sock", i.pid);
                match deliver_to_host(&host_sock, "editor-detached\n") {
                    Ok(_) => true,
                    Err(e) => {
                        log::warn!(
                            "arbiter: editor-detached delivery to {host_sock} failed: {e:#}"
                        );
                        false
                    }
                }
            }
            None => false,
        };
        writeln!(
            stream,
            "OK detached pid={pid} route→{ime_dest} overlay={overlay} delivered={delivered}",
            overlay = if cleared { "cleared" } else { "skipped" },
        )?;
        log::info!(
            "arbiter: detach-editor pid={pid} → route to {ime_dest} auto-overlay-clear={cleared} \
             delivered={delivered}"
        );
    } else {
        writeln!(stream, "OK no-op pid={pid} (not focused)")?;
        log::info!("arbiter: detach-editor pid={pid} — was not focused, no-op");
    }
    Ok(())
}

/// Shared backend for the five `ime-*` commands: route IME-side input
/// to the currently-focused editor's app.
///
/// Task 47 step 3a — `send-key-event` actually delivers via the
/// focused app's per-host control socket
/// (`/data/local/tmp/wart-host-<focus.pid>.sock`). Other ime-* verbs
/// (commit-text / set-composing-text / finish-composing-text /
/// set-selection) still just log the intent; they need new WIT
/// exports on the editor-bearing-app side, and that wiring is the
/// step 3b work (alongside the first-party war.ime.keyboard).
fn cmd_ime_route(stream: &mut UnixStream, verb: &str, rest: &str) -> Result<()> {
    let Some(focus) = state::current_editor_focus() else {
        writeln!(stream, "ERR no-focused-editor")?;
        return Ok(());
    };

    if verb == "send-key-event" {
        // Wire format from the IME's perspective:
        //   ime-send-key-event <code-point> <key-id> <down|up>
        // The arbiter forwards it to the per-host control socket as:
        //   key-event <code-point> <key-id> <down|up>
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() != 3 {
            writeln!(stream, "ERR send-key-event-bad-args: expected <code-point> <key-id> <down|up>")?;
            return Ok(());
        }
        let host_sock = format!("/data/local/tmp/wart-host-{}.sock", focus.pid);
        let line = format!("key-event {} {} {}\n", parts[0], parts[1], parts[2]);
        match deliver_to_host(&host_sock, &line) {
            Ok(()) => {
                writeln!(stream, "OK route→pid={} (key-event {} {} {})",
                         focus.pid, parts[0], parts[1], parts[2])?;
                log::info!(
                    "arbiter: ime-send-key-event → pid={} ({} {} {}) delivered via {host_sock}",
                    focus.pid, parts[0], parts[1], parts[2]
                );
            }
            Err(e) => {
                writeln!(stream, "ERR deliver-failed {host_sock}: {e:#}")?;
                log::warn!(
                    "arbiter: ime-send-key-event → pid={} failed: {e:#} \
                     (host socket missing? guest in fg but ime_inbound listener not spawned?)",
                    focus.pid
                );
            }
        }
        return Ok(());
    }

    // commit-text / set-composing-text / finish-composing-text /
    // set-selection — log only, await step 3b WIT exports.
    writeln!(
        stream,
        "OK route→pid={} (input-type={}) {verb} args={:?} [step 3b — log only]",
        focus.pid, focus.editor_info.input_type, rest,
    )?;
    log::info!(
        "arbiter: ime-{verb} → editor pid={} app-input-type={} args={:?} \
         (step 3b delivers via new editor-side WIT exports)",
        focus.pid,
        focus.editor_info.input_type,
        rest,
    );
    Ok(())
}

/// One-shot write to a wart-host child's control socket. Matches the
/// one-shot pattern used elsewhere — open, write, drain reply, close.
fn deliver_to_host(sock_path: &str, line: &str) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let mut stream = UnixStream::connect(sock_path)?;
    stream.write_all(line.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    // No reply expected; drain anyway in case the host echoes.
    let mut buf = [0u8; 64];
    let _ = stream.read(&mut buf);
    Ok(())
}

// ─── Crash marker (task 46 crash-marker) ──────────────────────────────

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        let ts = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let msg = format!("{info}")
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        let body = format!("{{\"ts\":{ts},\"panic\":\"{msg}\"}}\n");
        let _ = std::fs::write(ARBITER_CRASH_PATH, body);
    }));
}

fn drain_prior_crash_marker() {
    let p = Path::new(ARBITER_CRASH_PATH);
    if !p.exists() { return; }
    match std::fs::read_to_string(p) {
        Ok(s) => log::error!("wart-arbiter: prior run crashed — {}", s.trim()),
        Err(e) => log::warn!("wart-arbiter: prior crash marker unreadable: {e}"),
    }
    let _ = std::fs::remove_file(p);
}

// ─── Death notification + socket cleanup (task 54) ────────────────────

/// Process-wide serialization lock. Held by `handle_client` for the
/// duration of a command's state mutation + signal cascade + persist,
/// and by `handle_child_exit` for the same. Keeps the accept-loop
/// thread and the two death-watcher threads from interleaving.
fn arbiter_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Per-host control socket path for a given child pid. Symmetric with
/// `wart-host/src/ime_inbound.rs::spawn_listener`.
fn host_sock_path(pid: i32) -> String {
    format!("/data/local/tmp/wart-host-{pid}.sock")
}

/// Unlink a dead child's per-host control socket (RC2). Ignore
/// not-found — the child may have unlinked it itself on a graceful
/// exit, or it may never have bound one (headless consumer).
fn remove_host_socket(pid: i32) {
    let path = host_sock_path(pid);
    match std::fs::remove_file(&path) {
        Ok(()) => log::info!("arbiter: removed orphaned host socket {path}"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("arbiter: remove host socket {path} failed: {e}"),
    }
}

/// Single cleanup entry for a dead child pid. Invoked by BOTH the
/// event-driven subscriber thread and the polling backstop. Takes the
/// coarse `arbiter_lock` so it never races a command handler.
///
/// Steps:
///   1. Look the app up by pid (under the lock). Untracked → just sweep
///      its socket + return (covers a race where a command already
///      removed it, or a non-app pid the zygote reaped).
///   2. If the pid is part of an active IME overlay split, run the
///      `cmd_overlay_clear` path (`demote_from_overlay`) to hide the
///      IME + repromote the surviving side. In the editor-died case the
///      stray SIGUSR2 to the dead behind-pid is harmless (ESRCH).
///   3. `state::remove` cascades the remaining state teardown
///      (foreground / active-IME / editor-focus / overlay pointers).
///   4. Unlink the orphaned per-host control socket.
///   5. Persist + log.
fn handle_child_exit(pid: i32, detail: &str) {
    let _guard = arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());

    let Some(app) = state::snapshot().into_iter().find(|a| a.pid == pid) else {
        // Untracked or already cleaned. Still sweep a possibly-orphaned
        // socket for this pid in case it bound one before dying.
        remove_host_socket(pid);
        log::debug!("arbiter: on_child_exit pid={pid} ({detail}) — not tracked, socket swept");
        return;
    };

    log::info!("arbiter: on_child_exit pid={pid} app={} ({detail})", app.app_id);

    // Overlay teardown with signal semantics (the `cmd_overlay_clear`
    // path). Only fires when this pid was either side of an active split.
    if let Some(ov) = state::current_overlay() {
        if ov.ime_pid == pid || ov.behind_pid == pid {
            let cleared = demote_from_overlay();
            log::info!(
                "arbiter: on_child_exit pid={pid} tore down overlay split (cleared={:?})",
                cleared.is_some(),
            );
        }
    }

    // State teardown (fg / active-IME / editor-focus / overlay pointers).
    state::remove(&app.app_id);

    // Socket cleanup (RC2).
    remove_host_socket(pid);

    // Task 57 — never leave the screen empty. If the dead app was the
    // foreground (the overlay teardown + `state::remove` above left fg
    // cleared) and a home app is designated, bring home forward —
    // foregrounding the still-alive backgrounded launcher, or relaunching
    // it if the launcher itself was what died. If a behind-app was
    // repromoted by `demote_from_overlay`, fg is non-empty and this is a
    // no-op.
    if state::current_foreground().is_none() && state::current_home().is_some() {
        log::info!("arbiter: foreground empty after pid {pid} exit — falling back to home");
        ensure_home_foreground();
    }

    if let Err(e) = state::save_to(Path::new(ARBITER_STATE_PATH)) {
        log::warn!("arbiter: on_child_exit state save failed: {e:#}");
    }
    log::info!("arbiter: on_child_exit pid={pid} app={} — cleaned up", app.app_id);
}

/// Spawn the two death-watcher threads (task 54).
///
/// - **Subscriber** (primary): opens a long-lived `SUBSCRIBE_EXITS`
///   connection to the zygote and reads `EXITED <pid> <detail>` push
///   lines. Reconnects with a short backoff if the connection drops
///   (e.g. the zygote restarted).
/// - **Poller** (backstop): every 5 s, liveness-probes every tracked
///   pid and cleans up any that died. Covers a dropped subscriber link
///   and the zygote-crashed-mid-session case.
fn spawn_death_watchers() {
    // Subscriber thread.
    std::thread::Builder::new()
        .name("arbiter-exit-subscriber".into())
        .spawn(|| loop {
            match zygote_client::subscribe_exits() {
                Ok(stream) => {
                    log::info!("arbiter: subscribed to zygote exit notifications");
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        let line = line.trim();
                        if let Some(rest) = line.strip_prefix("EXITED ") {
                            let mut it = rest.splitn(2, ' ');
                            let pid = it.next().and_then(|s| s.parse::<i32>().ok());
                            let detail = it.next().unwrap_or("").to_string();
                            match pid {
                                Some(pid) => handle_child_exit(pid, &detail),
                                None => log::warn!("arbiter: malformed EXITED line: {line:?}"),
                            }
                        } else if !line.is_empty() {
                            log::warn!("arbiter: unexpected zygote push: {line:?}");
                        }
                    }
                    log::warn!("arbiter: exit-subscriber connection closed; reconnecting");
                }
                Err(e) => {
                    log::warn!("arbiter: exit-subscribe failed: {e:#}; retrying");
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        })
        .expect("spawn arbiter-exit-subscriber thread");

    // Polling backstop thread.
    std::thread::Builder::new()
        .name("arbiter-exit-poller".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_secs(5));
            for app in state::snapshot() {
                if !state::pid_alive(app.pid) {
                    handle_child_exit(app.pid, "poll-detected-dead");
                }
            }
        })
        .expect("spawn arbiter-exit-poller thread");
}

// ─── Policy helpers (task 46 step 4) ─────────────────────────────────

/// `/proc/<pid>/oom_score_adj` magic numbers. Foreground gets the
/// gentlest "killable but not first." Background apps get pushed
/// toward the front of the OOM kill queue so the system reclaims
/// them under pressure before reaching the foreground.
const OOM_FG: i32 = 0;
const OOM_BG: i32 = 500;

fn write_oom_score(pid: i32, value: i32) {
    let path = format!("/proc/{pid}/oom_score_adj");
    match std::fs::write(&path, format!("{value}\n")) {
        Ok(()) => log::info!("arbiter: wrote {path} = {value}"),
        Err(e) => log::warn!("arbiter: write {path} = {value} failed: {e}"),
    }
}

/// Send SIGUSR1 (→Background) or SIGUSR2 (→Foreground) to one of our
/// tracked child pids. The child's `app_role` handler stores the new
/// role atomically; the next frame in its render loop reacts.
fn send_role_signal(pid: i32, foreground: bool) {
    let sig = if foreground { libc::SIGUSR2 } else { libc::SIGUSR1 };
    let r = unsafe { libc::kill(pid, sig) };
    if r != 0 {
        log::warn!(
            "arbiter: kill({pid}, sig={sig}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Task 47 step 3c — send `SIGRTMIN+1` to flip a child into the
/// `OverlayBehind` role: visible, layered below the IME overlay,
/// lifecycle stays `Resumed`. The receiving wart-host's
/// `app_role::role_handler` stores the new role atomically; the next
/// frame in its render loop reacts.
fn send_overlay_behind_signal(pid: i32) {
    let sig = libc::SIGRTMIN() + 1;
    let r = unsafe { libc::kill(pid, sig) };
    if r != 0 {
        log::warn!(
            "arbiter: kill({pid}, sig={sig}) (overlay-behind) failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Task 47 step 3c — promote `(ime_app_id, ime_pid)` to be the
/// foreground overlay and demote the prior foreground app to
/// `OverlayBehind` (visible, layer 0, `Resumed`). Returns the pid of
/// the demoted app so callers can store it in `OverlayState.behind_pid`.
///
/// `behind_pid_hint` is a fallback when there's no current foreground
/// (e.g. the editor process was launched headless or got cleared) —
/// callers usually pass the focused editor's pid here.
fn promote_to_overlay(ime_app_id: &str, ime_pid: i32, behind_pid_hint: Option<i32>) -> Option<i32> {
    // Demote whoever is currently foreground. Use OverlayBehind, not
    // Background — the editor needs to keep rendering text mutations.
    let demoted = if let Some((prev_id, prev_pid)) = state::set_foreground(Some(ime_app_id)) {
        log::info!(
            "arbiter: overlay-demoting prior foreground app={prev_id} pid={prev_pid}"
        );
        send_overlay_behind_signal(prev_pid);
        // OOM score: behind-app is still Resumed but visually demoted —
        // keep it at fg priority so the IME isn't holding the OOM
        // killer's attention longer than the editor.
        write_oom_score(prev_pid, OOM_FG);
        Some(prev_pid)
    } else {
        behind_pid_hint
    };
    log::info!("arbiter: overlay-promoting IME app={ime_app_id} pid={ime_pid}");
    send_role_signal(ime_pid, /*foreground=*/ true);
    write_oom_score(ime_pid, OOM_FG);
    let behind_pid = demoted.unwrap_or(0);
    let _ = state::set_overlay(Some(state::OverlayState {
        ime_pid,
        behind_pid,
    }));
    demoted
}

/// Task 47 step 3c — tear down the IME overlay split: demote the IME
/// to `Background` and repromote the behind-app to `Foreground`. Used
/// by `cmd_detach_editor` (auto-tied) and `cmd_overlay_clear` (manual).
/// Returns the (ime_pid, behind_pid) pair that was active, if any.
fn demote_from_overlay() -> Option<(i32, i32)> {
    let prev = state::set_overlay(None)?;
    log::info!(
        "arbiter: overlay-clearing ime_pid={} behind_pid={}",
        prev.ime_pid, prev.behind_pid
    );
    send_role_signal(prev.ime_pid, /*foreground=*/ false);
    write_oom_score(prev.ime_pid, OOM_BG);
    if prev.behind_pid > 0 {
        // Find the app_id matching this pid so set_foreground points
        // at the right app. If the behind app was killed in the
        // meantime, this no-ops cleanly.
        let app = state::snapshot().into_iter().find(|a| a.pid == prev.behind_pid);
        if let Some(app) = app {
            let _ = state::set_foreground(Some(&app.app_id));
            send_role_signal(prev.behind_pid, /*foreground=*/ true);
            write_oom_score(prev.behind_pid, OOM_FG);
        } else {
            log::warn!(
                "arbiter: overlay clear — behind pid={} no longer tracked, fg left unset",
                prev.behind_pid
            );
            let _ = state::set_foreground(None);
        }
    }
    Some((prev.ime_pid, prev.behind_pid))
}

/// Demote whoever is currently foreground (if anyone) and promote the
/// given (app_id, pid) to foreground. Idempotent if the target is
/// already foreground.
fn promote_to_foreground(app_id: &str, pid: i32) {
    // set_foreground returns the previously-foreground (id, pid)
    // pair if there was one different from this app_id. The state
    // module has updated the fg slot before returning.
    if let Some((prev_id, prev_pid)) = state::set_foreground(Some(app_id)) {
        log::info!(
            "arbiter: demoting prior foreground app={prev_id} pid={prev_pid}"
        );
        send_role_signal(prev_pid, /*foreground=*/ false);
        write_oom_score(prev_pid, OOM_BG);
    }
    log::info!("arbiter: promoting foreground app={app_id} pid={pid}");
    send_role_signal(pid, /*foreground=*/ true);
    write_oom_score(pid, OOM_FG);
}

/// Task 57 — bring the designated home app to the foreground, launching
/// it first if it isn't running. No-op if no home is set or home is
/// already foreground.
///
/// **The caller must already hold `arbiter_lock`** (this mutates state +
/// sends signals + may launch). Called from boot, `set-home`, `go-home`,
/// and the fall-back in `handle_child_exit` — all of which hold the lock.
fn ensure_home_foreground() {
    let Some(home) = state::current_home() else { return };
    if state::current_foreground().as_deref() == Some(home.as_str()) {
        return;
    }
    // Already running (survived as a backgrounded process) → just promote.
    if let Some(s) = state::get(&home) {
        log::info!("arbiter: home {home} already running (pid {}) — foregrounding", s.pid);
        promote_to_foreground(&home, s.pid);
        return;
    }
    // Not running — launch it as a fullscreen GUI app, then promote.
    match zygote_client::launch_gui(&home) {
        Ok(pid) => {
            state::insert(AppState {
                app_id: home.clone(),
                pid,
                launched_at: SystemTime::now(),
                launched_mono: Instant::now(),
            });
            promote_to_foreground(&home, pid);
            log::info!("arbiter: launched home app {home} → pid {pid}");
        }
        Err(e) => log::warn!("arbiter: launch home app {home} failed: {e:#}"),
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum LaunchKind {
    Gui,
    Headless,
    /// Task 47 step 3c — overlay surface (e.g. IME apps).
    GuiOverlay,
}

fn cmd_launch(stream: &mut UnixStream, app_id: &str, kind: LaunchKind) -> Result<()> {
    if app_id.is_empty() && kind != LaunchKind::Gui {
        writeln!(stream, "ERR launch-empty-app-id")?;
        return Ok(());
    }
    let result = match kind {
        LaunchKind::Gui         => zygote_client::launch_gui(app_id),
        LaunchKind::GuiOverlay  => zygote_client::launch_gui_overlay(app_id),
        LaunchKind::Headless    => zygote_client::launch(app_id),
    };
    match result {
        Ok(pid) => {
            let key = if app_id.is_empty() { "<dev-cwasm>".to_string() } else { app_id.to_string() };
            state::insert(AppState {
                app_id: key.clone(),
                pid,
                launched_at: SystemTime::now(),
                launched_mono: Instant::now(),
            });
            // Task 46 step 4 — GUI launches auto-promote to
            // foreground (matches Android's "new activity comes
            // forward" expectation). Headless launches don't have
            // an SF surface so the foreground concept doesn't
            // apply; skip promotion for them. Overlay launches
            // (task 47 step 3c) don't auto-promote either — the
            // overlay is engaged explicitly via `overlay` /
            // `attach-editor` so the IME stays hidden+layered-low
            // until an editor focuses.
            if kind == LaunchKind::Gui {
                promote_to_foreground(&key, pid);
            } else if kind == LaunchKind::GuiOverlay {
                // Start the overlay HIDDEN — its default Foreground role
                // would otherwise show it immediately (e.g. the IME
                // keyboard visible on the home screen). The auto-tie in
                // cmd_attach_editor (promote_to_overlay) shows it when an
                // editor actually focuses; detach hides it again.
                send_role_signal(pid, /*foreground=*/ false);
            }
            writeln!(stream, "OK pid={pid} app={key}")?;
            log::info!("wart-arbiter: launched {key} → pid {pid} kind={kind:?}");
            Ok(())
        }
        Err(e) => {
            writeln!(stream, "ERR launch-failed {e:#}")?;
            log::warn!("wart-arbiter: launch {app_id} failed: {e:#}");
            Ok(())
        }
    }
}

fn cmd_foreground(stream: &mut UnixStream, app_id: &str) -> Result<()> {
    if app_id.is_empty() {
        writeln!(stream, "ERR foreground-empty-app-id")?;
        return Ok(());
    }
    let Some(s) = state::get(app_id) else {
        writeln!(stream, "ERR not-tracked {app_id}")?;
        return Ok(());
    };
    let prev = state::current_foreground();
    promote_to_foreground(&s.app_id, s.pid);
    let prev_str = prev.unwrap_or_else(|| "(none)".to_string());
    writeln!(stream, "OK fg={app_id} prev={prev_str} pid={pid}", pid = s.pid)?;
    Ok(())
}

/// Task 57 — designate the home/launcher app. `'-'` clears it. Takes
/// effect immediately: the home app is brought to the foreground (and
/// launched if not running), so `set-home <id>` from the boot script
/// both records the designation and shows the launcher.
fn cmd_set_home(stream: &mut UnixStream, rest: &str) -> Result<()> {
    let app_id = rest.trim();
    if app_id.is_empty() {
        writeln!(stream, "ERR set-home-empty-app-id")?;
        return Ok(());
    }
    if app_id == "-" {
        let prev = state::set_home(None);
        let prev_s = prev.unwrap_or_else(|| "(none)".to_string());
        writeln!(stream, "OK home cleared prev={prev_s}")?;
        log::info!("arbiter: home app cleared (prev={prev_s})");
        return Ok(());
    }
    let prev = state::set_home(Some(app_id));
    let prev_s = prev.unwrap_or_else(|| "(none)".to_string());
    ensure_home_foreground();
    writeln!(stream, "OK home={app_id} prev={prev_s}")?;
    log::info!("arbiter: home app = {app_id} (prev={prev_s})");
    Ok(())
}

/// Task 57 — foreground the designated home app (launching it if needed).
fn cmd_go_home(stream: &mut UnixStream) -> Result<()> {
    match state::current_home() {
        Some(home) => {
            ensure_home_foreground();
            writeln!(stream, "OK go-home={home}")?;
            Ok(())
        }
        None => {
            writeln!(stream, "ERR no-home-set (use: set-home <app-id>)")?;
            Ok(())
        }
    }
}

/// Task 47 step 3c — manually engage the overlay split. Promotes
/// `app_id` (typically an IME) as foreground overlay; demotes the
/// prior foreground to `OverlayBehind`. The auto-tied path inside
/// `cmd_attach_editor` fires the same flow when an editor focuses and
/// an `ActiveIme` is set; this manual handle is for testing /
/// arbiter-driven flows that don't go through attach-editor.
fn cmd_overlay(stream: &mut UnixStream, app_id: &str) -> Result<()> {
    if app_id.is_empty() {
        writeln!(stream, "ERR overlay-empty-app-id")?;
        return Ok(());
    }
    let Some(ime) = state::get(app_id) else {
        writeln!(stream, "ERR overlay-not-tracked {app_id}")?;
        return Ok(());
    };
    let prev_fg = state::current_foreground();
    // Find the prior fg's pid (if any) for the behind_pid_hint. The
    // promote_to_overlay helper handles the case where there is none.
    let behind_hint = prev_fg
        .as_ref()
        .and_then(|id| state::get(id))
        .map(|s| s.pid);
    let demoted = promote_to_overlay(&ime.app_id, ime.pid, behind_hint);
    let prev_str = prev_fg.unwrap_or_else(|| "(none)".to_string());
    let demoted_str = demoted
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".to_string());
    writeln!(
        stream,
        "OK overlay={app_id} pid={ime_pid} prev-fg={prev_str} behind-pid={demoted_str}",
        ime_pid = ime.pid,
    )?;
    Ok(())
}

/// Task 47 step 3c — tear down the overlay split. Demotes the IME
/// (Background) and repromotes the behind-app (Foreground).
fn cmd_overlay_clear(stream: &mut UnixStream) -> Result<()> {
    match demote_from_overlay() {
        Some((ime_pid, behind_pid)) => {
            writeln!(
                stream,
                "OK overlay-cleared ime-pid={ime_pid} repromoted-behind-pid={behind_pid}"
            )?;
        }
        None => {
            writeln!(stream, "OK overlay-was-not-active")?;
        }
    }
    Ok(())
}

fn cmd_kill(stream: &mut UnixStream, app_id: &str) -> Result<()> {
    if app_id.is_empty() {
        writeln!(stream, "ERR kill-empty-app-id")?;
        return Ok(());
    }
    let Some(s) = state::get(app_id) else {
        writeln!(stream, "ERR not-tracked {app_id}")?;
        return Ok(());
    };
    match zygote_client::kill(s.pid, false) {
        Ok(()) => {
            state::remove(app_id);
            writeln!(stream, "OK killed app={app_id} pid={pid}", pid = s.pid)?;
            log::info!("wart-arbiter: killed {app_id} pid={pid}", pid = s.pid);
            Ok(())
        }
        Err(e) => {
            writeln!(stream, "ERR kill-failed {app_id} pid={pid}: {e:#}", pid = s.pid)?;
            log::warn!("wart-arbiter: kill {app_id} failed: {e:#}");
            Ok(())
        }
    }
}

fn cmd_list(stream: &mut UnixStream) -> Result<()> {
    let mut apps = state::snapshot();
    apps.sort_by(|a, b| a.app_id.cmp(&b.app_id));
    let fg = state::current_foreground();
    let ime = state::current_active_ime().map(|i| i.app_id);
    let focus = state::current_editor_focus();
    writeln!(stream, "OK count={}", apps.len())?;
    for app in apps {
        let elapsed_ms = app.launched_mono.elapsed().as_millis();
        let mut markers = String::new();
        if fg.as_deref() == Some(&app.app_id) { markers.push_str(" [fg]"); }
        if ime.as_deref() == Some(&app.app_id) { markers.push_str(" [ime]"); }
        if focus.as_ref().map(|f| f.pid) == Some(app.pid) {
            markers.push_str(&format!(" [editor:{}]", focus.as_ref().unwrap().editor_info.input_type));
        }
        writeln!(
            stream,
            "  app={} pid={} elapsed_ms={elapsed_ms}{markers}",
            app.app_id, app.pid
        )?;
    }
    Ok(())
}

fn cmd_preload(stream: &mut UnixStream, app_id: &str) -> Result<()> {
    if app_id.is_empty() {
        writeln!(stream, "ERR preload-empty-app-id")?;
        return Ok(());
    }
    match zygote_client::preload(app_id) {
        Ok(reply) => {
            // Zygote replies `OK <kind> <count>`; forward as-is so
            // the client sees the same kind/count info.
            writeln!(stream, "{reply}")?;
            log::info!("wart-arbiter: preload {app_id} → {reply}");
            Ok(())
        }
        Err(e) => {
            writeln!(stream, "ERR preload-failed {app_id}: {e:#}")?;
            log::warn!("wart-arbiter: preload {app_id} failed: {e:#}");
            Ok(())
        }
    }
}
