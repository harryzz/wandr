//! wandr-net — the ART-off connectivity daemon (task 88, M1).
//!
//! Brings WiFi-STA up with the Java framework stopped, productizing the manual
//! recipe from [[project_artless_network]]: read the saved network's creds from
//! `WifiConfigStore.xml`, spawn the vendor `wpa_supplicant` + nudge its ctrl
//! socket to associate, lease an address with our pure-Rust DHCPv4 client, apply
//! the address/route via `ip`, configure DNS, and report link status to the
//! arbiter (`report-net-state`). Mirrors `wandr-sensors`: one process owns the
//! link, the bring-up script keeps it alive with a respawn supervisor.
//!
//! Modes:
//!   * (default) daemon — bring up, then monitor carrier and re-associate on loss.
//!   * `--once` — one bring-up, print status, exit (the on-device investigation
//!     harness; also handy for `--no-art` smoke tests).

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use wandr_hal_net::{
    apply_address, dhcp, has_carrier, install_default_rule, read_saved_creds, supplicant,
    DhcpLease, LinkStatus, WifiCreds, WANDR_NETID, WLAN_IF,
};

fn arbiter_sock_path() -> String {
    std::env::var("WANDR_ARBITER_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/data/local/tmp/wandr-arbiter.sock".to_string())
}

/// Push one command line to the arbiter (one-shot connect+write+close, like
/// wandr-sensors). Best-effort — a missing arbiter must not kill the daemon.
fn send_arbiter(line: &str) {
    let sock = arbiter_sock_path();
    match UnixStream::connect(&sock) {
        Ok(mut s) => {
            let _ = writeln!(s, "{line}");
            let _ = s.flush();
            let _ = s.shutdown(std::net::Shutdown::Write);
            log::debug!("wandr-net: {line} → {sock}");
        }
        Err(e) => log::debug!("wandr-net: {line:?} → {sock} failed: {e}"),
    }
}

/// base64 (standard alphabet, padded) — matches the host's decoder. The event-bus
/// payload is base64 over the line-framed arbiter socket (task 90).
fn b64_encode(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(A[(n >> 18 & 63) as usize] as char);
        out.push(A[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { A[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { A[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Report a link snapshot to the arbiter. Offline: `report-net-state offline`.
/// Online: `report-net-state online wifi <ssid> <ip>`. Also publishes the same
/// snapshot to the generic event bus under topic `net.status` (task 90) so any
/// guest subscribed via `package.toml [events]` receives it through
/// `wandr:events/incoming-handler` — the payload is the identical wire string
/// (`online wifi <ssid> <ip>` / `offline`), base64'd.
fn report(status: &LinkStatus) {
    let wire = if status.up {
        let ssid = status.ssid.as_deref().unwrap_or("-");
        let ip = status.ip.map(|i| i.to_string()).unwrap_or_else(|| "-".into());
        format!("online wifi {ssid} {ip}")
    } else {
        "offline".to_string()
    };
    send_arbiter(&format!("report-net-state {wire}"));
    send_arbiter(&format!("evt-publish net.status {}", b64_encode(wire.as_bytes())));
}

/// The supplicant child + the leased status from one successful bring-up.
struct Link {
    /// Kept alive for the session (dropping it kills the supplicant). `None` if a
    /// supplicant was already running and we reused it.
    _supplicant: Option<Child>,
    status: LinkStatus,
}

/// Run the full bring-up: associate (if needed) → DHCP → apply → DNS → status.
fn bring_up() -> Result<Link, String> {
    // 1. Saved creds (first WPA-PSK network).
    let creds = read_saved_creds().map_err(|e| format!("read creds: {e}"))?;
    let creds = creds
        .into_iter()
        .next()
        .ok_or_else(|| "no saved WPA-PSK network in WifiConfigStore.xml".to_string())?;
    log::info!("wandr-net: bringing up SSID {:?}", creds.ssid);

    // 2. Associate — unless the link already has carrier (a supplicant is up).
    let mut child = None;
    if has_carrier(WLAN_IF) {
        log::info!("wandr-net: {WLAN_IF} already has carrier — reusing existing association");
    } else {
        // Cold boot: ensure the WiFi chip is powered + a STA iface exists
        // (IWifi.start + createStaIface, as uid system — what WifiService does).
        // Idempotent + non-disruptive when the chip is already up; must precede the
        // supplicant so the iface is ready for addStaInterface.
        power_chip_via_hal()?;
        // Clear any leaked/old supplicant + stale ctrl socket so the fresh spawn
        // binds cleanly (a dropped Child doesn't die; a stale socket → ECONNREFUSED).
        supplicant::cleanup_stale(WLAN_IF);
        // Start the supplicant HAL-managed (no -i/-c) so the ISupplicant AIDL HAL
        // can addStaInterface + drive it; then associate over the HAL — the
        // Android-native path that replaces the legacy ctrl-socket nudge.
        child = Some(supplicant::spawn_hal().map_err(|e| format!("spawn supplicant: {e}"))?);
        associate_via_hal(&creds)?;
        // Confirm via carrier (the scoped no-Bn-callback choice; the HAL accepted
        // select() and the kernel reports L2 up on a completed 4-way handshake).
        if !wait_carrier(WLAN_IF, Duration::from_secs(25)) {
            return Err("association timed out (no carrier after select)".into());
        }
        log::info!("wandr-net: associated to {:?} (via ISupplicant AIDL)", creds.ssid);
    }

    // 3. DHCP lease.
    let lease = dhcp::acquire(WLAN_IF, 4).map_err(|e| format!("dhcp: {e}"))?;

    // 4. Configure the leased address on the link (root).
    apply_address(WLAN_IF, &lease).map_err(|e| format!("apply address: {e}"))?;

    // 5. Drive netd + dnsresolver over binder — but they accept uid `system`, not
    //    root (this daemon runs as root for the link bring-up), so re-exec the
    //    `--netd-config` path as uid 1000. It creates the netd network + routes
    //    and configures the DNS resolver.
    configure_netd(&lease)?;

    // 6. Catch-all rule so unmarked traffic uses netd's per-network table (root;
    //    bridges the per-UID fwmark gap left by the dead ConnectivityService).
    install_default_rule(WLAN_IF).map_err(|e| format!("install rule: {e}"))?;

    let status = LinkStatus {
        up: true,
        ssid: Some(creds.ssid),
        ip: Some(lease.ip),
        gateway: lease.gateway,
        dns: lease.dns,
    };
    Ok(Link {
        _supplicant: child,
        status,
    })
}

/// Single-quote a value for the `su 1000 -c` shell (handles embedded quotes).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Drive the ISupplicant AIDL association as uid `system` (the HAL rejects root):
/// re-exec `--associate` under `su 1000`. The HAL-managed supplicant is already
/// spawned by the root daemon. PSK is passed as an arg — it's already plaintext +
/// root-readable in WifiConfigStore, so this adds no exposure beyond root.
fn associate_via_hal(creds: &WifiCreds) -> Result<(), String> {
    let self_exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let inner = format!(
        "{self} --associate {ssid} {psk}",
        self = self_exe.to_string_lossy(),
        ssid = shell_quote(&creds.ssid),
        psk = shell_quote(&creds.psk),
    );
    let out = Command::new("su")
        .args(["1000", "-c", &inner])
        .output()
        .map_err(|e| format!("su 1000 spawn: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    log::info!("wandr-net: associate (uid system) -> {}", stdout.trim());
    if !out.status.success() {
        return Err(format!(
            "associate failed (exit {:?}): {} {}",
            out.status.code(),
            stdout.trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Drive the IWifi HAL chip power-up as uid `system` (the HAL rejects root):
/// re-exec `--power-chip` under `su 1000`.
fn power_chip_via_hal() -> Result<(), String> {
    let self_exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let inner = format!("{} --power-chip", self_exe.to_string_lossy());
    let out = Command::new("su")
        .args(["1000", "-c", &inner])
        .output()
        .map_err(|e| format!("su 1000 spawn: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    log::info!("wandr-net: power-chip (uid system) -> {}", stdout.trim());
    if !out.status.success() {
        return Err(format!(
            "power-chip failed: {} {}",
            stdout.trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Poll the interface's L2 carrier until up or the deadline.
fn wait_carrier(iface: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if has_carrier(iface) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    false
}

/// Drive the netd binder configuration as uid `system`: re-exec this binary's
/// `--netd-config` path under `su 1000` (netd + dnsresolver accept AID_SYSTEM, not
/// root — and this daemon runs as root for the link bring-up). The child does the
/// INetd network setup + IDnsResolver resolver config over binder.
fn configure_netd(lease: &DhcpLease) -> Result<(), String> {
    let gw = lease
        .gateway
        .ok_or_else(|| "lease has no gateway — cannot set default network".to_string())?;
    let self_exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dns = lease
        .dns
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let inner = format!(
        "{self} --netd-config {netid} {iface} {ip}/{prefix} {gw} {dns}",
        self = self_exe.to_string_lossy(),
        netid = WANDR_NETID,
        iface = WLAN_IF,
        ip = lease.ip,
        prefix = lease.prefix,
    );
    let out = Command::new("su")
        .args(["1000", "-c", &inner])
        .output()
        .map_err(|e| format!("su 1000 spawn: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    log::info!("wandr-net: netd-config (uid system) -> {}", stdout.trim());
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "netd-config failed (exit {:?}): {} {}",
            out.status.code(),
            stdout.trim(),
            stderr.trim()
        ));
    }
    Ok(())
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().collect();

    // `--netd-config <netid> <iface> <gw> <dns>...` — drive netd + dnsresolver
    // over binder to make `netid` the default network (INetd: createPhysical /
    // addInterface / addRoute / setDefault) and configure its DNS resolver
    // (IDnsResolver: setResolverConfiguration). Split out so it can run as uid
    // `system` (which netd/dnsresolver require — they reject root) while the
    // daemon proper runs as root for the link bring-up; invoked as
    // `su 1000 wandr-net --netd-config …` / via wandr-launch.
    if let Some(pos) = args.iter().position(|a| a == "--netd-config") {
        let rest = &args[pos + 1..];
        let netid: i32 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(-1);
        let iface = rest.get(1).cloned().unwrap_or_else(|| WLAN_IF.to_string());
        // <ip/prefix> of the local address (e.g. 192.168.1.179/22) — the connected
        // route is derived from it.
        let cidr = rest.get(2).cloned().unwrap_or_default();
        let (ip_str, prefix) = cidr.split_once('/').unwrap_or((cidr.as_str(), "24"));
        let local_ip: Option<std::net::Ipv4Addr> = ip_str.parse().ok();
        let prefix: u8 = prefix.parse().unwrap_or(24);
        let gw: Option<std::net::Ipv4Addr> = rest.get(3).and_then(|s| s.parse().ok());
        let servers: Vec<std::net::Ipv4Addr> =
            rest.iter().skip(4).filter_map(|s| s.parse().ok()).collect();
        let (Some(local_ip), Some(gw), false) = (local_ip, gw, netid < 0) else {
            eprintln!("usage: wandr-net --netd-config <netid> <iface> <ip/prefix> <gw-ipv4> <dns-ipv4>...");
            std::process::exit(2);
        };
        let subnet = wandr_hal_net::subnet_cidr(local_ip, prefix);
        let net_ok = wandr_hal_net::setup_network(netid, &iface, &subnet, gw);
        let dns_ok = !servers.is_empty()
            && wandr_hal_net::configure_resolver(netid, &iface, &servers, &[]);
        println!("netd-config netid={netid} iface={iface} subnet={subnet} gw={gw} net={net_ok} dns={dns_ok}");
        std::process::exit(if net_ok && dns_ok { 0 } else { 1 });
    }

    // `--probe-supplicant` — M2 reconnaissance: try to reach the ISupplicant HAL
    // over binder (run as `su 1000` to test the uid-system case).
    if args.iter().any(|a| a == "--probe-supplicant") {
        let ok = wandr_hal_net::probe_supplicant();
        println!("probe-supplicant -> {ok}");
        std::process::exit(if ok { 0 } else { 1 });
    }

    // `--power-chip` — cold-boot: drive the IWifi HAL to power the chip + create
    // the STA iface (run as `su 1000`). `--stop-chip` powers it down (test/teardown).
    if args.iter().any(|a| a == "--power-chip") {
        let ok = wandr_hal_net::ensure_chip_up(WLAN_IF).is_some();
        println!("power-chip -> {ok}");
        std::process::exit(if ok { 0 } else { 1 });
    }
    if args.iter().any(|a| a == "--stop-chip") {
        let ok = wandr_hal_net::stop_chip();
        println!("stop-chip -> {ok}");
        std::process::exit(if ok { 0 } else { 1 });
    }
    if args.iter().any(|a| a == "--remove-iface") {
        let ok = wandr_hal_net::remove_sta_iface(WLAN_IF);
        println!("remove-iface -> {ok}");
        std::process::exit(if ok { 0 } else { 1 });
    }

    // `--associate <ssid> <psk>` — drive association via the ISupplicant AIDL HAL.
    // Split out to run as uid `system` (the HAL rejects root) while the daemon
    // proper runs as root to spawn the supplicant; invoked as `su 1000 …`.
    if let Some(pos) = args.iter().position(|a| a == "--associate") {
        let rest = &args[pos + 1..];
        let ssid = rest.first().cloned().unwrap_or_default();
        let psk = rest.get(1).cloned().unwrap_or_default();
        if ssid.is_empty() || psk.is_empty() {
            eprintln!("usage: wandr-net --associate <ssid> <psk>");
            std::process::exit(2);
        }
        let ok = wandr_hal_net::associate_supplicant(WLAN_IF, &ssid, &psk);
        println!("associate ssid={ssid:?} -> {ok}");
        std::process::exit(if ok { 0 } else { 1 });
    }

    let once = args.iter().any(|a| a == "--once");

    match bring_up() {
        Ok(link) => {
            report(&link.status);
            println!(
                "wandr-net: ONLINE ssid={:?} ip={:?} gw={:?} dns={:?}",
                link.status.ssid, link.status.ip, link.status.gateway, link.status.dns
            );
            if once {
                // Investigation / smoke mode: report once and exit (the supplicant
                // child is dropped → killed, but the kernel keeps the lease/route).
                return;
            }
            monitor(link);
        }
        Err(e) => {
            log::error!("wandr-net: bring-up failed: {e}");
            report(&LinkStatus::default());
            // Exit non-zero so the respawn supervisor retries after a short sleep.
            std::process::exit(1);
        }
    }
}

/// Steady state: hold the supplicant alive and watch carrier; on a drop, re-run
/// the full bring-up. Periodically re-report so a freshly-(re)started arbiter
/// learns the current state.
fn monitor(mut link: Link) {
    let mut ticks: u32 = 0;
    loop {
        std::thread::sleep(Duration::from_secs(5));
        ticks += 1;

        if !has_carrier(WLAN_IF) {
            log::warn!("wandr-net: {WLAN_IF} lost carrier — re-associating");
            report(&LinkStatus::default());
            match bring_up() {
                Ok(new) => {
                    link = new;
                    report(&link.status);
                }
                Err(e) => {
                    log::error!("wandr-net: re-bring-up failed: {e} — retrying next tick");
                }
            }
            continue;
        }

        // Re-announce roughly every 30s so arbiter restarts pick us up.
        if ticks % 6 == 0 {
            report(&link.status);
        }
    }
}
