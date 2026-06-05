//! wart-net — the ART-off connectivity daemon (task 88, M1).
//!
//! Brings WiFi-STA up with the Java framework stopped, productizing the manual
//! recipe from [[project_artless_network]]: read the saved network's creds from
//! `WifiConfigStore.xml`, spawn the vendor `wpa_supplicant` + nudge its ctrl
//! socket to associate, lease an address with our pure-Rust DHCPv4 client, apply
//! the address/route via `ip`, configure DNS, and report link status to the
//! arbiter (`report-net-state`). Mirrors `wart-sensors`: one process owns the
//! link, the bring-up script keeps it alive with a respawn supervisor.
//!
//! Modes:
//!   * (default) daemon — bring up, then monitor carrier and re-associate on loss.
//!   * `--once` — one bring-up, print status, exit (the on-device investigation
//!     harness; also handy for `--no-art` smoke tests).

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::time::Duration;

use wart_hal_net::{
    apply_address, dhcp, has_carrier, install_default_rule, read_saved_creds, supplicant,
    DhcpLease, LinkStatus, WART_NETID, WLAN_IF,
};

fn arbiter_sock_path() -> String {
    std::env::var("WART_ARBITER_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/data/local/tmp/wart-arbiter.sock".to_string())
}

/// Push one command line to the arbiter (one-shot connect+write+close, like
/// wart-sensors). Best-effort — a missing arbiter must not kill the daemon.
fn send_arbiter(line: &str) {
    let sock = arbiter_sock_path();
    match UnixStream::connect(&sock) {
        Ok(mut s) => {
            let _ = writeln!(s, "{line}");
            let _ = s.flush();
            let _ = s.shutdown(std::net::Shutdown::Write);
            log::debug!("wart-net: {line} → {sock}");
        }
        Err(e) => log::debug!("wart-net: {line:?} → {sock} failed: {e}"),
    }
}

/// Report a link snapshot to the arbiter. Offline:
/// `report-net-state offline`. Online: `report-net-state online wifi <ssid> <ip>`.
fn report(status: &LinkStatus) {
    if status.up {
        let ssid = status.ssid.as_deref().unwrap_or("-");
        let ip = status
            .ip
            .map(|i| i.to_string())
            .unwrap_or_else(|| "-".into());
        send_arbiter(&format!("report-net-state online wifi {ssid} {ip}"));
    } else {
        send_arbiter("report-net-state offline");
    }
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
    log::info!("wart-net: bringing up SSID {:?}", creds.ssid);

    // 2. Associate — unless the link already has carrier (a supplicant is up).
    let mut child = None;
    if has_carrier(WLAN_IF) {
        log::info!("wart-net: {WLAN_IF} already has carrier — reusing existing association");
    } else {
        // Clear any leaked/old supplicant + stale ctrl socket so the fresh spawn
        // binds cleanly (a dropped Child doesn't die; a stale socket → ECONNREFUSED).
        supplicant::cleanup_stale(WLAN_IF);
        supplicant::write_conf(&creds).map_err(|e| format!("write conf: {e}"))?;
        child = Some(supplicant::spawn(WLAN_IF).map_err(|e| format!("spawn supplicant: {e}"))?);
        let ctrl = supplicant::CtrlSocket::connect(WLAN_IF)
            .map_err(|e| format!("ctrl connect: {e}"))?;
        ctrl.associate().map_err(|e| format!("associate: {e}"))?;
        if !ctrl.wait_connected(Duration::from_secs(20)) {
            return Err("association timed out (no wpa_state=COMPLETED)".into());
        }
        log::info!("wart-net: associated to {:?}", creds.ssid);
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
        netid = WART_NETID,
        iface = WLAN_IF,
        ip = lease.ip,
        prefix = lease.prefix,
    );
    let out = Command::new("su")
        .args(["1000", "-c", &inner])
        .output()
        .map_err(|e| format!("su 1000 spawn: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    log::info!("wart-net: netd-config (uid system) -> {}", stdout.trim());
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
    // `su 1000 wart-net --netd-config …` / via wart-launch.
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
            eprintln!("usage: wart-net --netd-config <netid> <iface> <ip/prefix> <gw-ipv4> <dns-ipv4>...");
            std::process::exit(2);
        };
        let subnet = wart_hal_net::subnet_cidr(local_ip, prefix);
        let net_ok = wart_hal_net::setup_network(netid, &iface, &subnet, gw);
        let dns_ok = !servers.is_empty()
            && wart_hal_net::configure_resolver(netid, &iface, &servers, &[]);
        println!("netd-config netid={netid} iface={iface} subnet={subnet} gw={gw} net={net_ok} dns={dns_ok}");
        std::process::exit(if net_ok && dns_ok { 0 } else { 1 });
    }

    let once = args.iter().any(|a| a == "--once");

    match bring_up() {
        Ok(link) => {
            report(&link.status);
            println!(
                "wart-net: ONLINE ssid={:?} ip={:?} gw={:?} dns={:?}",
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
            log::error!("wart-net: bring-up failed: {e}");
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
            log::warn!("wart-net: {WLAN_IF} lost carrier — re-associating");
            report(&LinkStatus::default());
            match bring_up() {
                Ok(new) => {
                    link = new;
                    report(&link.status);
                }
                Err(e) => {
                    log::error!("wart-net: re-bring-up failed: {e} — retrying next tick");
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
