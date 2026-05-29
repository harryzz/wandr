//! System-shell status data — `my:skiko-gfx/status` WIT impl (task 55).
//!
//! ART-free: clock from the device local time, battery from sysfs — no
//! `system_server` (per [[feedback_no_art_layer_dependencies]]). Consumed
//! by the `war.statusbar` guest, which polls these ~1 Hz and draws the
//! top-overlay strip.

use std::process::Command;

use crate::bindings::my::skiko_gfx::status::Host;

impl Host for crate::HostState {
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
