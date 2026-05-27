//! Per-host control socket for arbiter→guest event delivery
//! (task 47 step 3a).
//!
//! The outbound half of the IME protocol (guest→arbiter editor-focus
//! events) was step 2 — `ime_host_impl.rs` opens a fresh UNIX socket
//! per call. This module is the INBOUND half: each wart-host child
//! binds `/data/local/tmp/wart-host-<pid>.sock` and the arbiter
//! pushes events down it when an IME app calls e.g. `ime-send-key-event`.
//!
//! Architecture (matches the existing InputFlinger drain pattern):
//!
//!   accept thread (background)         render loop (main thread)
//!   ───────────────────────             ───────────────────────────
//!   listener.accept()                   ── per frame ──
//!   read lines                          drain_queue() → Vec<InboundEvent>
//!   parse                               for each event:
//!   queue.push_back(event)                dispatch_key_v2(skiko, store, …)
//!                                       (re-uses task 33 step 3 dispatch)
//!
//! wasmtime's `Store` is `!Send` — only the render-loop thread can
//! call into the wasm guest. The accept thread JUST parses + queues;
//! the render loop drains + dispatches.
//!
//! Step 3a wire format (one line per event, ASCII):
//!
//!   key-event <code-point> <key-id> <down|up>
//!
//! Step 3b extensions (when the war.ime.keyboard warpkg ships):
//!
//!   commit-text  <utf8-string-rest-of-line>       (needs new WIT export)
//!   composing    <utf8-string-rest-of-line>       (needs new WIT export)
//!   set-selection <start> <end>
//!
//! Per-host socket path is derived from getpid() — the arbiter knows
//! the focused-app's pid (it's in `EditorFocus`), so it can address
//! `/data/local/tmp/wart-host-<focus.pid>.sock` directly. No
//! registration handshake needed.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};

/// One incoming event. Stored in the queue between the accept thread
/// and the render-loop drain.
#[derive(Clone, Copy, Debug)]
pub enum InboundEvent {
    /// Synthesize a key event into the focused guest. `action` is
    /// 0=down, 1=up (matches `dispatch_key_v2`'s `kind` byte).
    KeyEvent { code_point: u32, key_id: u32, action: u8 },
}

fn queue() -> &'static Mutex<VecDeque<InboundEvent>> {
    static Q: OnceLock<Mutex<VecDeque<InboundEvent>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Drain all queued events. Called once per frame by the render loop.
/// Returns an empty Vec when no events are queued.
pub fn drain_queue() -> Vec<InboundEvent> {
    match queue().lock() {
        Ok(mut q) => q.drain(..).collect(),
        Err(_)    => Vec::new(),
    }
}

/// Bind the per-host socket + spawn the accept thread. Returns the
/// socket path for logging. Called by `standalone::run_with_engine`
/// AFTER fork (so each forked child binds its own pid-named socket).
pub fn spawn_listener() -> Result<String> {
    let path = format!("/data/local/tmp/wart-host-{}.sock", std::process::id());
    if Path::new(&path).exists() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("UnixListener::bind {path}"))?;

    // 0o666 — same as the arbiter's socket. The arbiter is root in
    // the dev path so perms aren't load-bearing; on a sepolicy'd
    // production build the path + SELinux context matter more.
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));

    let path_clone = path.clone();
    std::thread::Builder::new()
        .name("wart-host-ime-inbound".into())
        .spawn(move || {
            loop {
                let (stream, _addr) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) => {
                        log::warn!("ime-inbound: accept failed: {e}");
                        continue;
                    }
                };
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    parse_and_queue(&line);
                }
            }
        })
        .with_context(|| "spawn wart-host-ime-inbound thread")?;

    Ok(path_clone)
}

/// Parse one wire-format line and push the matching event onto the
/// queue. Silently drops malformed lines (with a warn log) so a buggy
/// or hostile arbiter can't crash the guest.
fn parse_and_queue(line: &str) {
    let line = line.trim_end();
    if let Some(rest) = line.strip_prefix("key-event ") {
        // <code-point> <key-id> <down|up>
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() != 3 {
            log::warn!("ime-inbound: malformed key-event: {line:?}");
            return;
        }
        let Ok(code_point) = parts[0].parse::<u32>() else {
            log::warn!("ime-inbound: bad code-point in {line:?}");
            return;
        };
        let Ok(key_id) = parts[1].parse::<u32>() else {
            log::warn!("ime-inbound: bad key-id in {line:?}");
            return;
        };
        let action: u8 = match parts[2] {
            "down" => 0,
            "up"   => 1,
            other  => {
                log::warn!("ime-inbound: bad action {other:?} in {line:?}");
                return;
            }
        };
        if let Ok(mut q) = queue().lock() {
            q.push_back(InboundEvent::KeyEvent { code_point, key_id, action });
        }
    } else if !line.is_empty() {
        // Unknown verb — log so we can spot a protocol-skew between
        // arbiter + host versions. Don't crash.
        log::warn!("ime-inbound: unknown verb in {line:?}");
    }
}
