//! Start the vendor `wpa_supplicant` for the ISupplicant AIDL path (task 88, M2).
//!
//! Under `--no-art` `WifiService` is gone, so nobody starts/steers the supplicant.
//! This module starts the surviving vendor binary **HAL-managed** (the vendor
//! init form: a global control interface, no per-iface config) so the
//! `ISupplicant` AIDL HAL can create + drive the interface. The actual
//! association (`addStaInterface → addNetwork → set creds → select`) is done over
//! binder in [`crate::supplicant_hal`]. (M1 drove the legacy per-iface ctrl socket
//! directly with `SELECT_NETWORK`/`REASSOCIATE`; M2 replaced that with this
//! Android-native AIDL path, the same one `WifiNative` uses.)

use std::io;
use std::process::{Child, Command};
use std::time::Duration;

/// Where the vendor supplicant lives + where its ctrl sockets are created.
const SUPPLICANT_BIN: &str = "/vendor/bin/hw/wpa_supplicant";
const SOCKET_DIR: &str = "/data/vendor/wifi/wpa/sockets";

/// A saved network's credentials (read from WifiConfigStore.xml by the daemon).
#[derive(Clone, Debug)]
pub struct WifiCreds {
    pub ssid: String,
    pub psk: String,
}

/// Kill any `wpa_supplicant` already running + remove the stale per-interface
/// ctrl socket so a fresh spawn is clean. Under `--no-art` wandr-net is the sole
/// WiFi manager, so killing every supplicant is safe (and necessary: a
/// `std::process::Child` we dropped on a previous failed bring-up does NOT die on
/// its own). Best-effort.
pub fn cleanup_stale(ifname: &str) {
    let _ = Command::new("pkill").args(["-9", "-x", "wpa_supplicant"]).status();
    let _ = std::fs::remove_file(format!("{SOCKET_DIR}/{ifname}"));
    // Brief settle so the kernel releases the iface before the new supplicant
    // grabs it (a back-to-back grab can race the nl80211 teardown).
    std::thread::sleep(Duration::from_millis(300));
}

/// Spawn the vendor `wpa_supplicant` in **HAL-managed** mode — the form the
/// vendor init service uses (`-O<dir> -dd -g@android:wpa_wlan0`, no `-i`/`-c`).
/// In this mode the supplicant registers the `ISupplicant` AIDL service and lets
/// the HAL create + drive interfaces via `addStaInterface()`; the `-g@android:…`
/// global control socket is how the AIDL service reaches the wpa core. No config
/// file — credentials are pushed over AIDL (`setPskPassphrase`). Pair with
/// [`crate::supplicant_hal::associate`]. Call [`cleanup_stale`] first. The caller
/// owns the returned `Child` (the daemon keeps it alive for the session).
pub fn spawn_hal() -> io::Result<Child> {
    std::fs::create_dir_all(SOCKET_DIR)?;
    let child = Command::new(SUPPLICANT_BIN)
        .args(["-O", SOCKET_DIR, "-dd", "-g@android:wpa_wlan0"])
        .spawn()?;
    log::info!(
        "supplicant: spawned {SUPPLICANT_BIN} HAL-managed (pid {})",
        child.id()
    );
    Ok(child)
}
