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
//!                         marked with [fg].
//!   kill <app-id>       — KILL the zygote-tracked pid for this app-id
//!   preload <app-id>    — PRELOAD on the zygote (post-install refresh)
//!   foreground <id>     — promote <id> to foreground; SIGUSR1 the
//!                         previous fg, SIGUSR2 the new fg, write
//!                         /proc/<pid>/oom_score_adj on both.

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
           wart-arbiter foreground      <app-id>\n",
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
    writeln!(stream, "OK count={}", apps.len())?;
    for app in apps {
        let elapsed_ms = app.launched_mono.elapsed().as_millis();
        let marker = if fg.as_deref() == Some(&app.app_id) { " [fg]" } else { "" };
        writeln!(
            stream,
            "  app={} pid={} elapsed_ms={elapsed_ms}{marker}",
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
