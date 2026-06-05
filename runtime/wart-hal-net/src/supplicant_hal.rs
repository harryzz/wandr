//! ISupplicant HAL probe (task 88 M2 reconnaissance) — can a `--no-art` process
//! reach the vendor WiFi supplicant HAL (`android.hardware.wifi.supplicant
//! .ISupplicant`) over binder, as uid `system`? This is the Android-native
//! association path (`WifiNative` → `ISupplicant`) that would replace the
//! ctrl-socket nudge. The AIDL parses/compiles; the open question is the runtime:
//! servicemanager visibility + SELinux (the supplicant HAL has a vendor label).
//! This is a read-only probe (`listInterfaces` / `getStaInterface`), not a driver.

#![cfg(target_os = "android")]
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, clippy::all)]

mod binder_aidl {
    include!(concat!(env!("OUT_DIR"), "/isupplicant_bindings.rs"));
}

use binder_aidl::android::hardware::wifi::supplicant::ISupplicant::ISupplicant;
use std::sync::OnceLock;

fn ensure_process_state() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        if rsbinder::ProcessState::init_default().is_ok() {
            rsbinder::ProcessState::start_thread_pool();
        }
    });
}

/// Try to obtain the ISupplicant HAL + make read-only calls. Logs each step so a
/// servicemanager miss vs. an SELinux/permission denial vs. success is distinct.
/// Returns true if the proxy was obtained and `listInterfaces` succeeded.
pub fn probe() -> bool {
    ensure_process_state();
    const NAME: &str = "android.hardware.wifi.supplicant.ISupplicant/default";

    let svc = match rsbinder::hub::get_interface::<dyn ISupplicant>(NAME) {
        Ok(s) => {
            log::info!("supplicant-probe: got ISupplicant proxy ({NAME})");
            s
        }
        Err(e) => {
            log::error!("supplicant-probe: get_interface({NAME}) FAILED: {e:?}");
            return false;
        }
    };

    let mut ok = false;
    match svc.r#listInterfaces() {
        Ok(ifaces) => {
            ok = true;
            log::info!("supplicant-probe: listInterfaces() OK — {} iface(s):", ifaces.len());
            for i in &ifaces {
                log::info!("supplicant-probe:   iface name={:?} type={:?}", i.r#name, i.r#type);
            }
        }
        Err(e) => log::error!("supplicant-probe: listInterfaces() FAILED: {e:?}"),
    }

    // The real M2 entry point — getting the STA iface sub-interface.
    match svc.r#getStaInterface("wlan0") {
        Ok(_) => log::info!("supplicant-probe: getStaInterface(wlan0) OK — STA iface reachable"),
        Err(e) => log::warn!("supplicant-probe: getStaInterface(wlan0): {e:?}"),
    }
    ok
}
