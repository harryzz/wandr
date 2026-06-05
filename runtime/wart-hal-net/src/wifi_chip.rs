//! WiFi chip power-up via the `android.hardware.wifi.IWifi` HAL (task 88 M2 —
//! cold-boot support). On a `--no-art` boot where WiFi was never on under ART, the
//! chip is unpowered and there is no `wlan0` — normally `WifiService`/`WifiNative`
//! drive the `IWifi` HAL to power the chip + create the STA interface. This module
//! does that over binder: `start()` → `getChip()` → `configureChip(<STA mode>)` →
//! `createStaIface()`. Must run as uid `system` (vendor HAL). `start()` is
//! confirmed via `isStarted()` and `createStaIface()` returns the iface directly,
//! so no `IWifiChipEventCallback` Bn is needed (scoped choice).

#![cfg(target_os = "android")]
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, clippy::all)]

mod binder_aidl {
    include!(concat!(env!("OUT_DIR"), "/iwifi_bindings.rs"));
}

use binder_aidl::android::hardware::wifi::{
    IWifi::IWifi, IfaceConcurrencyType::IfaceConcurrencyType,
};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const WIFI_SVC: &str = "android.hardware.wifi.IWifi/default";

fn ensure_process_state() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        if rsbinder::ProcessState::init_default().is_ok() {
            rsbinder::ProcessState::start_thread_pool();
        }
    });
}

/// Ensure the WiFi chip is powered and a STA interface exists. Returns the STA
/// interface name on success (e.g. `"wlan0"`). MUST be called as uid `system`.
pub fn ensure_chip_up(iface: &str) -> bool {
    ensure_process_state();
    let wifi = match rsbinder::hub::get_interface::<dyn IWifi>(WIFI_SVC) {
        Ok(w) => w,
        Err(e) => {
            log::error!("wifi: get IWifi failed: {e:?}");
            return false;
        }
    };
    match ensure_chip_up_inner(&wifi, iface) {
        Ok(name) => {
            log::info!("wifi: chip up, STA iface = {name:?}");
            true
        }
        Err(e) => {
            log::error!("wifi: chip power-up failed: {e}");
            false
        }
    }
}

fn ensure_chip_up_inner(
    wifi: &rsbinder::Strong<dyn IWifi>,
    iface: &str,
) -> Result<String, String> {
    // 1. Power on the chip(s) if not already (the cold case). `isStarted()` is the
    //    canonical "chip powered" signal — pollable, no event callback. (Note: on
    //    this device the `wlan0` *netdev* is kernel-driver-created + persistent, so
    //    its presence is NOT a reliable signal; the HAL's started state is.)
    if !wifi.r#isStarted().map_err(|e| format!("isStarted: {e:?}"))? {
        wifi.r#start().map_err(|e| format!("start: {e:?}"))?;
        let deadline = Instant::now() + Duration::from_secs(6);
        while !wifi.r#isStarted().map_err(|e| format!("isStarted: {e:?}"))? {
            if Instant::now() >= deadline {
                return Err("IWifi.start() did not reach isStarted within 6s".into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        log::info!("wifi: IWifi.start() — chip powered");
    }

    let chip_ids = wifi.r#getChipIds().map_err(|e| format!("getChipIds: {e:?}"))?;
    let chip_id = *chip_ids.first().ok_or("no wifi chips reported")?;
    let chip = wifi.r#getChip(chip_id).map_err(|e| format!("getChip({chip_id}): {e:?}"))?;

    // 2. If the chip already has the STA iface, we're done — don't reconfigure or
    //    recreate (the non-disruptive warm path).
    if let Ok(sta) = chip.r#getStaIface(iface) {
        return sta.r#getName().map_err(|e| format!("getName: {e:?}"));
    }

    // 3. Otherwise create it — first ensure a STA-capable mode (derived from the
    //    chip's reported concurrency combinations, not hardcoded — STA = 0).
    let modes = chip
        .r#getAvailableModes()
        .map_err(|e| format!("getAvailableModes: {e:?}"))?;
    let sta_mode = modes
        .iter()
        .find(|m| {
            m.r#availableCombinations.iter().any(|c| {
                c.r#limits
                    .iter()
                    .any(|l| l.r#types.iter().any(|t| *t == IfaceConcurrencyType::STA))
            })
        })
        .map(|m| m.r#id)
        .ok_or("no chip mode supports STA")?;
    if chip.r#getMode().unwrap_or(-1) != sta_mode {
        chip.r#configureChip(sta_mode)
            .map_err(|e| format!("configureChip({sta_mode}): {e:?}"))?;
        log::info!("wifi: chip configured to mode {sta_mode} (STA)");
    }

    let staiface = chip
        .r#createStaIface()
        .map_err(|e| format!("createStaIface: {e:?}"))?;
    staiface.r#getName().map_err(|e| format!("getName: {e:?}"))
}

/// Remove the STA interface (the dynamically-created `wlan0` netdev) via the
/// chip — produces the genuine cold state (no `wlan0`) for testing the cold path,
/// and is the clean teardown counterpart to `createStaIface`. uid `system`.
pub fn remove_sta_iface(iface: &str) -> bool {
    ensure_process_state();
    let wifi = match rsbinder::hub::get_interface::<dyn IWifi>(WIFI_SVC) {
        Ok(w) => w,
        Err(e) => {
            log::error!("wifi: get IWifi failed: {e:?}");
            return false;
        }
    };
    let res = (|| -> Result<(), String> {
        let ids = wifi.r#getChipIds().map_err(|e| format!("getChipIds: {e:?}"))?;
        let id = *ids.first().ok_or("no wifi chips")?;
        let chip = wifi.r#getChip(id).map_err(|e| format!("getChip: {e:?}"))?;
        chip.r#removeStaIface(iface)
            .map_err(|e| format!("removeStaIface({iface}): {e:?}"))
    })();
    match res {
        Ok(()) => {
            log::info!("wifi: removed STA iface {iface}");
            true
        }
        Err(e) => {
            log::error!("wifi: {e}");
            false
        }
    }
}

/// Power the chip DOWN (release the STA iface) — for testing the cold path and
/// for clean teardown. MUST be called as uid `system`.
pub fn stop_chip() -> bool {
    ensure_process_state();
    match rsbinder::hub::get_interface::<dyn IWifi>(WIFI_SVC) {
        Ok(w) => match w.r#stop() {
            Ok(_) => {
                log::info!("wifi: IWifi.stop() — chip powered down");
                true
            }
            Err(e) => {
                log::error!("wifi: IWifi.stop() failed: {e:?}");
                false
            }
        },
        Err(e) => {
            log::error!("wifi: get IWifi failed: {e:?}");
            false
        }
    }
}
