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
use std::time::{Instant, SystemTime};

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
        Some("list") => run_client("list", None),
        Some("kill") => run_client("kill", args.get(1).cloned()),
        Some("preload") => run_client("preload", args.get(1).cloned()),
        Some("foreground") => run_client("foreground", args.get(1).cloned()),
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
           wart-arbiter list\n\
           wart-arbiter kill            <app-id>\n\
           wart-arbiter preload         <app-id>\n\
           wart-arbiter foreground      <app-id>\n\
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
        "launch" | "launch-headless" | "kill" | "preload" | "foreground"
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

    let result = match verb {
        "launch"          => cmd_launch(&mut stream, &rest, /*gui=*/ true),
        "launch-headless" => cmd_launch(&mut stream, &rest, /*gui=*/ false),
        "kill"            => cmd_kill(&mut stream, &rest),
        "list"            => cmd_list(&mut stream),
        "preload"         => cmd_preload(&mut stream, &rest),
        "foreground"      => cmd_foreground(&mut stream, &rest),
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

    writeln!(
        stream,
        "OK attached editor pid={pid} app={} input-type={input_type} \
         prev-pid={prev_pid} route→{ime_dest}",
        owner.app_id,
    )?;
    log::info!(
        "arbiter: attach-editor pid={pid} app={} input-type={input_type} hint={hint:?} \
         initial-text-len={} → route to {ime_dest} (step 2 delivers on-editor-attached)",
        owner.app_id,
        initial_text.len(),
    );
    Ok(())
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
        writeln!(stream, "OK detached pid={pid} route→{ime_dest}")?;
        log::info!(
            "arbiter: detach-editor pid={pid} → route to {ime_dest} (step 2 delivers on-editor-detached)"
        );
    } else {
        writeln!(stream, "OK no-op pid={pid} (not focused)")?;
        log::info!("arbiter: detach-editor pid={pid} — was not focused, no-op");
    }
    Ok(())
}

/// Shared backend for the five `ime-*` commands: route IME-side input
/// (commit-text / send-key-event / set-composing-text / etc.) to the
/// currently-focused editor's app. Step 1 just logs the intent —
/// actual delivery is step 2's per-host control socket.
fn cmd_ime_route(stream: &mut UnixStream, verb: &str, rest: &str) -> Result<()> {
    let Some(focus) = state::current_editor_focus() else {
        writeln!(stream, "ERR no-focused-editor")?;
        return Ok(());
    };
    writeln!(
        stream,
        "OK route→pid={} (input-type={}) {verb} args={:?}",
        focus.pid, focus.editor_info.input_type, rest,
    )?;
    log::info!(
        "arbiter: ime-{verb} → editor pid={} app-input-type={} args={:?} \
         (step 2 delivers to the focused app's host)",
        focus.pid,
        focus.editor_info.input_type,
        rest,
    );
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

fn cmd_launch(stream: &mut UnixStream, app_id: &str, gui: bool) -> Result<()> {
    if app_id.is_empty() && !gui {
        writeln!(stream, "ERR launch-empty-app-id")?;
        return Ok(());
    }
    let result = if gui {
        zygote_client::launch_gui(app_id)
    } else {
        zygote_client::launch(app_id)
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
            // apply; skip promotion for them.
            if gui {
                promote_to_foreground(&key, pid);
            }
            writeln!(stream, "OK pid={pid} app={key}")?;
            log::info!("wart-arbiter: launched {key} → pid {pid}");
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
