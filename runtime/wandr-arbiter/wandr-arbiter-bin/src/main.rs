//! wandr-arbiter — Hybrid runtime policy daemon (task 46 step 3).
//!
//! Two modes:
//!
//! - **Daemon** (`wandr-arbiter --daemon`): binds
//!   `/data/local/tmp/wandr-arbiter.sock`, accepts text commands
//!   from clients, dispatches to wandr-host's zygote socket.
//! - **Client** (`wandr-arbiter <cmd> [args]`): connects to the
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

mod sensor_driver;
mod state;
mod zygote_client;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context, Result};

use wandr_arbiter_alarm::AlarmModule;
use wandr_arbiter_core::{Event, Registry, Reply, Store, PRIMARY_DISPLAY};
use wandr_arbiter_audio::AudioModule;
use wandr_arbiter_keyguard::KeyguardModule;
use wandr_arbiter_notify::NotifyModule;
use wandr_arbiter_power::PowerModule;
use wandr_arbiter_net::NetModule;
use wandr_arbiter_events::EventsModule;
use wandr_arbiter_sensors::SensorsModule;
use wandr_arbiter_shell::ShellModule;
use wandr_arbiter_wm::WmModule;

use crate::state::AppState;

/// The arbiter control socket — resolved (NOT hardcoded) from `WANDR_ARBITER_SOCK`
/// else the canonical default. This is the OWNER of the path (the daemon binds it);
/// the host crate + the C++ `wandr-inputflinger` service resolve the same env var so
/// all three agree without re-hardcoding the literal. Resolved once.
fn arbiter_sock_path() -> &'static str {
    use std::sync::OnceLock;
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| {
        std::env::var("WANDR_ARBITER_SOCK")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/data/local/tmp/wandr-arbiter.sock".to_string())
    })
    .as_str()
}

/// Where the running-apps state is persisted between daemon restarts.
/// Children of the zygote outlive arbiter restarts; this lets us
/// reattach to them via `kill(pid, 0)` liveness checks rather than
/// starting with an empty map every time.
const ARBITER_STATE_PATH: &str = "/data/local/tmp/wandr-arbiter-state.json";

/// Panic-hook destination. The arbiter is small + mostly synchronous,
/// but if a Mutex gets poisoned or some socket I/O panics, we want a
/// trail visible on the next startup.
const ARBITER_CRASH_PATH: &str = "/data/local/tmp/wandr-arbiter-crash.json";

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
        // Task 92 — task-manager wire verbs (host→arbiter; CLI for testing):
        // task-list (machine-parseable role+uptime snapshot), task-kill <app-id>
        // (protected/not-found-aware kill).
        Some("task-list") => run_client("task-list", None),
        Some("task-kill") => run_client("task-kill", args.get(1).cloned()),
        Some("preload") => run_client("preload", args.get(1).cloned()),
        Some("foreground") => run_client("foreground", args.get(1).cloned()),
        // Task 57 — launcher / home app.
        Some("set-home") => run_client("set-home", args.get(1).cloned()),
        Some("go-home")  => run_client("go-home", None),
        // Task 56 — taskbar nav (CLI handles for testing).
        Some("back")        => run_client("back", None),
        Some("cycle-task")  => run_client("cycle-task", None),
        // Keyguard — lock/unlock (no-arg; auto-lock is screen-off-driven).
        Some("lock")   => run_client("lock", None),
        Some("unlock") => run_client("unlock", None),
        // Task 47 step 3c — manual overlay engage/clear.
        Some("overlay") => run_client("overlay", args.get(1).cloned()),
        Some("overlay-clear") => run_client("overlay-clear", None),
        // Task 47 step 1 — IME routing commands. The client side
        // passes the verb's tail through unchanged; the daemon
        // parses arg-shapes per command.
        Some(verb @ ("set-ime" | "attach-editor" | "detach-editor"
                    | "ime-commit-text" | "ime-send-key-event"
                    | "ime-set-composing-text" | "ime-finish-composing-text"
                    | "ime-set-selection" | "ime-overlay-height"
                    // Chrome-coherence — host→arbiter verbs, exposed on the CLI
                    // for testing/debugging (register-chrome <app-id> <pid>,
                    // set-orientation-lock <0|1>).
                    | "register-chrome" | "set-orientation-lock"
                    // Arbiter Inc. 3c — alarm verbs (host→arbiter; CLI for testing):
                    // schedule-alarm <app-id> <id> <when-unix-ms> <repeat-ms> [kind],
                    // cancel-alarm <app-id> <id>.
                    | "schedule-alarm" | "cancel-alarm"
                    // Signal bg-receipt M3 — notification verbs (host→arbiter; CLI
                    // for testing): notify-post <owner> <id> <enc-title> <enc-body>,
                    // notify-cancel <owner> <id>, notify-list, notify-click <nid>.
                    | "notify-post" | "notify-cancel" | "notify-list" | "notify-click"
                    // PowerManager — class report (host→arbiter; CLI for testing).
                    | "report-power-class"
                    // wandr-arbiter-audio M1 — audio-focus verbs (host→arbiter; CLI
                    // for testing): audio-focus-request <pid|app-id> <kind>,
                    // audio-focus-abandon <pid|app-id>, audio-focus-list.
                    | "audio-focus-request" | "audio-focus-abandon" | "audio-focus-list"
                    // wandr-arbiter-audio M3 — comms-session + routing verbs:
                    // audio-call-start/end <pid|app-id>, audio-route <pid|app-id>
                    // <speaker|earpiece>.
                    | "audio-call-start" | "audio-call-end" | "audio-route"
                    // Ringer (incoming call) + ringer policy verbs:
                    | "audio-ring-start" | "audio-ring-stop" | "audio-ringer-mode"
                    // play-tone [pid|app-id] [ms] [hz] [vol0-1] — arbiter→host tone
                    // (test/warm-up); target optional, defaults to the foreground host.
                    | "play-tone"
                    // wandr-arbiter-audio P8 — volume <up|down> (host→arbiter on the
                    // raw socket from a VOLUME key; CLI form for testing/scripting).
                    | "volume"
                    // wandr-arbiter-audio P8 — global mute <on|off|toggle> +
                    // per-app mute <pid|app-id> <on|off|toggle>; mic-mute
                    // <on|off|toggle> (input/mic-disable, global); per-app
                    // app-mic-mute <pid|app-id> <on|off|toggle>.
                    | "mute" | "app-mute" | "mic-mute" | "app-mic-mute"
                    // Task 73 — WM geometry (handled by a core module, not the
                    // legacy match). The host normally sends this over the raw
                    // socket; the CLI form is for testing.
                    | "report-orientation"
                    // Task 77 — SensorService verbs (sim/test + device verify):
                    // report-sensor <kind> <x> [y z], sensor-state, sensor-hold
                    // <kind> <on|off>.
                    | "report-sensor" | "sensor-state" | "sensor-hold"
                    // Task 81 — ART-less display power (host→arbiter on the raw
                    // socket from a POWER key; CLI form for testing/boot force-on):
                    // power-key [pid] (toggle), panel <on|off> (explicit).
                    | "power-key" | "panel"
                    // Task 86 — auto-brightness manual override + live curve tuning:
                    // brightness <auto|0.0..1.0>, brightness-scale <lux>.
                    | "brightness" | "brightness-scale"
                    // Task 86 follow-on — PowerManager screen-off-timeout:
                    // user-activity (input dispatcher poke), screen-timeout <ms|off>.
                    | "user-activity" | "screen-timeout"
                    // Task 88 — connectivity verbs. The wandr-net daemon sends
                    // report-net-state over the raw socket and the host sends
                    // net-subscribe; the CLI forms are for testing/verification
                    // (net-status dumps the current link, e.g. `wandr-arbiter
                    // net-status`).
                    | "report-net-state" | "net-status" | "net-subscribe"
                    // Task 90 M2 — privileged wifi management (CLI test forms):
                    // `wandr-arbiter wifi-scan`, `wifi-connect <b64ssid> <b64psk>`,
                    // `wifi-set-enabled <0|1>`, `wifi-is-enabled`. The host sends the
                    // same verbs over the raw socket from its wifi-host impl.
                    | "wifi-scan" | "wifi-connect" | "wifi-set-enabled" | "wifi-is-enabled"
                    // Task 90 — generic event bus verbs. The host's wandr:events
                    // producer + the wandr-net daemon send evt-publish; the host
                    // sends evt-subscribe (host-config from package.toml). CLI forms
                    // for testing: `wandr-arbiter evt-publish <topic> <base64>`,
                    // `wandr-arbiter evt-subscribe <pid> <topic>`.
                    | "evt-publish" | "evt-subscribe" | "evt-unsubscribe")) => {
            run_client_multi(verb, &args[1..])
        }
        Some(other) => {
            eprintln!("wandr-arbiter: unknown command: {other}");
            print_usage();
            std::process::exit(2);
        }
        None => {
            print_usage();
            std::process::exit(2);
        }
    };

    if let Err(e) = result {
        eprintln!("wandr-arbiter: {e:#}");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n\
         \n\
         Daemon mode:\n\
           wandr-arbiter --daemon\n\
         \n\
         Client commands (require the daemon running):\n\
           wandr-arbiter launch          <app-id>\n\
           wandr-arbiter launch-headless <app-id>\n\
           wandr-arbiter launch-overlay  <app-id>          (IME-shaped surface, task 47 step 3c)\n\
           wandr-arbiter list\n\
           wandr-arbiter kill            <app-id>\n\
           wandr-arbiter preload         <app-id>\n\
           wandr-arbiter foreground      <app-id>\n\
           wandr-arbiter set-home        <app-id>          (designate the home/launcher app; '-' clears)\n\
           wandr-arbiter go-home                           (foreground the home app)\n\
           wandr-arbiter back                              (task 56 — route ESC to fg app)\n\
           wandr-arbiter cycle-task                        (task 56 — switch to next user app)\n\
           wandr-arbiter overlay         <app-id>          (engage IME overlay split, task 47 step 3c)\n\
           wandr-arbiter overlay-clear                     (tear it down)\n\
         \n\
         IME routing (task 47):\n\
           wandr-arbiter set-ime         <app-id>\n\
           wandr-arbiter attach-editor   <pid> [input-type] [hint] [initial-text]\n\
           wandr-arbiter detach-editor   <pid>\n\
           wandr-arbiter ime-commit-text         <text>\n\
           wandr-arbiter ime-send-key-event      <code-point> <key-id> <down|up>\n\
           wandr-arbiter ime-set-composing-text  <text>\n\
           wandr-arbiter ime-finish-composing-text\n\
           wandr-arbiter ime-set-selection       <start> <end>\n",
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
        | "task-kill"
    );
    let line = match (needs_arg, arg) {
        (true, Some(a))  => format!("{verb} {a}\n"),
        (true, None)     => {
            return Err(anyhow!("wandr-arbiter {verb}: requires <app-id>"));
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
    let mut stream = UnixStream::connect(arbiter_sock_path())
        .with_context(|| format!("connect {} — is the daemon running?", arbiter_sock_path()))?;
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
    log::info!("wandr-arbiter: starting daemon — sock={}", arbiter_sock_path());
    log::info!("wandr-arbiter: zygote sock = {}", zygote_client::zygote_sock_path());

    // Task 46 crash-marker — panic-hook drops a JSON file on the way
    // out, drained + logged on next startup.
    install_panic_hook();
    drain_prior_crash_marker();

    // Task 46 crash-marker — restore in-memory state from disk. Each
    // persisted pid is liveness-checked via `kill(pid, 0)`; survivors
    // are re-inserted, dead ones are dropped + logged.
    let restored_fg = match state::restore_from(Path::new(ARBITER_STATE_PATH)) {
        Ok((alive, dead, fg)) => {
            log::info!(
                "wandr-arbiter: state restore — {alive} alive app(s) re-attached, {dead} dropped"
            );
            fg
        }
        Err(e) => {
            log::warn!("wandr-arbiter: state restore failed: {e:#} (continuing with empty state)");
            None
        }
    };
    // Task 74 — seed the live surface model from the restored registry. Every
    // re-attached app starts Background; the persisted foreground app is marked
    // Foreground (no signal — it is already running in that state). After this
    // the model is the sole state, maintained incrementally by write-through.
    seed_surface_model(restored_fg);

    let sock_path = Path::new(arbiter_sock_path());
    if sock_path.exists() {
        std::fs::remove_file(sock_path)
            .with_context(|| format!("removing stale socket {}", arbiter_sock_path()))?;
    }
    let listener = UnixListener::bind(arbiter_sock_path())
        .with_context(|| format!("UnixListener::bind {}", arbiter_sock_path()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(
        arbiter_sock_path(),
        std::fs::Permissions::from_mode(0o666),
    );
    log::info!("wandr-arbiter: listening");

    // Task 54 — start the death watchers. The subscriber thread is the
    // primary (event-driven) path; the poller is the ≤5 s backstop that
    // also covers a dropped subscriber connection or a crashed zygote.
    spawn_death_watchers();
    spawn_alarm_timer();
    // Task 81 — the screen poller reads `debug.tracing.screen_state`, an
    // SF-sourced sysprop that goes STALE with the Java framework stopped (the
    // last value before ART-off lingers → spurious doze/auto-lock). Under
    // `WANDR_NO_ART` the arbiter is the sole screen-state authority instead: it
    // drives `Event::ScreenState` from its own `panel_on` (POWER-key toggles via
    // the power module) and force-ons the panel at boot below.
    if no_art() {
        log::info!("wandr-arbiter: WANDR_NO_ART — screen poller OFF; arbiter owns panel power");
        apply_display_power(true); // boot force-on (panel may be off from a prior --no-art wedge)
        // Task 86 follow-on — PowerManager screen-off-timeout. There is no PMS under
        // --no-art, so the arbiter owns the inactivity→sleep decision: this ticker
        // feeds Event::IdleTick to the power module (which checks idle vs the
        // timeout); wandr-inputflinger pokes `user-activity` on real input. Under
        // ART-up the framework's PowerManagerService owns this (poller drives doze).
        spawn_inactivity_timer();
    } else {
        spawn_screen_poller();
    }
    sensor_driver::spawn();

    // Task 57 — boot to home. If a home app was designated in a previous
    // session (restored above), foreground it now (launching it if it
    // didn't survive the restart) so the device comes up to a usable
    // home screen with no adb command needed. No-op if no home is set
    // (the pre-launcher default). The zygote is started before the
    // arbiter by run-hybrid-stack.sh, so `launch_gui` can reach it.
    {
        let _g = arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());
        if state::current_home().is_some() {
            log::info!("wandr-arbiter: boot — foregrounding home app");
            ensure_home_foreground();
            if let Err(e) = state::save_to(Path::new(ARBITER_STATE_PATH), model_foreground_slot().as_ref().map(|(_, id)| id.as_str())) {
                log::warn!("wandr-arbiter: boot home-foreground state save failed: {e:#}");
            }
        }
    }

    loop {
        let (stream, _addr) = match listener.accept() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("wandr-arbiter: accept failed: {e}");
                continue;
            }
        };
        if let Err(e) = handle_client(stream) {
            log::warn!("wandr-arbiter: client error: {e:#}");
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

    let (verb, rest) = match line.split_once(' ') {
        Some((v, r)) => (v, r.trim().to_string()),
        None => (line, String::new()),
    };

    // High-frequency read-only polls — chrome/guests query these every render
    // (~1 Hz each, across statusbar/taskbar/etc.), so logging them at info floods
    // logcat. Log polls at debug; state-mutating commands stay at info.
    const QUIET_VERBS: &[&str] = &[
        "notify-list", "net-status", "task-list", "list", "sensor-state",
        "audio-focus-list", "status",
    ];
    if QUIET_VERBS.contains(&verb) {
        log::debug!("wandr-arbiter: cmd={line:?}");
    } else {
        log::info!("wandr-arbiter: cmd={line:?}");
    }

    // Task 54 — serialize command handling against the death-notification
    // / polling watcher threads. The accept loop is single-threaded so
    // commands were implicitly serialized before; the new background
    // threads (subscriber + poller) also mutate state + send signals, so
    // one coarse process-wide lock keeps the two paths from interleaving
    // a state mutation with a signal cascade. Low frequency — fine.
    let _guard = arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());

    // Task 73 — strangler join: a registered core module handles the verb if
    // it owns it (today: `report-orientation` → wandr-arbiter-wm); otherwise
    // fall through to the legacy match below, unchanged.
    let result = if module_owns(verb) {
        dispatch_module(verb, &rest, &mut stream)
    } else {
    match verb {
        // The zygote-coupled verbs stay in the binary (they call the zygote and
        // own the returned pid). GUI launches then dispatch `foreground` to the
        // shell module; `go-home`/`set-home` go through `ensure_home_foreground`,
        // which may relaunch home. Everything else (foreground/kill/set-ime/
        // attach/detach/ime-*/back/cycle-task/overlay/overlay-clear/list) is owned
        // by the shell module and routed above via `module_owns`.
        "launch"          => cmd_launch(&mut stream, &rest, LaunchKind::Gui),
        "launch-headless" => cmd_launch(&mut stream, &rest, LaunchKind::Headless),
        // Task 47 step 3c — overlay launch (e.g. an IME app): a bottom-strip
        // overlay SurfaceControl, started hidden; shown by `overlay` /
        // `attach-editor` when an editor focuses.
        "launch-overlay"  => cmd_launch(&mut stream, &rest, LaunchKind::GuiOverlay),
        "preload"         => cmd_preload(&mut stream, &rest),
        // Task 57 — launcher / home app (may relaunch home → zygote-coupled).
        "set-home"        => cmd_set_home(&mut stream, &rest),
        "go-home"         => cmd_go_home(&mut stream),
        // Task 90 M2 — privileged wifi management. The host's wifi-host impl
        // (and `wandr-arbiter wifi-*` for testing) sends these; the arbiter
        // relays to the wandr-net daemon's control socket (the single live
        // link owner) and pipes the reply back. `rest` carries the daemon
        // args verbatim (base64'd ssid/psk for connect, 0|1 for set-enabled).
        "wifi-scan"        => relay_wifi(&mut stream, "scan"),
        "wifi-connect"     => relay_wifi(&mut stream, &format!("connect {rest}")),
        "wifi-set-enabled" => relay_wifi(&mut stream, &format!("set-enabled {rest}")),
        "wifi-is-enabled"  => relay_wifi(&mut stream, "is-enabled"),
        other => {
            writeln!(stream, "ERR unknown-command {other}")?;
            Ok(())
        }
    }
    };

    // Task 46 crash-marker — persist state after every command. The
    // mutating commands (launch / kill / foreground) need this so
    // post-restart we see the right apps + fg; non-mutating (list /
    // preload) save anyway because it's cheap (one ~1 KB write) and
    // the simpler code path is worth the micro-cost.
    if let Err(e) = state::save_to(Path::new(ARBITER_STATE_PATH), model_foreground_slot().as_ref().map(|(_, id)| id.as_str())) {
        log::warn!("wandr-arbiter: state save failed: {e:#}");
    }

    // Task 74 Step A — rebuild the shadow surface model + assert it agrees with
    // the legacy `active_app_pid()`. Pure observation; drives nothing yet.
    log_visible_app();

    // Task 84 — re-author the input-window list for wandr-inputflinger after any
    // command that may have moved a surface/role/inset/focus (no-op + cheap when
    // the derived block is unchanged, or when not running --no-art).
    push_input_windows();

    result
}








/// One-shot write to a wandr-host child's control socket. Matches the
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

// ─── wifi management relay (task 90 M2) ───────────────────────────────

/// The wandr-net control socket the wifi-management verbs relay to. The arbiter
/// is the single coordinator on the host→arbiter→daemon path (it can intercept
/// connect for the M3 WifiConfigManager); the daemon owns the live link.
fn wandr_net_sock_path() -> String {
    std::env::var("WANDR_NET_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/data/local/tmp/wandr-net.sock".to_string())
}

/// Relay one command line to the `wandr-net` control socket and return its full
/// reply (read to EOF). Pure pass-through — the daemon owns the mechanism.
fn relay_to_wandr_net(line: &str) -> std::io::Result<String> {
    use std::io::Read;
    let sock = wandr_net_sock_path();
    let mut stream = UnixStream::connect(&sock)?;
    stream.write_all(line.as_bytes())?;
    if !line.ends_with('\n') {
        stream.write_all(b"\n")?;
    }
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;
    Ok(buf)
}

/// Relay a `wifi-*` verb to the daemon and write the reply back to the arbiter
/// client (the host's `connectivity_wifi_impl` or a CLI test). `daemon_line` is the
/// daemon-side verb (`scan` / `connect …` / `set-enabled …` / `is-enabled`).
fn relay_wifi(stream: &mut UnixStream, daemon_line: &str) -> Result<()> {
    match relay_to_wandr_net(daemon_line) {
        Ok(reply) => {
            stream.write_all(reply.as_bytes())?;
        }
        Err(e) => {
            writeln!(stream, "err wifi-relay {e} (wandr-net daemon down?)")?;
        }
    }
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
        Ok(s) => log::error!("wandr-arbiter: prior run crashed — {}", s.trim()),
        Err(e) => log::warn!("wandr-arbiter: prior crash marker unreadable: {e}"),
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

// ─── Core kernel (task 73 — modular wandr-arbiter-core) ────────────────
//
// The arbiter is being re-centralized into the native system_server
// coordinator (WMS·IMMS·AMS), one responsibility crate at a time. The
// kernel (`wandr-arbiter-core`) owns a per-display Store + an event bus +
// a module Registry; modules (`wandr-arbiter-wm` is the first) own verbs
// and react to events. Strangler migration: the legacy `match verb`
// below stays intact for every un-migrated verb; the registry is probed
// first and handles only the verbs a module claims (today:
// `report-orientation`). The legacy handlers feed the bus via `bus_emit`
// at the seams this task changes (the keyboard-inset / geometry path and
// the foreground cascade) so the WM module can push geometry.
//
// Both the Store and the Registry are process-wide singletons (mirroring
// state.rs). All access happens under `arbiter_lock` (held by the accept
// loop + the death watchers), so the inner Mutexes never contend; they
// exist only to hand out `&'static mut`-shaped access.

fn core_store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Store::new()))
}

fn registry() -> &'static Mutex<Registry> {
    static REG: OnceLock<Mutex<Registry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(build_registry()))
}

/// Build the module registry. New responsibility = +1 crate, +1 line here
/// (Open/Closed — the doc's whole point).
fn build_registry() -> Registry {
    let mut reg = Registry::new();
    reg.register(Box::new(WmModule::new()));
    // Task 74 C — the AMS+IMMS orchestration module: foreground/overlay/editor-
    // focus/IME-routing/task-cycle policy. The Open/Closed payoff — one line.
    reg.register(Box::new(ShellModule::new()));
    // Arbiter Inc. 3c — AlarmManager/JobScheduler (timed wake). One line.
    reg.register(Box::new(AlarmModule::new()));
    // Signal bg-receipt M3 — the notification module (post/cancel/list/click).
    reg.register(Box::new(NotifyModule::new()));
    // PowerManager — doze policy (reacts to Event::ScreenState, fans `doze` to hosts).
    reg.register(Box::new(PowerModule::new()));
    // Keyguard — lockscreen (auto-lock on screen-off; lock/unlock). One line.
    reg.register(Box::new(KeyguardModule::new()));
    // AudioService role (Arbiter Inc.) — M1: the cross-app audio-focus stack.
    reg.register(Box::new(AudioModule::new()));
    // SensorService role (task 77) — enable-on-demand HAL arbitration + raw→
    // semantic translation (proximity near/far). The binary's sensor-driver
    // thread feeds it Event::SensorReading; it emits Effect::SetSensor. One line.
    reg.register(Box::new(SensorsModule::new()));
    // ConnectivityService role (task 88 M1) — link status of record + guest
    // on-connectivity-change fan-out. The wandr-net daemon feeds it
    // report-net-state; it emits Effect::HostLine to subscribers. One line.
    reg.register(Box::new(NetModule::new()));
    // Generic event broker (task 90) — topic→subscribers + retained value. The
    // host's wandr:events/producer + the wandr-net daemon publish here; guests
    // subscribe (host-config from package.toml) and receive Effect::HostLine
    // pushes. One line.
    reg.register(Box::new(EventsModule::new()));
    reg
}

/// True if a registered module owns `verb` (so `handle_client` routes it to
/// the module instead of the legacy match).
fn module_owns(verb: &str) -> bool {
    registry().lock().unwrap_or_else(|e| e.into_inner()).owns(verb)
}

/// Dispatch a module-owned command through the registry (draining the event
/// cascade). Returns the reply + accumulated effects, locks released. Caller
/// holds `arbiter_lock`; the returned effects must be run via `execute_effects`
/// AFTER this returns (outside the store/registry locks).
fn run_module(verb: &str, args: &str) -> Option<(Reply, Vec<wandr_arbiter_core::Effect>)> {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut store = core_store().lock().unwrap_or_else(|e| e.into_inner());
    reg.dispatch_command(verb, args, &mut store)
}

/// Run a module-owned command + flush its effects + write the single reply line.
/// Caller holds `arbiter_lock`.
fn dispatch_module(verb: &str, rest: &str, stream: &mut UnixStream) -> Result<()> {
    match run_module(verb, rest) {
        Some((reply, effects)) => {
            execute_effects(effects);
            writeln!(stream, "{}", reply.render())?;
        }
        None => {
            // `module_owns` said yes but dispatch found no owner — a wiring
            // bug, not a client error. Report it rather than hang the client.
            writeln!(stream, "ERR module-dispatch-miss {verb}")?;
            log::warn!("arbiter: module_owns({verb}) true but dispatch_command returned None");
        }
    }
    Ok(())
}

/// Bridge a binary-side launch/home flow into the shell module's foreground
/// policy: dispatch `foreground <app_id>` and run the resulting effects. The app
/// must already be in the Store registry + have a surface. Caller holds
/// `arbiter_lock`.
fn dispatch_foreground(app_id: &str) {
    if let Some((_reply, effects)) = run_module("foreground", app_id) {
        execute_effects(effects);
    }
}

/// Inject an event from a legacy handler onto the bus and run whatever effects
/// the reacting modules requested. The seam by which the legacy state machine
/// drives the modules. Caller holds `arbiter_lock`.
fn bus_emit(ev: Event) {
    let effects = {
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        let mut store = core_store().lock().unwrap_or_else(|e| e.into_inner());
        reg.dispatch_event(ev, &mut store)
    };
    execute_effects(effects);
}

/// The single mechanism executor (task 74) — the only place that performs the
/// raw OS work a module's [`Effect`] declares. Runs effects in emission order.
/// Shared by `dispatch_module` + `bus_emit`. Caller holds `arbiter_lock`.
fn execute_effects(effects: Vec<wandr_arbiter_core::Effect>) {
    use wandr_arbiter_core::Effect;
    for eff in effects {
        match eff {
            Effect::HostLine { pid, line } => {
                let sock = host_sock_path(pid);
                if let Err(e) = deliver_to_host(&sock, &line) {
                    log::debug!("arbiter: host push → {sock} failed: {e:#}");
                }
            }
            Effect::SetRole { pid, role } => apply_role(pid, role),
            Effect::Foreground { app_id } => {
                // Signal bg-receipt M3 — a notification tap brings the owner
                // forward. Re-enters the `foreground` verb (safe: execute_effects
                // runs with no registry/store lock held, like dispatch_foreground).
                dispatch_foreground(&app_id);
            }
            Effect::Kill { pid } => {
                if let Err(e) = zygote_client::kill(pid, false) {
                    log::warn!("arbiter: Kill effect pid={pid} failed: {e:#}");
                }
            }
            Effect::Persist => {
                if let Err(e) = state::save_to(Path::new(ARBITER_STATE_PATH), model_foreground_slot().as_ref().map(|(_, id)| id.as_str())) {
                    log::warn!("arbiter: Persist effect failed: {e:#}");
                }
            }
            Effect::SetDisplayPower { on } => {
                // Task 78/81 — the power module's screen-power decision drives
                // SurfaceFlinger setPowerMode (the proper panel-off/on).
                //
                // ART-up: the arbiter is root and SF short-circuits its
                // ACCESS_SURFACE_FLINGER check, so we call wandr-hal-display inline.
                // ART-off (`WANDR_NO_ART`): bare root HANGS on that check (the
                // permission service lives in the dead system_server), so we must
                // run as uid system — shell out to `wandr-launch wandr-screen on|off`
                // (task 83 launcher drops root→system). Either way, failure is
                // logged + ignored (never fatal — a stuck call is worse than a
                // missed blank).
                apply_display_power(on);
            }
            Effect::SetSensor { kind, on, rate_hz } => {
                // Task 77 — the sensors module's enable-on-demand decision. The
                // HAL driver thread (Step 3) performs the actual enable/disable;
                // until it's wired this logs the contract so the policy is
                // observable on the bus.
                sensor_driver::set_sensor(kind, on, rate_hz);
            }
            Effect::SetBacklight { level, sensor } => {
                // Task 86 — the power module's auto-brightness (ambient-light curve,
                // sensor=true → BrightnessMode SENSOR) or a manual override
                // (sensor=false → USER). Applied via the Lights HAL (sysfs fallback).
                apply_backlight_mode(level, sensor);
            }
            Effect::Launch { app_id, kind } => {
                // Arbiter Inc. 3c — the alarm module emits this to wake a dead
                // owner. Launch via the zygote + register the surface so the next
                // tick (owner now up) delivers `alarm-fired`. Caller holds
                // arbiter_lock but NOT the store lock (effects run post-dispatch),
                // so the state::insert / model_put_surface re-lock is safe.
                use wandr_arbiter_core::{LaunchKind, Role};
                let result = match kind {
                    LaunchKind::Gui => zygote_client::launch_gui(&app_id),
                    LaunchKind::GuiOverlay => zygote_client::launch_gui_overlay(&app_id),
                    LaunchKind::Headless => zygote_client::launch(&app_id),
                };
                match result {
                    Ok(pid) => {
                        state::insert(AppState {
                            app_id: app_id.clone(),
                            pid,
                            launched_at: SystemTime::now(),
                            launched_mono: Instant::now(),
                        });
                        let role = if kind == LaunchKind::Headless { Role::Headless } else { Role::Background };
                        model_put_surface(pid, &app_id, role);
                        // M1 (Signal bg-receipt) — a wake-launch (alarm/keep-alive)
                        // must NOT steal the foreground: `model_put_surface` only
                        // updates the model, so without this the fresh GUI surface
                        // comes up at the host default (visible) over the launcher.
                        // `apply_role(Background)` SIGUSR1s the child → it starts
                        // hidden, mirroring cmd_launch's GuiOverlay "start hidden"
                        // path. Safe immediately post-launch (the zygote ack means
                        // the child has installed its role handler); a no-op for
                        // Headless (no SF surface).
                        apply_role(pid, role);
                        log::info!("arbiter: Launch effect — {app_id} kind={kind:?} → pid {pid} (hidden/background)");
                    }
                    Err(e) => log::warn!("arbiter: Launch effect {app_id} kind={kind:?} failed: {e:#}"),
                }
            }
        }
    }
}

/// Map a surface [`Role`] to the host mechanism (the ONLY place Role→OS lives).
/// Modules emit `Effect::SetRole`; this performs the signal + OOM-score +
/// present, keeping `unsafe libc::kill` / `/proc` writes out of the modules.
fn apply_role(pid: i32, role: wandr_arbiter_core::Role) {
    use wandr_arbiter_core::Role;
    match role {
        // Keyguard reuses the foreground mechanism (show + focus + present). It's
        // launched at a high layer (--standalone-overlay-lock), so SIGUSR2 shows it
        // above the app+nav (below the status bar) + grabs focus; the covered app
        // is demoted to Background by the keyguard module so it doesn't fight back.
        Role::Foreground | Role::Overlay | Role::Lockscreen => {
            send_role_signal(pid, /*foreground=*/ true);
            write_oom_score(pid, OOM_FG);
            send_present(pid);
        }
        Role::OverlayBehind => {
            send_overlay_behind_signal(pid);
            write_oom_score(pid, OOM_FG);
        }
        Role::Background => {
            send_role_signal(pid, /*foreground=*/ false);
            write_oom_score(pid, OOM_BG);
        }
        Role::Chrome | Role::Headless => {}
    }
}

// ─── Surface-model boot seeding (task 74) ─────────────────────────────
//
// The surface/role model is the sole arbiter state (Step D). At daemon startup
// it is seeded once from the restored registry: every re-attached app starts
// `Background`; the persisted foreground app is marked `Foreground` (no signal —
// it is already running in that state). Thereafter the model is maintained
// incrementally by the write-through helpers below; it is never re-derived.

/// Seed the primary display's surface stack from the restored registry.
fn seed_surface_model(restored_fg: Option<String>) {
    use wandr_arbiter_core::Role;
    let mut store = core_store().lock().unwrap_or_else(|e| e.into_inner());
    // Read the registry from the *held* store — NOT via `state::snapshot()`,
    // which (now Store-backed, task 74 C1) would re-lock core_store and deadlock.
    let apps = store.apps_snapshot();
    let ds = store.display_mut(PRIMARY_DISPLAY);
    ds.clear();
    for app in apps {
        let role = if restored_fg.as_deref() == Some(app.app_id.as_str()) {
            Role::Foreground
        } else {
            Role::Background
        };
        ds.put_surface(app.pid, app.app_id, role);
    }
}

// ── model accessors used by the binary (task 74) ──────────────────────
//
// Only the few the binary still needs outside the modules: seeding/launch put a
// surface, persistence + the home-fallback read the foreground slot, and the
// device trace reads visible_app. The orchestration reads/writes moved into
// `wandr-arbiter-shell` (which uses `ctx.store` directly).

fn model_put_surface(pid: i32, app_id: &str, role: wandr_arbiter_core::Role) {
    let mut store = core_store().lock().unwrap_or_else(|e| e.into_inner());
    store.display_mut(PRIMARY_DISPLAY).put_surface(pid, app_id, role);
}
fn model_visible_app() -> Option<i32> {
    let store = core_store().lock().unwrap_or_else(|e| e.into_inner());
    store.display(PRIMARY_DISPLAY).and_then(|d| d.visible_app())
}
/// `(pid, app_id)` of the foreground *slot* (the `Overlay` surface during a
/// split, else the `Foreground` surface) — used by persistence + the death-path
/// home-fallback gate.
fn model_foreground_slot() -> Option<(i32, String)> {
    let store = core_store().lock().unwrap_or_else(|e| e.into_inner());
    store
        .display(PRIMARY_DISPLAY)
        .and_then(|d| d.foreground_slot())
        .map(|s| (s.pid, s.app_id.clone()))
}

/// Log the visible-app derivation whenever it changes — a lightweight device
/// trace of the model's single source of truth (the Step A–C parity canary was
/// retired with the legacy singletons in Step D).
fn log_visible_app() {
    let model = model_visible_app();
    static LAST: OnceLock<Mutex<Option<Option<i32>>>> = OnceLock::new();
    let cell = LAST.get_or_init(|| Mutex::new(None));
    let mut last = cell.lock().unwrap_or_else(|e| e.into_inner());
    if *last != Some(model) {
        log::info!("arbiter: surface-model — visible_app={model:?}");
        *last = Some(model);
    }
}

/// Per-host control socket path for a given child pid. Symmetric with
/// `wandr-host/src/ime_inbound.rs::spawn_listener`.
fn host_sock_path(pid: i32) -> String {
    format!("/data/local/tmp/wandr-host-{pid}.sock")
}

/// The wandr-inputflinger window-feed socket (task 84) — resolved from
/// `WANDR_INPUTFLINGER_SOCK`, else the canonical default. Symmetric with the C++
/// side's resolution (arg > env > default): one named source, overridable.
///
/// ABSTRACT namespace by default (leading `@`): wandr-inputflinger binds it as uid
/// system, which can't create a file under `/data/local/tmp`; the abstract
/// namespace sidesteps filesystem perms entirely. A `@name` ⇒ abstract `\0name`.
fn inputflinger_sock_path() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| {
        std::env::var("WANDR_INPUTFLINGER_SOCK")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "@wandr-inputflinger".to_string())
    })
    .as_str()
}

/// Connect to the wandr-inputflinger socket, honoring the abstract-namespace `@`
/// convention (mirrors the C++ `fill_un_addr`). Filesystem paths take the plain
/// `UnixStream::connect`.
fn connect_inputflinger(path: &str) -> std::io::Result<UnixStream> {
    if let Some(name) = path.strip_prefix('@') {
        // The abstract-namespace extension trait lives under a per-OS module
        // (android on-device, linux for desktop unit builds); both expose
        // `from_abstract_name`.
        #[cfg(target_os = "android")]
        use std::os::android::net::SocketAddrExt;
        #[cfg(target_os = "linux")]
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::SocketAddr;
        let addr = SocketAddr::from_abstract_name(name.as_bytes())?;
        UnixStream::connect_addr(&addr)
    } else {
        UnixStream::connect(path)
    }
}

/// Task 84 — author + push the input-window list to wandr-inputflinger so the
/// standalone `InputDispatcher` can hit-test app touches under ART-off (with
/// system_server gone, SurfaceFlinger never pushes WindowInfos to it, so every
/// touch was dropped). The arbiter is the WMS: [`wandr_arbiter_wm::input_window_block`]
/// derives the ordered rects from the surface model + geometry. Diffed against the
/// last-sent block so an unchanged model is a no-op (this runs after every command
/// + child-exit). Gated on `--no-art` — the only time wandr-inputflinger owns
/// dispatch; a normal-ART run leaves windows to SurfaceFlinger. Caller holds
/// `arbiter_lock`.
fn push_input_windows() {
    if !no_art() {
        return;
    }
    let block = {
        let store = core_store().lock().unwrap_or_else(|e| e.into_inner());
        wandr_arbiter_wm::input_window_block(&store, PRIMARY_DISPLAY)
    };
    let Some(block) = block else { return };
    static LAST: OnceLock<Mutex<String>> = OnceLock::new();
    let cell = LAST.get_or_init(|| Mutex::new(String::new()));
    {
        let mut last = cell.lock().unwrap_or_else(|e| e.into_inner());
        if *last == block {
            return; // model unchanged → nothing to push
        }
        *last = block.clone();
    }
    let sock = inputflinger_sock_path();
    log::debug!("arbiter: input-windows push →\n{}", block.trim_end());
    match connect_inputflinger(sock) {
        Ok(mut stream) => {
            use std::io::Write;
            let _ = stream.write_all(block.as_bytes());
            let _ = stream.flush();
            // Half-close so the C++ reader sees EOF + feeds the (complete) block.
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
        Err(e) => log::debug!("arbiter: input-windows → {sock} failed: {e:#}"),
    }
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

    // Task 74 C — the shell module owns the teardown policy: it tears down an
    // overlay split this pid was part of, removes the app from the registry, and
    // prunes the surface. Inject the event + run the effects (overlay role
    // signals) the module emits.
    bus_emit(Event::SurfaceRemoved { pid });

    // Socket cleanup (RC2) — mechanism, stays in the binary.
    remove_host_socket(pid);

    // Task 57 — never leave the screen empty. If pruning left no foreground and a
    // home app is designated, bring home forward (foregrounding the still-alive
    // backgrounded launcher, or relaunching it if the launcher itself died). This
    // home-fallback launch is zygote-coupled, so it stays in the binary.
    if model_foreground_slot().is_none() && state::current_home().is_some() {
        log::info!("arbiter: foreground empty after pid {pid} exit — falling back to home");
        ensure_home_foreground();
    }

    if let Err(e) = state::save_to(Path::new(ARBITER_STATE_PATH), model_foreground_slot().as_ref().map(|(_, id)| id.as_str())) {
        log::warn!("arbiter: on_child_exit state save failed: {e:#}");
    }
    log_visible_app();
    // Task 84 — a surface vanished; re-author the input-window list (drops its
    // window; promotes home's if the fallback ran above).
    push_input_windows();
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
/// Arbiter Inc. 3c — the alarm timer thread. Every ~1 s, if any alarms exist,
/// inject [`Event::AlarmTick`] (stamped with the current unix-ms) so the alarm
/// module fires the due ones. Skips the bus entirely when no alarms are
/// scheduled (no idle churn). Takes `arbiter_lock` to serialize with command
/// handlers + the death watchers.
fn spawn_alarm_timer() {
    std::thread::Builder::new()
        .name("arbiter-alarm-timer".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_millis(1000));
            let has = core_store()
                .lock()
                .map(|s| s.has_alarms())
                .unwrap_or(false);
            if !has {
                continue;
            }
            let now_ms = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let _guard = arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());
            bus_emit(Event::AlarmTick { now_ms });
        })
        .expect("spawn alarm timer thread");
}

/// True with the Java framework stopped (set by `run-hybrid-stack.sh --no-art`).
/// Switches the arbiter to owning display power + screen state itself (the SF
/// sysprop the poller reads is stale once system_server is gone).
fn no_art() -> bool {
    std::env::var_os("WANDR_NO_ART").is_some()
}

/// Resolve a binary deployed alongside `wandr-arbiter` (e.g. `wandr-launch`,
/// `wandr-screen`) — derived from this exe's own dir so it follows the deploy
/// location, falling back to the canonical device dir.
fn sibling_bin(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .unwrap_or_else(|| std::path::PathBuf::from(format!("/data/local/tmp/{name}")))
}

/// Apply a SurfaceFlinger panel-power change (task 78/81). ART-up: inline
/// wandr-hal-display (the arbiter is root and SF short-circuits its permission
/// check). ART-off: bare root HANGS on `ACCESS_SURFACE_FLINGER`, so run as uid
/// system via `wandr-launch wandr-screen on|off` (task 83). Never fatal.
fn apply_display_power(on: bool) {
    if no_art() {
        let launch = sibling_bin("wandr-launch");
        let screen = sibling_bin("wandr-screen");
        match std::process::Command::new(&launch)
            .arg(&screen)
            .arg(if on { "on" } else { "off" })
            .status()
        {
            Ok(s) => log::info!("arbiter: SetDisplayPower on={on} via wandr-launch → {s}"),
            Err(e) => log::warn!(
                "arbiter: SetDisplayPower on={on} via {} failed: {e:#}",
                launch.display()
            ),
        }
        apply_backlight(if on { default_on_fraction() } else { 0.0 });
    } else {
        let ok = wandr_hal_display::set_display_power(on);
        log::info!("arbiter: SetDisplayPower on={on} applied={ok}");
    }
}

const BACKLIGHT_NODE: &str = "/sys/class/leds/lcd-backlight/brightness";
const BACKLIGHT_MAX_NODE: &str = "/sys/class/leds/lcd-backlight/max_brightness";

/// The default-on backlight fraction — what `apply_display_power(on)` sets at boot /
/// wake until the first ambient-light reading refines it (task 86). Was a raw
/// `150` in task 84/85; expressed as a fraction now so it's panel-range-independent
/// (≈ the same visible level on a 255-step panel). `WANDR_BACKLIGHT_DEFAULT` (0.0–1.0)
/// overrides — the ONE named default-brightness source.
const DEFAULT_ON_FRACTION: f32 = 0.6;

fn default_on_fraction() -> f32 {
    std::env::var("WANDR_BACKLIGHT_DEFAULT")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|f| (0.0..=1.0).contains(f))
        .unwrap_or(DEFAULT_ON_FRACTION)
}

/// The panel's raw backlight range (`max_brightness`), read once and cached. Lets
/// `apply_backlight` map a normalized fraction → raw units without hardcoding the
/// panel's depth. `WANDR_BACKLIGHT_MAX` overrides; fallback 255 (the near-universal
/// 8-bit range) if the node is absent.
fn backlight_max() -> u32 {
    use std::sync::OnceLock;
    static MAX: OnceLock<u32> = OnceLock::new();
    *MAX.get_or_init(|| {
        if let Some(n) = std::env::var("WANDR_BACKLIGHT_MAX").ok().and_then(|s| s.parse::<u32>().ok()) {
            if n > 0 {
                return n;
            }
        }
        std::fs::read_to_string(BACKLIGHT_MAX_NODE)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(255)
    })
}

/// Apply a normalized backlight fraction (0.0–1.0) to the primary panel — the
/// task-86 auto-brightness applier (and the on/off default from
/// `apply_display_power`). Under `--no-art` there is no DisplayManager, so
/// `setPowerMode(ON)` lights the panel but the backlight stays at whatever it held
/// (often 0 → the screen renders but is physically invisible — this bit us in
/// task-84 bring-up); the arbiter, the sole display-power authority under
/// `--no-art`, owns it.
///
/// Writes the **sysfs** node — exactly what the Lights HAL writes on this device, so
/// it's the same endpoint, not a hack. We deliberately do NOT call SurfaceFlinger
/// `setDisplayBrightness` inline here: the arbiter runs as plain root and a bare-root
/// SF call HANGS on the permission check under `--no-art` (the reason
/// `apply_display_power` shells out to `wandr-launch wandr-screen`), and on taimen
/// `setDisplayBrightness` is `IllegalState` regardless (HWC unsupported, task-86
/// device-confirmed). The SF path stays reachable + tested via `wandr-screen
/// brightness <f>` (uid system) for a future device whose HWC supports it.
///
/// Only under `--no-art` — with the Java framework up, DisplayManager owns the
/// backlight and we must not fight it. Node + max range env-overridable; best-effort.
/// Apply a normalized backlight fraction. `sensor` tags the change as
/// auto-brightness (BrightnessMode SENSOR) vs a manual/on-off USER set.
///
/// Prefers the **Lights HAL** (`ILights::setLightState`, wandr-hal-lights) — the
/// proper Android endpoint the dead `LightsService`/`DisplayPowerController` used to
/// drive; the vendor HAL owns the actual write (and any hardware sensor-aware
/// handling). Falls back to the raw sysfs node only if the HAL is unavailable
/// (SELinux-blocked / absent). Caller (the power module) already dedups + debounces
/// + ramps, so this just applies.
fn apply_backlight(frac: f32) {
    // The bare entry is the on/off boot/wake default (a fixed level from
    // apply_display_power), not a sensor reading → USER mode.
    apply_backlight_mode(frac, false)
}

fn apply_backlight_mode(frac: f32, sensor: bool) {
    if !no_art() {
        return; // ART-up: DisplayManager owns the backlight; don't fight it.
    }
    let frac = frac.clamp(0.0, 1.0);
    // The Lights HAL is the proper path; try it first (cached service handle).
    if wandr_hal_lights::set_backlight(frac, sensor) {
        log::info!("arbiter: backlight → frac={frac:.3} via ILights (sensor={sensor})");
        return;
    }
    // Fallback: raw sysfs node (what the vendor lights HAL writes anyway), e.g. if
    // the HAL is SELinux-blocked from this context.
    let max = backlight_max();
    let level = (frac * max as f32).round() as u32;
    let path = std::env::var("WANDR_BACKLIGHT_PATH").unwrap_or_else(|_| BACKLIGHT_NODE.to_string());
    match std::fs::write(&path, level.to_string()) {
        Ok(()) => log::info!("arbiter: backlight → {level}/{max} (frac={frac:.3}, sysfs {path})"),
        Err(e) => log::debug!("arbiter: backlight {level} → {path} failed: {e}"),
    }
}

/// PowerManager — poll the display power state (`debug.tracing.screen_state`, the
/// SF-sourced survivor sysprop; NOT IPowerManager, which is the system_server ART
/// layer we drop) every ~2 s and inject [`Event::ScreenState`] so the power module
/// applies the doze grace + fans the cadence to hosts. The arbiter is the single
/// power authority; the poll interval doubles as the grace tick. `On`=2 / `Vr`=5
/// are live; everything else (Off=1, Doze=3, …) is non-live.
fn spawn_screen_poller() {
    fn read_live() -> Option<bool> {
        let out = std::process::Command::new("/system/bin/getprop")
            .arg("debug.tracing.screen_state")
            .output()
            .ok()?;
        let v: i32 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(matches!(v, 2 | 5)) // Display.STATE_ON | STATE_VR
    }
    std::thread::Builder::new()
        .name("arbiter-screen-poller".into())
        .spawn(|| {
            let mut last: Option<bool> = None;
            loop {
                std::thread::sleep(Duration::from_millis(2000));
                let Some(live) = read_live() else { continue };
                // Always emit while non-live (the power module re-checks the grace
                // each tick); when live, only emit on the change to live (no idle
                // churn while the screen stays on).
                if live && last == Some(true) {
                    continue;
                }
                last = Some(live);
                let _guard = arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());
                bus_emit(Event::ScreenState { live });
            }
        })
        .expect("spawn screen poller thread");
}

/// Inactivity ticker (PowerManager screen-off-timeout role, task 86 follow-on) —
/// only under `--no-art` (no PMS). Periodically injects [`Event::IdleTick`] so the
/// power module re-checks input-idle elapsed vs the screen-off timeout and sleeps the
/// panel when exceeded. The tick interval bounds how late the sleep can fire (a few
/// seconds past the timeout — fine), and is cheap (one lock + bus dispatch). The
/// activity side is `user-activity`, poked by wandr-inputflinger's dispatcher policy.
fn spawn_inactivity_timer() {
    const TICK: Duration = Duration::from_secs(5);
    std::thread::Builder::new()
        .name("arbiter-inactivity-timer".into())
        .spawn(|| loop {
            std::thread::sleep(TICK);
            let _guard = arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());
            bus_emit(Event::IdleTick);
        })
        .expect("spawn inactivity timer thread");
}

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
/// lifecycle stays `Resumed`. The receiving wandr-host's
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



/// Task 71 (WMS-authority step) — tell a host its surface is now visible and it
/// should paint a fresh frame. The arbiter is the visibility authority; this
/// makes "repaint on show" an explicit push instead of leaving the host to infer
/// it from the async role signal (which left re-shown surfaces present-but-empty).
/// Fire-and-forget; a host with no live socket (just-forked / dead) simply misses
/// it and its own role-transition dirty-frame still covers the common case.
fn send_present(pid: i32) {
    let sock = format!("/data/local/tmp/wandr-host-{pid}.sock");
    if let Err(e) = deliver_to_host(&sock, "present\n") {
        log::debug!("arbiter: present → {sock} failed: {e:#}");
    }
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
    if model_foreground_slot().as_ref().map(|(_, id)| id.as_str()) == Some(home.as_str()) {
        return;
    }
    // Already running (survived as a backgrounded process) → just promote via the
    // shell module's foreground policy.
    if let Some(s) = state::get(&home) {
        log::info!("arbiter: home {home} already running (pid {}) — foregrounding", s.pid);
        dispatch_foreground(&home);
        return;
    }
    // Not running — launch it as a fullscreen GUI app (zygote, binary-side), then
    // dispatch foreground to the module.
    match zygote_client::launch_gui(&home) {
        Ok(pid) => {
            state::insert(AppState {
                app_id: home.clone(),
                pid,
                launched_at: SystemTime::now(),
                launched_mono: Instant::now(),
            });
            model_put_surface(pid, &home, wandr_arbiter_core::Role::Background);
            dispatch_foreground(&home);
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
    // Idempotent for GUI apps: if this app is already running, bring it to the
    // FRONT instead of spawning a second instance — tapping a running app's icon
    // in the launcher should resume it (Android's singleTask-ish "open"), not
    // fork a duplicate. (A stale/dead entry falls through to a fresh launch.)
    if kind == LaunchKind::Gui && !app_id.is_empty() {
        if let Some(s) = state::get(app_id) {
            if state::pid_alive(s.pid) {
                dispatch_foreground(&s.app_id);
                writeln!(stream, "OK pid={} app={app_id} foregrounded", s.pid)?;
                log::info!(
                    "wandr-arbiter: {app_id} already running (pid {}) — foregrounding instead of relaunch",
                    s.pid
                );
                return Ok(());
            }
        }
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
            // Task 74 — register the surface. GUI/overlay start Background;
            // promotion / overlay-engage sets the live role. Headless has no SF
            // surface (never visible).
            model_put_surface(pid, &key, match kind {
                LaunchKind::Headless => wandr_arbiter_core::Role::Headless,
                _ => wandr_arbiter_core::Role::Background,
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
                dispatch_foreground(&key);
            } else if kind == LaunchKind::GuiOverlay {
                // Start the overlay HIDDEN — its default Foreground role
                // would otherwise show it immediately (e.g. the IME
                // keyboard visible on the home screen). The auto-tie in
                // cmd_attach_editor (promote_to_overlay) shows it when an
                // editor actually focuses; detach hides it again.
                send_role_signal(pid, /*foreground=*/ false);
            }
            writeln!(stream, "OK pid={pid} app={key}")?;
            log::info!("wandr-arbiter: launched {key} → pid {pid} kind={kind:?}");
            Ok(())
        }
        Err(e) => {
            writeln!(stream, "ERR launch-failed {e:#}")?;
            log::warn!("wandr-arbiter: launch {app_id} failed: {e:#}");
            Ok(())
        }
    }
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
            log::info!("wandr-arbiter: preload {app_id} → {reply}");
            Ok(())
        }
        Err(e) => {
            writeln!(stream, "ERR preload-failed {app_id}: {e:#}")?;
            log::warn!("wandr-arbiter: preload {app_id} failed: {e:#}");
            Ok(())
        }
    }
}
