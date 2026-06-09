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

use binder_aidl::android::hardware::wifi::supplicant::{
    ISupplicant::ISupplicant, KeyMgmtMask::KeyMgmtMask,
};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const SVC_NAME: &str = "android.hardware.wifi.supplicant.ISupplicant/default";

fn ensure_process_state() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        if rsbinder::ProcessState::init_default().is_ok() {
            rsbinder::ProcessState::start_thread_pool();
        }
    });
}

/// Get the ISupplicant HAL, retrying briefly because it registers a moment after
/// the supplicant is spawned.
fn get_supplicant() -> Option<rsbinder::Strong<dyn ISupplicant>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(s) = rsbinder::hub::get_interface::<dyn ISupplicant>(SVC_NAME) {
            return Some(s);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Drive association via the ISupplicant AIDL HAL (the Android-native path that
/// `WifiNative` uses): `addStaInterface → addNetwork → setSsid/setKeyMgmt(WPA_PSK)
/// /setPskPassphrase/setScanSsid → select`. The supplicant must already be running
/// HAL-managed ([`crate::supplicant::spawn_hal`]). MUST be called as uid `system`
/// (the HAL rejects root). Does NOT wait for `COMPLETED` — the caller confirms via
/// carrier (the scoped no-Bn-callback choice). Returns true if `select()` was
/// accepted.
pub fn associate(iface: &str, ssid: &str, psk: &str) -> bool {
    ensure_process_state();
    let Some(sup) = get_supplicant() else {
        log::error!("supplicant: ISupplicant unavailable (HAL not up?)");
        return false;
    };
    match associate_inner(&sup, iface, ssid, psk) {
        Ok(()) => {
            log::info!("supplicant: AIDL associate triggered (ssid={ssid:?} iface={iface})");
            true
        }
        Err(e) => {
            log::error!("supplicant: AIDL associate failed: {e:?}");
            false
        }
    }
}

fn associate_inner(
    sup: &rsbinder::Strong<dyn ISupplicant>,
    iface: &str,
    ssid: &str,
    psk: &str,
) -> rsbinder::status::Result<()> {
    // addStaInterface creates the HAL StaIface object (legacy -i mode never does,
    // which is why getStaInterface failed in the probe); if it already exists,
    // fall back to getStaInterface.
    let staiface = match sup.r#addStaInterface(iface) {
        Ok(i) => i,
        Err(_) => sup.r#getStaInterface(iface)?,
    };
    let net = staiface.r#addNetwork()?;
    net.r#setSsid(ssid.as_bytes())?;
    net.r#setKeyMgmt(KeyMgmtMask::WPA_PSK)?;
    net.r#setPskPassphrase(psk)?;
    net.r#setScanSsid(true)?;
    net.r#select()?;
    Ok(())
}

/// Try to obtain the ISupplicant HAL + make read-only calls. Logs each step so a
/// servicemanager miss vs. an SELinux/permission denial vs. success is distinct.
/// Returns true if the proxy was obtained and `listInterfaces` succeeded.
pub fn probe() -> bool {
    ensure_process_state();
    const NAME: &str = SVC_NAME;

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
