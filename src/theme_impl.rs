//! System theme — `my:skiko-gfx/theme` WIT impl.
//!
//! v1 reads via `cmd uimode night` (stdout parsing). It's a shell-out
//! per call; current consumers only read once at composition time, so
//! the cost is negligible. If a live-watcher / per-frame poll becomes
//! a thing, switch to caching + a sysprop watcher or rsbinder to
//! `IUiModeManager`.
//!
//! Material You accent reads are deferred — returns 0 (caller picks
//! a fallback palette). Pixel 2 XL stock is pre-Material-You anyway.

use std::process::Command;

use crate::bindings::my::skiko_gfx::theme::{Host, NightMode};

impl Host for crate::HostState {
    fn get_night_mode(&mut self) -> NightMode {
        match read_night_mode_via_cmd() {
            Some(n) => n,
            None => {
                log::warn!("theme: get_night_mode — cmd uimode night failed, returning Auto");
                NightMode::Auto
            }
        }
    }

    fn get_accent_color(&mut self) -> u32 {
        // Material You accent — needs JNI to Resources or a binder
        // call into theme service. Deferred; return 0 = "use fallback".
        0
    }
}

/// Parses output like:
///   Night mode: yes
///   Night mode: no
///   Night mode: auto
fn read_night_mode_via_cmd() -> Option<NightMode> {
    let out = Command::new("cmd").args(["uimode", "night"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let after = s.split(':').nth(1)?.trim().to_ascii_lowercase();
    let mode = match after.as_str() {
        "yes" => NightMode::On,
        "no"  => NightMode::Off,
        _     => NightMode::Auto,
    };
    log::info!("theme: read night-mode={mode:?} (raw={:?})", s.trim());
    Some(mode)
}
