//! wart-hal-lights — shared `android.hardware.light` ILights access.
//!
//! Drives the **Lights HAL** `setLightState(BACKLIGHT, …)` — the proper Android
//! brightness endpoint. The framework layers that used to call it
//! (`DisplayPowerController` ramp/rate-limit, `LightsService`) are Java in
//! `system_server` and die under `--no-art`; the HAL itself is a native vendor
//! service that survives, so the arbiter calls it directly (like it drives the
//! sensor/SF HALs). Brightness is encoded as gray ARGB and tagged with a
//! `BrightnessMode` (USER vs SENSOR) exactly as `LightsService.setLightLocked`
//! does, so the vendor HAL can apply sensor-aware handling. Off-android = no-op.

/// LightType.BACKLIGHT ordinal (android.hardware.light.LightType).
#[cfg(target_os = "android")]
const LIGHT_TYPE_BACKLIGHT: i8 = 0;
/// BrightnessMode ordinals (android.hardware.light.BrightnessMode).
#[cfg(target_os = "android")]
const BRIGHTNESS_MODE_USER: i8 = 0;
#[cfg(target_os = "android")]
const BRIGHTNESS_MODE_SENSOR: i8 = 1;

#[cfg(target_os = "android")]
#[allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, unused_imports, clippy::all)]
mod binder_aidl {
    include!(concat!(env!("OUT_DIR"), "/light_bindings.rs"));
}

#[cfg(target_os = "android")]
mod binder_path {
    use super::binder_aidl::android::hardware::light::{
        BrightnessMode::BrightnessMode, FlashMode::FlashMode, HwLightState::HwLightState,
        ILights::ILights, LightType::LightType,
    };
    use super::{BRIGHTNESS_MODE_SENSOR, BRIGHTNESS_MODE_USER, LIGHT_TYPE_BACKLIGHT};
    use std::sync::OnceLock;

    /// Ensure rsbinder's `ProcessState` + thread pool are up before any binder call
    /// (same as wart-hal-display/sensors — idempotent per process).
    fn ensure_process_state() {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            if !std::path::Path::new("/dev/binder").exists() {
                log::warn!("wart-hal-lights: /dev/binder absent — lights HAL unavailable");
                return;
            }
            if rsbinder::ProcessState::init_default().is_ok() {
                rsbinder::ProcessState::start_thread_pool();
            }
        });
    }

    fn service() -> Option<&'static rsbinder::Strong<dyn ILights>> {
        static SVC: OnceLock<Option<rsbinder::Strong<dyn ILights>>> = OnceLock::new();
        SVC.get_or_init(|| {
            ensure_process_state();
            // Registered as "android.hardware.light.ILights/default" on Android 11+
            // (the vendor /vendor/bin/hw/android.hardware.light-service).
            rsbinder::hub::get_interface::<dyn ILights>("android.hardware.light.ILights/default").ok()
        })
        .as_ref()
    }

    /// Set the panel backlight to `frac` (0.0–1.0) via the Lights HAL. `sensor`
    /// selects BrightnessMode SENSOR (auto-brightness) vs USER (manual/on-off).
    /// Encodes brightness as gray ARGB `0xFF | l<<16 | l<<8 | l` (the
    /// `LightsService.setBrightness` formula). Returns true if any BACKLIGHT light
    /// accepted it. Caller should dedup (skip unchanged values).
    pub fn set_backlight(frac: f32, sensor: bool) -> bool {
        let Some(svc) = service() else { return false };
        let lights = match svc.r#getLights() {
            Ok(v) => v,
            Err(e) => {
                log::debug!("wart-hal-lights: getLights failed: {e:?}");
                return false;
            }
        };
        let l = (frac.clamp(0.0, 1.0) * 255.0).round() as i32;
        let color = (0xff00_0000u32 as i32) | (l << 16) | (l << 8) | l;
        let state = HwLightState {
            r#color: color,
            r#flashMode: FlashMode(0), // NONE
            r#flashOnMs: 0,
            r#flashOffMs: 0,
            r#brightnessMode: BrightnessMode(if sensor {
                BRIGHTNESS_MODE_SENSOR
            } else {
                BRIGHTNESS_MODE_USER
            }),
        };
        let mut any = false;
        for hw in lights.iter().filter(|h| h.r#type == LightType(LIGHT_TYPE_BACKLIGHT)) {
            match svc.r#setLightState(hw.r#id, &state) {
                Ok(()) => any = true,
                Err(e) => log::debug!("wart-hal-lights: setLightState(id={}) failed: {e:?}", hw.r#id),
            }
        }
        any
    }

    /// Whether the HAL is reachable and exposes a BACKLIGHT light (so the caller can
    /// fall back to sysfs when it isn't — e.g. SELinux blocked, or no such HAL).
    pub fn available() -> bool {
        let Some(svc) = service() else { return false };
        matches!(svc.r#getLights(), Ok(lights)
            if lights.iter().any(|h| h.r#type == LightType(LIGHT_TYPE_BACKLIGHT)))
    }
}

/// Set the panel backlight via the Lights HAL. `sensor` = auto-brightness
/// (BrightnessMode SENSOR) vs manual (USER). Returns false off-android or if the
/// HAL is unavailable (caller falls back to sysfs).
#[cfg(target_os = "android")]
pub fn set_backlight(frac: f32, sensor: bool) -> bool {
    binder_path::set_backlight(frac, sensor)
}
#[cfg(not(target_os = "android"))]
pub fn set_backlight(_frac: f32, _sensor: bool) -> bool {
    false
}

/// Whether the Lights HAL is reachable + exposes a backlight (else fall back to sysfs).
#[cfg(target_os = "android")]
pub fn available() -> bool {
    binder_path::available()
}
#[cfg(not(target_os = "android"))]
pub fn available() -> bool {
    false
}
