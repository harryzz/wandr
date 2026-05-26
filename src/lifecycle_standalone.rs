//! Lifecycle plumbing for the standalone (no-`NativeActivity`) path —
//! task 33 Step 5. Three small concerns bundled here:
//!
//!   1. **Signal-driven shutdown.** SIGTERM and SIGINT set an atomic flag
//!      the render loop polls each iteration. On set, the loop breaks
//!      and the caller fires `Destroyed` into the guest before returning.
//!   2. **Screen on/off → Paused/Resumed.** A background thread polls
//!      `getprop debug.tracing.screen_state` (Android `Display.STATE_OFF=1`,
//!      `STATE_ON=2`). State changes are forwarded over a `mpsc::Receiver`
//!      the render loop drains each iteration. Graceful no-op if the
//!      property is absent (older devices, tracing disabled).
//!   3. **Crash resilience.** A panic hook writes a JSON marker to
//!      `/data/local/tmp/wart-host-crash.json` so the next launch (or a
//!      future init.rc service) can surface what happened. On clean exit
//!      we remove the marker. On startup we log + remove a prior marker.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

const CRASH_MARKER: &str = "/data/local/tmp/wart-host-crash.json";

// ── Signals ──────────────────────────────────────────────────────────

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn shutdown_handler(_sig: libc::c_int) {
    // Async-signal-safe: no allocation, no logging, no locks.
    SHUTDOWN.store(true, Ordering::SeqCst);
}

pub fn install_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = shutdown_handler as *const () as usize;
        // SA_RESTART so a SIGTERM mid-`read()` on the input fd doesn't
        // poison the polling loop; the handler just sets the flag and
        // the loop's next iteration sees it.
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);

        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            if libc::sigaction(sig, &sa, std::ptr::null_mut()) != 0 {
                log::warn!(
                    "standalone: sigaction({sig}) failed: errno={}",
                    *libc::__errno()
                );
            }
        }
    }
}

pub fn should_shutdown() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

// ── Screen state ─────────────────────────────────────────────────────

/// Mirrors `android.view.Display.State` ordinals (the values returned by
/// the `debug.tracing.screen_state` sysprop).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScreenState {
    Unknown    = 0,
    Off        = 1,
    On         = 2,
    Doze       = 3,
    DozeSuspend= 4,
    Vr         = 5,
    OnSuspend  = 6,
}

impl ScreenState {
    fn from_prop(s: &str) -> Self {
        match s.trim().parse::<i32>().unwrap_or(0) {
            1 => Self::Off,
            2 => Self::On,
            3 => Self::Doze,
            4 => Self::DozeSuspend,
            5 => Self::Vr,
            6 => Self::OnSuspend,
            _ => Self::Unknown,
        }
    }

    /// Should the guest be live (rendering, responding) in this state?
    pub fn is_live(self) -> bool {
        matches!(self, Self::On | Self::Vr)
    }
}

fn read_screen_state_prop() -> Option<ScreenState> {
    // `__system_property_get` would be more direct but requires libbase; an
    // exec of `getprop` is ~ms-scale and only fires from the poll thread.
    let out = std::process::Command::new("/system/bin/getprop")
        .arg("debug.tracing.screen_state")
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(ScreenState::from_prop(trimmed))
    }
}

/// Spawn a watcher thread that polls the screen-state sysprop and pushes
/// transitions onto the returned channel. Returns `None` if the property
/// is absent on this device (no watcher started — caller treats screen
/// as always-on).
pub fn spawn_screen_state_watcher() -> Option<mpsc::Receiver<ScreenState>> {
    let initial = read_screen_state_prop()?;
    log::info!("standalone: screen-state watcher up (initial={initial:?})");
    let (tx, rx) = mpsc::channel();
    // Seed the channel so the render loop receives the initial state on
    // its first drain; this lets the cold-start path fall straight to
    // Paused if the panel happens to be off at launch.
    let _ = tx.send(initial);
    std::thread::Builder::new()
        .name("screen-state-watcher".into())
        .spawn(move || {
            let mut last = initial;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let Some(cur) = read_screen_state_prop() else {
                    continue;
                };
                if cur != last {
                    log::info!("standalone: screen-state {last:?} -> {cur:?}");
                    if tx.send(cur).is_err() {
                        break; // receiver gone
                    }
                    last = cur;
                }
            }
        })
        .ok()?;
    Some(rx)
}

// ── Crash marker ─────────────────────────────────────────────────────

pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Forward to the prior hook first so logcat still gets the
        // formatted panic message + backtrace via android_logger.
        prev(info);
        write_crash_marker(&format!("{info}"));
    }));
}

fn write_crash_marker(panic_msg: &str) {
    // Best-effort, no `?` — we're inside a panic.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let escaped = panic_msg
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let body = format!(
        "{{\"ts\":{ts},\"panic\":\"{escaped}\"}}\n"
    );
    let _ = std::fs::write(CRASH_MARKER, body);
}

/// Log + remove a marker left by the prior run (if any).
pub fn drain_prior_crash_marker() {
    let p = Path::new(CRASH_MARKER);
    if !p.exists() { return; }
    match std::fs::read_to_string(p) {
        Ok(s) => log::error!("standalone: prior run crashed — {}", s.trim()),
        Err(e) => log::warn!("standalone: prior crash marker unreadable: {e}"),
    }
    let _ = std::fs::remove_file(p);
}

/// Remove the marker after a clean exit (no prior crash to report on
/// next launch).
pub fn record_clean_exit() {
    let _ = std::fs::remove_file(CRASH_MARKER);
}
