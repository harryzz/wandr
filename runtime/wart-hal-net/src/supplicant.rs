//! WiFi association via the vendor `wpa_supplicant` control socket (task 88, M1).
//!
//! Under `--no-art` `WifiService` is gone, so nobody starts/steers the supplicant.
//! The surviving vendor binary still works; we reproduce what `WifiService` does:
//! write a config with the saved network, spawn `wpa_supplicant`, then nudge it to
//! associate over its per-interface AF_UNIX control socket
//! (`SELECT_NETWORK 0` + `REASSOCIATE`). This is the proven ctrl-socket recipe
//! from [[project_artless_network]] (replacing the throwaway `wpanudge`); the
//! ISupplicant AIDL is the later-hardening path.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Where the vendor supplicant lives and where its ctrl sockets are created.
const SUPPLICANT_BIN: &str = "/vendor/bin/hw/wpa_supplicant";
const SOCKET_DIR: &str = "/data/vendor/wifi/wpa/sockets";
const CONF_PATH: &str = "/data/vendor/wifi/wpa/wart-wpa.conf";

/// A saved network's credentials (read from WifiConfigStore.xml by the daemon).
#[derive(Clone, Debug)]
pub struct WifiCreds {
    pub ssid: String,
    pub psk: String,
}

/// Write the supplicant config for one WPA-PSK network. World-readable (644)
/// because `wpa_supplicant` drops to uid `wifi` and must read it — a root-only
/// `600` is the classic "Failed to open config file: Permission denied" trap.
pub fn write_conf(creds: &WifiCreds) -> io::Result<()> {
    // Escape embedded quotes/backslashes in the SSID/PSK for the quoted form.
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let conf = format!(
        "ctrl_interface={dir}\n\
         update_config=1\n\
         network={{\n\
         \tssid=\"{ssid}\"\n\
         \tpsk=\"{psk}\"\n\
         \tkey_mgmt=WPA-PSK\n\
         \tscan_ssid=1\n\
         }}\n",
        dir = SOCKET_DIR,
        ssid = esc(&creds.ssid),
        psk = esc(&creds.psk),
    );
    std::fs::create_dir_all(Path::new(CONF_PATH).parent().unwrap())?;
    std::fs::write(CONF_PATH, conf)?;
    // 644 — readable by the wifi-uid supplicant.
    set_mode(CONF_PATH, 0o644)?;
    Ok(())
}

fn set_mode(path: &str, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Kill any `wpa_supplicant` already running + remove the stale per-interface
/// ctrl socket, so a fresh spawn binds cleanly. Under `--no-art` wart-net is the
/// sole WiFi manager, so killing every supplicant is safe (and necessary: a
/// `std::process::Child` we dropped on a previous failed bring-up does NOT die on
/// its own, and a leftover socket inode gives the next connect an immediate
/// `ECONNREFUSED`). Best-effort.
pub fn cleanup_stale(ifname: &str) {
    let _ = Command::new("pkill").args(["-9", "-x", "wpa_supplicant"]).status();
    let _ = std::fs::remove_file(format!("{SOCKET_DIR}/{ifname}"));
    // Brief settle so the kernel releases the iface before the new supplicant
    // grabs it (a back-to-back grab can race the nl80211 teardown).
    std::thread::sleep(Duration::from_millis(300));
}

/// Spawn the vendor `wpa_supplicant` against our config on `ifname`. The caller
/// owns the returned `Child` (the daemon keeps it alive for the session). Call
/// [`cleanup_stale`] first when there's no carrier so a leaked/old supplicant
/// doesn't conflict.
pub fn spawn(ifname: &str) -> io::Result<Child> {
    std::fs::create_dir_all(SOCKET_DIR)?;
    let child = Command::new(SUPPLICANT_BIN)
        .args([
            "-i",
            ifname,
            "-Dnl80211",
            "-c",
            CONF_PATH,
            "-O",
            SOCKET_DIR,
        ])
        .spawn()?;
    log::info!("supplicant: spawned {SUPPLICANT_BIN} on {ifname} (pid {})", child.id());
    Ok(child)
}

/// A connected control-socket channel to `wpa_supplicant` for one interface.
pub struct CtrlSocket {
    sock: UnixDatagram,
    local_path: String,
}

impl CtrlSocket {
    /// Connect to `<SOCKET_DIR>/<ifname>`, binding a local reply path in the same
    /// (wifi-writable) directory so the supplicant can send replies back. Binds
    /// the local socket once, then retries `connect` + a `PING`/`PONG` liveness
    /// probe until the supplicant is actually serving: the socket file appears a
    /// moment after spawn, and even once it exists a freshly-spawned (or stale)
    /// socket gives `ECONNREFUSED`/`ENOENT` until the supplicant binds it — so we
    /// must retry the connect itself, not just wait for the path.
    pub fn connect(ifname: &str) -> io::Result<Self> {
        let remote = format!("{SOCKET_DIR}/{ifname}");
        let local_path = format!("{SOCKET_DIR}/wart-ctrl-{}", std::process::id());
        let _ = std::fs::remove_file(&local_path);

        let sock = UnixDatagram::bind(&local_path)?;
        // wpa (uid wifi) must be able to write the reply back to us.
        let _ = set_mode(&local_path, 0o666);
        // 2s recv timeout — replies are prompt; a missing one isn't fatal.
        set_rcv_timeout(&sock, Duration::from_secs(2))?;
        let cs = CtrlSocket { sock, local_path };

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            // connect() may be re-called to (re)set the peer; refused/missing while
            // the supplicant is still coming up, so just retry.
            if cs.sock.connect(&remote).is_ok() {
                if let Ok(reply) = cs.cmd("PING") {
                    if reply.contains("PONG") {
                        return Ok(cs);
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!("supplicant ctrl socket {remote} not serving after 10s"),
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Send a control command and return the reply text (trimmed). A timeout
    /// yields an empty string — some commands deliver without a readable reply.
    pub fn cmd(&self, command: &str) -> io::Result<String> {
        self.sock.send(command.as_bytes())?;
        let mut buf = [0u8; 4096];
        match self.sock.recv(&mut buf) {
            Ok(n) => Ok(String::from_utf8_lossy(&buf[..n]).trim().to_string()),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                Ok(String::new())
            }
            Err(e) => Err(e),
        }
    }

    /// Drive association: select the (single) network and (re)associate. Android's
    /// supplicant loads the network but waits for this external trigger.
    pub fn associate(&self) -> io::Result<()> {
        let _ = self.cmd("SELECT_NETWORK 0")?;
        let _ = self.cmd("REASSOCIATE")?;
        Ok(())
    }

    /// Poll `STATUS` until `wpa_state=COMPLETED` (4-way handshake done) or the
    /// deadline elapses. Returns true on association.
    pub fn wait_connected(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(status) = self.cmd("STATUS") {
                if status.contains("wpa_state=COMPLETED") {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        false
    }
}

impl Drop for CtrlSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.local_path);
    }
}

fn set_rcv_timeout(sock: &UnixDatagram, dur: Duration) -> io::Result<()> {
    let tv = libc::timeval {
        tv_sec: dur.as_secs() as libc::time_t,
        tv_usec: dur.subsec_micros() as libc::suseconds_t,
    };
    let rc = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
