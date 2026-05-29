//! System-shell status data — `my:skiko-gfx/status` WIT impl (task 55).
//!
//! ART-free: clock from the device local time, battery from sysfs — no
//! `system_server` (per [[feedback_no_art_layer_dependencies]]). Consumed
//! by the `war.statusbar` guest, which polls these ~1 Hz and draws the
//! top-overlay strip.

use std::process::Command;

use crate::bindings::my::skiko_gfx::status::Host;

/// Status-bar strip height in physical pixels — the single source of
/// truth (task 55). The host creates the top-overlay surface at this
/// height, the `status.bar-height()` verb exposes it, and the launcher
/// queries it for its top inset so the two never desync. Tunable at
/// runtime via `WART_STATUSBAR_PX` (default 132). Set it uniformly across
/// the stack (zygote env) so every process agrees.
pub fn status_bar_height_px() -> u32 {
    std::env::var("WART_STATUSBAR_PX")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&h| h > 0)
        .unwrap_or(132)
}

impl Host for crate::HostState {
    fn bar_height(&mut self) -> u32 {
        status_bar_height_px()
    }

    fn clock_text(&mut self) -> String {
        // Local wall-clock. Rust std has no localtime; `date` is a native
        // toolbox/coreutils binary (not ART), so shelling out is ART-free.
        // Called ~1 Hz by the status bar, so the per-call spawn is cheap.
        match Command::new("date").arg("+%H:%M").output() {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            }
            Ok(o) => {
                log::warn!("status: `date` exited {}", o.status);
                String::new()
            }
            Err(e) => {
                log::warn!("status: spawn `date` failed: {e}");
                String::new()
            }
        }
    }

    fn battery_text(&mut self) -> String {
        // sysfs — no binder, no system_server. Path is the standard
        // power_supply class node; absent on some emulators (returns "").
        const CAP: &str = "/sys/class/power_supply/battery/capacity";
        match std::fs::read_to_string(CAP) {
            Ok(s) => {
                let pct = s.trim();
                if pct.is_empty() { String::new() } else { format!("{pct}%") }
            }
            Err(e) => {
                log::debug!("status: read {CAP} failed: {e}");
                String::new()
            }
        }
    }
}
