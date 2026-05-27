//! wart-zygote — native fork+COW launcher for wart apps (task 45 spike).
//!
//! See `tasks/45-wart-zygote-spike.md` for the architectural framing.
//! In one line: this is `app_process`-shaped but ART-free — preload
//! `wasmtime::Engine` once, `fork()` per app, child inherits the engine
//! via copy-on-write.
//!
//! Step 1 (this file's current scope): plain UNIX-socket protocol,
//! text wire format (`LAUNCH <app-id>\n` → `OK <pid>\n` / `ERR <reason>\n`).
//! The forked child dispatches to `run_once::run_with_engine(&engine, app_id)`
//! which is `wasi:cli/command`-shaped (one call to `wasi:cli/run.run`,
//! exit). No EGL/SF preload in the parent (D5), no binder preload (D7) —
//! the child first-inits both via the existing `run_once` plumbing.
//!
//! Step 2+ will extend the child path with the full Compose render loop
//! (refactored out of `standalone.rs`) and add multi-app concurrency
//! verification.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use wasmtime::Engine;

use crate::{run_once, App};

/// Where the zygote listens for `LAUNCH` requests.
///
/// `/data/local/tmp/` is the dev path — SELinux-permissive for `su`,
/// SAR-stable. Production would move to `/dev/socket/wart-zygote` via
/// init.rc + a `wart_zygote` SELinux domain (task 46+).
pub const ZYGOTE_SOCK_PATH: &str = "/data/local/tmp/wart-zygote.sock";

/// One-shot preloaded engine. Held in a `OnceLock` so the listen loop
/// can hand a `&Engine` to each forked child without re-allocating.
///
/// On `fork()` the child inherits the OnceLock's slot via COW; the
/// inner `Engine` (with its Cranelift caches, type registry, etc.) is
/// COW-shared with the parent. As long as nothing in the child path
/// mutates the engine in place, all pages stay read-only and shared.
static PRELOADED_ENGINE: OnceLock<Engine> = OnceLock::new();

/// Parent-side entry: bind the socket, accept LAUNCH commands, fork
/// per request. Never returns under normal operation.
///
/// `preload_app_id` is documentary at MVP — we preload only the
/// `wasmtime::Engine` (which is app-agnostic). Per-app `Component`
/// preload comes in a follow-up once we add a real preload-registry.
/// The arg is accepted now so the CLI shape ages well.
pub fn serve(preload_app_id: Option<&str>) -> Result<()> {
    // Logging up first — the zygote parent runs unattended; logcat is the
    // only easy observation channel.
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
    );
    log::info!(
        "wart-zygote: starting — sock={} preload_hint={:?}",
        ZYGOTE_SOCK_PATH,
        preload_app_id,
    );

    // Preload the engine. This is the whole point of the zygote — the
    // pages this allocates (wasmtime Engine state, Cranelift tables, etc.)
    // get COW-shared into every forked child.
    let engine = App::make_engine();
    PRELOADED_ENGINE
        .set(engine)
        .map_err(|_| anyhow!("PRELOADED_ENGINE set twice"))?;
    log::info!("wart-zygote: engine preloaded");

    // Bind the listen socket. Unlink any stale path first — the AF_UNIX
    // bind would otherwise fail with EADDRINUSE on a respawn.
    let sock_path = Path::new(ZYGOTE_SOCK_PATH);
    if sock_path.exists() {
        std::fs::remove_file(sock_path)
            .with_context(|| format!("removing stale socket {ZYGOTE_SOCK_PATH}"))?;
    }
    let listener = UnixListener::bind(ZYGOTE_SOCK_PATH)
        .with_context(|| format!("UnixListener::bind {ZYGOTE_SOCK_PATH}"))?;
    // World-writeable so non-root clients on the dev path can talk to us.
    // (Production wart-arbiter is root and the path is sepolicy-gated.)
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(
        ZYGOTE_SOCK_PATH,
        std::fs::Permissions::from_mode(0o666),
    );
    log::info!("wart-zygote: listening on {ZYGOTE_SOCK_PATH}");

    loop {
        let (stream, _addr) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) => {
                log::warn!("wart-zygote: accept failed: {e}");
                continue;
            }
        };
        if let Err(e) = handle_one(&listener, stream) {
            log::warn!("wart-zygote: client error: {e:#}");
        }
    }
}

/// Handle a single client connection: parse one command, fork if it's
/// a LAUNCH, else respond with an error and return.
fn handle_one(listener: &UnixListener, mut stream: UnixStream) -> Result<()> {
    // Read a single line. Tiny buffer; one-shot text protocol.
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Err(anyhow!("client closed without sending a command"));
    }
    let cmd = line.trim_end_matches('\n').trim_end_matches('\r');
    log::info!("wart-zygote: cmd={cmd:?}");

    let Some(app_id) = cmd.strip_prefix("LAUNCH ") else {
        let _ = writeln!(stream, "ERR unknown-command {cmd}");
        return Err(anyhow!("unknown command: {cmd}"));
    };
    let app_id = app_id.trim().to_string();
    if app_id.is_empty() {
        let _ = writeln!(stream, "ERR empty-app-id");
        return Err(anyhow!("empty app-id"));
    }

    // fork(). Returns 0 to child, child-pid to parent, -1 on error.
    //
    // Safety: we do nothing async-signal-unsafe between fork and the
    // child's first action; the child immediately drops fds we don't
    // want it to hold, then enters Rust code that owns its own state.
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => {
            let err = std::io::Error::last_os_error();
            let _ = writeln!(stream, "ERR fork {err}");
            Err(anyhow!("fork: {err}"))
        }
        0 => {
            // CHILD path.
            //
            // (1) Close the FDs we inherited from the parent that we
            //     don't want to keep: the listen socket (parent uses
            //     it), and our reader/stream copies (used to ack the
            //     client back through; only the parent should respond).
            //     We do that by dropping the Rust handles — Drop calls
            //     close(2).
            drop(reader);
            drop(stream);
            // The listener is moved by reference into this function; we
            // can't drop it here. Close its FD directly. (Safe because
            // after this the child never touches the listener.)
            let listen_fd = std::os::unix::io::AsRawFd::as_raw_fd(listener);
            unsafe { libc::close(listen_fd) };

            // (2) Optional hold-for-measurement. Env-gated; off by
            //     default. Used by the step-1 COW analysis to freeze
            //     the child right after fork (preloaded engine page-
            //     state still intact, no run_once side effects) so
            //     /proc/<pid>/smaps_rollup can be sampled.
            if let Ok(s) = std::env::var("WART_ZYGOTE_HOLD_SECS") {
                if let Ok(secs) = s.parse::<u64>() {
                    log::info!("wart-zygote/child: holding {secs}s before run (WART_ZYGOTE_HOLD_SECS)");
                    std::thread::sleep(std::time::Duration::from_secs(secs));
                }
            }

            // (3) Dispatch to the existing run_once path. This is what
            //     `wart-host --run-once <app-id>` does, just with a
            //     caller-supplied engine instead of a freshly built one.
            let engine = PRELOADED_ENGINE
                .get()
                .expect("PRELOADED_ENGINE not set in child");
            let exit = match run_once::run_with_engine(engine, &app_id) {
                Ok(()) => 0,
                Err(e) => {
                    log::error!("wart-zygote/child: run failed: {e:#}");
                    1
                }
            };
            // (4) Exit immediately. Do not let Rust's normal exit path
            //     run global destructors — those are COW pages we
            //     shouldn't dirty on the way out.
            unsafe { libc::_exit(exit) };
        }
        child_pid => {
            // PARENT path. Acknowledge to the client and return.
            //
            // We deliberately do NOT waitpid() the child here. The
            // child is fully detached; if the caller wants to know
            // when it exits they can poll its process or use a future
            // socket protocol enhancement. For step 1 the smoke test
            // just sleeps + checks logcat.
            writeln!(stream, "OK {child_pid}")
                .with_context(|| format!("ack {child_pid} to client"))?;
            log::info!("wart-zygote: forked pid={child_pid} for app_id={app_id}");
            Ok(())
        }
    }
}

/// Client-side: connect to the zygote, write `LAUNCH <app-id>\n`, read
/// the response, return the child pid on success.
pub fn launch_client(app_id: &str) -> Result<i32> {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
    );

    let mut stream = UnixStream::connect(ZYGOTE_SOCK_PATH)
        .with_context(|| format!("connect {ZYGOTE_SOCK_PATH} — is the zygote running?"))?;
    write!(stream, "LAUNCH {app_id}\n")?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    let response = response.trim_end();
    log::info!("wart-zygote/client: response={response:?}");

    if let Some(rest) = response.strip_prefix("OK ") {
        let pid: i32 = rest
            .trim()
            .parse()
            .with_context(|| format!("parse pid from {response:?}"))?;
        println!("launched {app_id} → pid {pid}");
        Ok(pid)
    } else if let Some(rest) = response.strip_prefix("ERR ") {
        Err(anyhow!("zygote rejected: {rest}"))
    } else {
        Err(anyhow!("zygote returned malformed response: {response:?}"))
    }
}
