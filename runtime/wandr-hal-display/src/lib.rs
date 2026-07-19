//! wandr-hal-display — shared SurfaceFlinger display-power access (task 78).
//!
//! Wraps `android.gui.ISurfaceComposer` (the `SurfaceFlingerAIDL` service) over
//! rsbinder to drive `setPowerMode` — the proper panel-off (the HWC powers the
//! panel down, deeper than a backlight write). Used by the arbiter's power
//! applier when proximity says "near" during a call. Off-android = no-op stub.
//!
//! Device-confirmed (Pixel 2 XL): the transaction order matches the full upstream
//! interface, the primary display token is obtainable as root, and `setPowerMode`
//! shares `getPhysicalDisplayToken`'s permission gate (which succeeds as root).

/// Android `hal::PowerMode` ints (composer HAL): panel fully off / fully on.
#[cfg(target_os = "android")]
const POWER_MODE_OFF: i32 = 0;
#[cfg(target_os = "android")]
const POWER_MODE_ON: i32 = 2;

#[cfg(target_os = "android")]
#[allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, unused_imports, clippy::all)]
mod binder_aidl {
    include!(concat!(env!("OUT_DIR"), "/sf_bindings.rs"));
}

#[cfg(target_os = "android")]
mod binder_path {
    use super::binder_aidl::android::gui::ISurfaceComposer::ISurfaceComposer;
    use super::{POWER_MODE_OFF, POWER_MODE_ON};
    use std::sync::OnceLock;

    /// Ensure rsbinder's `ProcessState` + thread pool are up before any binder
    /// call (same as wandr-sensors-client — idempotent per process).
    fn ensure_process_state() {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            if !std::path::Path::new("/dev/binder").exists() {
                log::warn!("wandr-hal-display: /dev/binder absent — display power unavailable");
                return;
            }
            if rsbinder::ProcessState::init_default().is_ok() {
                rsbinder::ProcessState::start_thread_pool();
            }
        });
    }

    fn service() -> Option<&'static rsbinder::Strong<dyn ISurfaceComposer>> {
        static SVC: OnceLock<Option<rsbinder::Strong<dyn ISurfaceComposer>>> = OnceLock::new();
        SVC.get_or_init(|| {
            ensure_process_state();
            rsbinder::hub::get_interface::<dyn ISurfaceComposer>("SurfaceFlingerAIDL").ok()
        })
        .as_ref()
    }

    /// Set the primary display's power. Resolves the display token fresh each call
    /// (cheap; called only on a proximity transition) — derives the id from
    /// `getPhysicalDisplayIds` (no hardcoded display 0).
    pub fn set_display_power(on: bool) -> bool {
        let Some(svc) = service() else {
            log::warn!("wandr-hal-display: SurfaceFlingerAIDL unavailable");
            return false;
        };
        let ids = match svc.r#getPhysicalDisplayIds() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("wandr-hal-display: getPhysicalDisplayIds err={e:?}");
                return false;
            }
        };
        let Some(&id) = ids.first() else {
            log::warn!("wandr-hal-display: no physical display");
            return false;
        };
        let token = match svc.r#getPhysicalDisplayToken(id) {
            Ok(Some(t)) => t,
            Ok(None) => {
                log::warn!("wandr-hal-display: null display token for id {id}");
                return false;
            }
            Err(e) => {
                log::warn!("wandr-hal-display: getPhysicalDisplayToken err={e:?}");
                return false;
            }
        };
        let mode = if on { POWER_MODE_ON } else { POWER_MODE_OFF };
        match svc.r#setPowerMode(&token, mode) {
            Ok(_) => {
                log::info!("wandr-hal-display: display power {} (mode {mode})", if on { "ON" } else { "OFF" });
                true
            }
            Err(e) => {
                log::warn!("wandr-hal-display: setPowerMode({mode}) err={e:?}");
                false
            }
        }
    }

    /// Set the primary display's brightness via SurfaceFlinger `setDisplayBrightness`
    /// (the modern composer-driven path, 0.0–1.0; `displayBrightness < 0` = backlight
    /// off). The call is fully reachable as uid system (no permission issue), but only
    /// works where the HWC supports brightness. Task-86 device-confirmed: on the
    /// Pixel 2 XL (taimen) it returns `IllegalState` — that panel routes brightness
    /// through the Lights HAL / sysfs, so auto-brightness uses the sysfs backlight
    /// there; `set_display_brightness` is the preferred path on devices that support
    /// it. Returns false off-android / on failure.
    pub fn set_display_brightness(level: f32) -> bool {
        use super::binder_aidl::android::gui::DisplayBrightness::DisplayBrightness;
        let Some(svc) = service() else {
            log::warn!("wandr-hal-display: SurfaceFlingerAIDL unavailable");
            return false;
        };
        let ids = match svc.r#getPhysicalDisplayIds() {
            Ok(v) => v,
            Err(e) => { log::warn!("wandr-hal-display: getPhysicalDisplayIds err={e:?}"); return false; }
        };
        let Some(&id) = ids.first() else { log::warn!("wandr-hal-display: no physical display"); return false; };
        let token = match svc.r#getPhysicalDisplayToken(id) {
            Ok(Some(t)) => t,
            Ok(None) => { log::warn!("wandr-hal-display: null display token"); return false; }
            Err(e) => { log::warn!("wandr-hal-display: getPhysicalDisplayToken err={e:?}"); return false; }
        };
        let b = DisplayBrightness {
            r#sdrWhitePoint: level,
            r#sdrWhitePointNits: -1.0,
            r#displayBrightness: level,
            r#displayBrightnessNits: -1.0,
        };
        match svc.r#setDisplayBrightness(&token, &b) {
            Ok(_) => {
                log::info!("wandr-hal-display: setDisplayBrightness({level}) ok");
                true
            }
            Err(e) => {
                // Device-confirmed on taimen: this returns IllegalState — the panel
                // doesn't support composer-driven brightness (brightness routes through
                // the Lights HAL / sysfs here). Reachable + no permission issue, just
                // unsupported by the HWC; callers fall back to the sysfs backlight.
                log::warn!("wandr-hal-display: setDisplayBrightness({level}) err={e:?}");
                false
            }
        }
    }
}

/// Turn the primary display panel on or off (SurfaceFlinger `setPowerMode`).
/// Returns false off-android or on any failure (caller treats as a no-op).
pub fn set_display_power(on: bool) -> bool {
    #[cfg(target_os = "android")]
    {
        binder_path::set_display_power(on)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = on;
        false
    }
}

/// Set the primary display's brightness (0.0–1.0) via SurfaceFlinger
/// `setDisplayBrightness`. Returns false off-android or on any failure.
pub fn set_display_brightness(level: f32) -> bool {
    #[cfg(target_os = "android")]
    {
        binder_path::set_display_brightness(level)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = level;
        false
    }
}
