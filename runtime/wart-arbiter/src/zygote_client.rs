//! Text-protocol client for wart-host's zygote socket.
//!
//! Reuses the protocol established in task-45 / task-46 step 1+2:
//!   LAUNCH <app-id>            -> OK <child-pid> | ERR <reason>
//!   LAUNCH_GUI [<app-id>]      -> OK <child-pid> | ERR <reason>
//!   KILL <pid>                 -> OK <pid> | ERR <reason>
//!   KILL_FORCE <pid>           -> OK <pid> | ERR <reason>
//!   PRELOAD <app-id>           -> OK <kind> <count> | ERR <reason>
//!
//! One-shot connections per command (matches the AOSP zygote
//! pattern). The arbiter is a long-lived process so the cost of
//! reconnecting per operation is negligible.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use anyhow::{anyhow, Context, Result};

/// Default path where wart-host's zygote listens. Configurable via
/// the `WART_ZYGOTE_SOCK` env var for tests / non-standard layouts.
pub const DEFAULT_ZYGOTE_SOCK: &str = "/data/local/tmp/wart-zygote.sock";

pub fn zygote_sock_path() -> String {
    std::env::var("WART_ZYGOTE_SOCK")
        .unwrap_or_else(|_| DEFAULT_ZYGOTE_SOCK.to_string())
}

/// Send `LAUNCH_GUI <app-id>` (or `LAUNCH_GUI` alone if `app_id` is
/// empty, which the zygote treats as "use dev cwasm"). Returns the
/// forked child's pid.
pub fn launch_gui(app_id: &str) -> Result<i32> {
    let line = if app_id.is_empty() {
        "LAUNCH_GUI\n".to_string()
    } else {
        format!("LAUNCH_GUI {app_id}\n")
    };
    let reply = send(&line)?;
    parse_pid(&reply)
}

/// Task 47 step 3c — send `LAUNCH_GUI_OVERLAY <app-id>`. The forked
/// child acquires a bottom-strip overlay SurfaceControl instead of a
/// fullscreen one. Used for IME apps such as `war.ime.keyboard`.
pub fn launch_gui_overlay(app_id: &str) -> Result<i32> {
    if app_id.is_empty() {
        return Err(anyhow!("LAUNCH_GUI_OVERLAY requires an app-id"));
    }
    let reply = send(&format!("LAUNCH_GUI_OVERLAY {app_id}\n"))?;
    parse_pid(&reply)
}

/// Send `LAUNCH <app-id>` (headless `wasi:cli/command` consumer).
/// Returns the forked child's pid.
pub fn launch(app_id: &str) -> Result<i32> {
    let reply = send(&format!("LAUNCH {app_id}\n"))?;
    parse_pid(&reply)
}

/// Send `KILL <pid>` (SIGTERM) or `KILL_FORCE <pid>` (SIGKILL).
/// Returns Ok on the zygote's `OK <pid>` ack.
pub fn kill(pid: i32, force: bool) -> Result<()> {
    let verb = if force { "KILL_FORCE" } else { "KILL" };
    let reply = send(&format!("{verb} {pid}\n"))?;
    if reply.starts_with("OK ") {
        Ok(())
    } else {
        Err(anyhow!("zygote rejected {verb} {pid}: {reply}"))
    }
}

/// Send `PRELOAD <app-id>` — pre-deserialize the app's components in
/// the zygote so the next fork COW-inherits them. The installer
/// should call this after a successful install.
pub fn preload(app_id: &str) -> Result<String> {
    let reply = send(&format!("PRELOAD {app_id}\n"))?;
    if reply.starts_with("OK ") {
        Ok(reply)
    } else {
        Err(anyhow!("zygote rejected PRELOAD {app_id}: {reply}"))
    }
}

/// Single-command round-trip. Opens a connection, writes one line,
/// reads one line back, closes. Returns the response without the
/// trailing newline.
fn send(line: &str) -> Result<String> {
    let path = zygote_sock_path();
    let mut stream = UnixStream::connect(&path)
        .with_context(|| format!("connect {path} — is wart-host --zygote running?"))?;
    stream.write_all(line.as_bytes())
        .with_context(|| format!("write to {path}"))?;
    stream.flush().ok();

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)
        .with_context(|| format!("read from {path}"))?;
    Ok(response.trim_end_matches('\n').trim_end_matches('\r').to_string())
}

fn parse_pid(reply: &str) -> Result<i32> {
    let rest = reply.strip_prefix("OK ")
        .ok_or_else(|| anyhow!("zygote did not OK: {reply}"))?;
    rest.trim().parse::<i32>()
        .map_err(|e| anyhow!("parse pid from {reply:?}: {e}"))
}
