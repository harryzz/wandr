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

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wandr_hal_net::{
    dhcp, has_carrier, read_saved_creds, supplicant, DhcpLease, LinkStatus, WifiCreds,
    WANDR_NETID, WLAN_IF,
};

/// The wandr-net control socket — the host→arbiter→daemon wifi-management relay
/// (task 90 M2) lands here. The single live daemon owns the supplicant, so scan /
/// connect / radio go *through* this socket rather than spawning a competing
/// `wandr-net` process. Path overridable via `WANDR_NET_SOCK` (matches the
/// arbiter's `relay_to_wandr_net`).
fn control_sock_path() -> String {
    std::env::var("WANDR_NET_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/data/local/tmp/wandr-net.sock".to_string())
}

fn arbiter_sock_path() -> String {
    std::env::var("WANDR_ARBITER_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/data/local/tmp/wandr-arbiter.sock".to_string())
}

/// Query the arbiter: one-shot connect+write, then read the full reply. `None`
/// if the arbiter is unreachable. Used at bring-up to fetch the auto-connect
/// network from the WifiConfigManager store (task 90 M3).
fn query_arbiter(line: &str) -> Option<String> {
    let sock = arbiter_sock_path();
    let mut s = UnixStream::connect(&sock).ok()?;
    writeln!(s, "{line}").ok()?;
    s.flush().ok()?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut buf = String::new();
    s.read_to_string(&mut buf).ok()?;
    Some(buf)
}

/// Ask the arbiter's WifiConfigManager for the auto-connect network's creds
/// (`wifi-auto-network` → `OK ssid=<b64> psk=<b64> sec=<s>` / `OK none`). The
/// wandr-owned saved-network store (task 90 M3) supersedes the single
/// WifiConfigStore.xml read; falls through to the file when the arbiter has no
/// auto-connect network (or is unreachable).
fn saved_creds_from_arbiter() -> Option<WifiCreds> {
    let reply = query_arbiter("wifi-auto-network")?;
    let body = reply.trim().strip_prefix("OK ")?;
    if body.trim() == "none" {
        return None;
    }
    let field = |k: &str| body.split_whitespace().find_map(|kv| kv.strip_prefix(k));
    let ssid = String::from_utf8_lossy(&b64_decode(field("ssid=")?)).into_owned();
    let psk = String::from_utf8_lossy(&b64_decode(field("psk=")?)).into_owned();
    if ssid.is_empty() {
        return None;
    }
    log::info!("wandr-net: auto-connect network {ssid:?} from arbiter store");
    Some(WifiCreds { ssid, psk })
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

/// base64 decode (standard alphabet) — the inverse of [`b64_encode`]. Skips `=`
/// padding + any whitespace. The control-socket protocol base64s the SSID/PSK so
/// they tokenise cleanly across the host→arbiter→daemon hops regardless of spaces.
fn b64_decode(s: &str) -> Vec<u8> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        let Some(v) = val(c) else { continue };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
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

/// Run the full bring-up for the first saved network (boot path): associate if
/// needed → DHCP → apply → DNS → status. Reuses an existing association.
fn bring_up() -> Result<Link, String> {
    // Prefer the wandr-owned saved-network store (task 90 M3, via the arbiter
    // WifiConfigManager); fall back to the framework's WifiConfigStore.xml when the
    // arbiter has no auto-connect network or is unreachable (boot ordering).
    let creds = match saved_creds_from_arbiter() {
        Some(c) => c,
        None => read_saved_creds()
            .map_err(|e| format!("read creds: {e}"))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                "no auto-connect network (arbiter store empty + WifiConfigStore.xml has none)"
                    .to_string()
            })?,
    };
    bring_up_with(creds, false)
}

/// Bring the link up on `creds`. `force_assoc` re-associates even when carrier is
/// present (an explicit `--connect` to a possibly-different SSID); `false` reuses an
/// existing association (the boot path). DHCP → apply → netd/DNS → catch-all rule.
fn bring_up_with(creds: WifiCreds, force_assoc: bool) -> Result<Link, String> {
    log::info!("wandr-net: bringing up SSID {:?} (force={force_assoc})", creds.ssid);

    // Associate — unless the link already has carrier and we're not forcing.
    let mut child = None;
    if has_carrier(WLAN_IF) && !force_assoc {
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

    // 3. DHCP lease (root — raw socket). The link is already up from association.
    let lease = dhcp::acquire(WLAN_IF, 4).map_err(|e| format!("dhcp: {e}"))?;

    // 4. M1 binder rewrite — address + network + routes + DNS all over binder as
    //    uid `system` (netd/dnsresolver reject root), via the `--netd-config`
    //    re-exec: the leased address now goes through INetd.interfaceAddAddress
    //    (replacing the root `ip addr` shell-out), alongside the network create /
    //    routes / setDefault / resolver config already on binder.
    //    + `networkAddUidRanges(0..MAX)` so unmarked traffic routes via netd's own
    //    per-UID rules (the CS mechanism) — no rtnetlink catch-all `ip rule` needed.
    configure_netd(&lease)?;

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

/// Re-exec this binary with `flag` under `su 1000` (the HAL ops — chip power,
/// associate — reject root, and the daemon proper runs as root). Returns the
/// trimmed stdout on success. The single uid-system re-exec primitive.
fn run_self_su1000(flag: &str) -> Result<String, String> {
    let self_exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let inner = format!("{} {flag}", self_exe.to_string_lossy());
    let out = Command::new("su")
        .args(["1000", "-c", &inner])
        .output()
        .map_err(|e| format!("su 1000 spawn: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    log::info!("wandr-net: {flag} (uid system) -> {stdout}");
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(format!(
            "{flag} failed: {stdout} {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Drive the IWifi HAL chip power-up as uid `system` (the HAL rejects root):
/// re-exec `--power-chip` under `su 1000`.
fn power_chip_via_hal() -> Result<(), String> {
    run_self_su1000("--power-chip").map(|_| ())
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

/// True if the WiFi radio (STA interface) is up. `set-enabled false` (stop-chip)
/// tears the iface down, so its presence is the honest radio-on signal.
fn radio_enabled() -> bool {
    std::path::Path::new(&format!("/sys/class/net/{WLAN_IF}")).exists()
}

/// Serve the control socket: one line per connection (`scan` / `connect <b64ssid>
/// <b64psk>` / `set-enabled <0|1>` / `is-enabled`), reply written back to EOF. The
/// arbiter relays the host's `wifi-*` verbs here (task 90 M2). Runs in its own
/// thread; the shared [`Link`] is the live association the daemon owns.
fn serve_control(link: Arc<Mutex<Link>>) {
    let path = control_sock_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("wandr-net: control socket bind {path} failed: {e} — wifi relay unavailable");
            return;
        }
    };
    // World-rw so the arbiter can always reach it (both are root today; future-proof).
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));
    log::info!("wandr-net: control socket listening at {path}");
    for conn in listener.incoming() {
        match conn {
            Ok(mut s) => {
                if let Err(e) = handle_control_conn(&mut s, &link) {
                    log::debug!("wandr-net: control conn error: {e}");
                }
            }
            Err(e) => log::debug!("wandr-net: control accept failed: {e}"),
        }
    }
}

fn handle_control_conn(s: &mut UnixStream, link: &Arc<Mutex<Link>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let reply = dispatch_control(line.trim(), link);
    s.write_all(reply.as_bytes())?;
    s.flush()?;
    Ok(())
}

/// Route one control line to its handler, returning the reply text (newline-
/// terminated lines). Unknown verbs get an `err` line so the relay surfaces it.
fn dispatch_control(line: &str, link: &Arc<Mutex<Link>>) -> String {
    let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
    match verb {
        "scan" => control_scan(),
        "connect" => control_connect(rest.trim(), link),
        "set-enabled" => control_set_enabled(rest.trim()),
        "is-enabled" => format!("ok enabled={}\n", radio_enabled() as u8),
        "" => String::new(),
        other => format!("err unknown-verb {other}\n"),
    }
}

/// `scan` → IWificond scan; emit one `ap …` line per AP (SSID base64'd) + a final
/// `ok count=<n>`. Read-only w.r.t. the live association.
fn control_scan() -> String {
    match wandr_hal_net::scan_networks() {
        Some(aps) => {
            let mut out = String::new();
            for ap in &aps {
                out.push_str(&format!(
                    "ap ssid={} bssid={} rssi={} freq={} sec={} connected={}\n",
                    b64_encode(ap.ssid.as_bytes()),
                    ap.bssid,
                    ap.rssi_dbm,
                    ap.frequency_mhz,
                    ap.security,
                    ap.connected as u8,
                ));
            }
            out.push_str(&format!("ok count={}\n", aps.len()));
            out
        }
        None => "err scan-failed\n".to_string(),
    }
}

/// `connect <b64ssid> <b64psk>` → re-associate the live link to an explicit
/// network (`bring_up_with(force=true)`), swap it in as the daemon's owned link,
/// and report the new state up. Holds the link lock for the bring-up so the
/// monitor thread doesn't race a re-associate (low-frequency, user-initiated).
fn control_connect(rest: &str, link: &Arc<Mutex<Link>>) -> String {
    let mut it = rest.split_whitespace();
    let (Some(bs), Some(bp)) = (it.next(), it.next()) else {
        return "err connect-usage <b64ssid> <b64psk>\n".to_string();
    };
    let ssid = String::from_utf8_lossy(&b64_decode(bs)).into_owned();
    let psk = String::from_utf8_lossy(&b64_decode(bp)).into_owned();
    if ssid.is_empty() {
        return "err connect empty-ssid\n".to_string();
    }
    let mut guard = link.lock().unwrap_or_else(|e| e.into_inner());
    match bring_up_with(WifiCreds { ssid: ssid.clone(), psk }, true) {
        Ok(new) => {
            let status = new.status.clone();
            *guard = new;
            drop(guard);
            report(&status);
            let ip = status.ip.map(|i| i.to_string()).unwrap_or_else(|| "-".into());
            format!("ok online ssid={} ip={ip}\n", b64_encode(ssid.as_bytes()))
        }
        Err(e) => format!("err connect {e}\n"),
    }
}

/// `set-enabled <0|1>` → power the WiFi chip up (`--power-chip`) / down
/// (`--stop-chip`) via the uid-system re-exec (the IWifi HAL rejects root).
fn control_set_enabled(rest: &str) -> String {
    let on = matches!(rest, "1" | "true" | "on");
    let flag = if on { "--power-chip" } else { "--stop-chip" };
    match run_self_su1000(flag) {
        Ok(_) => format!("ok enabled={}\n", on as u8),
        Err(e) => format!("err set-enabled {e}\n"),
    }
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
        // M1 binder rewrite: apply the leased address via INetd FIRST (uid system),
        // replacing the root `ip addr` shell-out — so the connected route seeds
        // correctly before setup_network.
        let addr_ok = wandr_hal_net::apply_address_netd(&iface, local_ip, prefix);
        let subnet = wandr_hal_net::subnet_cidr(local_ip, prefix);
        let net_ok = wandr_hal_net::setup_network(netid, &iface, &subnet, gw);
        // Assign all UIDs to the network — the CS fwmark/UID-range mechanism (binder)
        // replacing the rtnetlink catch-all `from all lookup` rule. Belt-and-braces
        // with networkSetDefault, and correct for any future non-root guest uid.
        let uids_ok = wandr_hal_net::add_default_uid_ranges(netid);
        let dns_ok = !servers.is_empty()
            && wandr_hal_net::configure_resolver(netid, &iface, &servers, &[]);
        println!("netd-config netid={netid} iface={iface} subnet={subnet} gw={gw} addr={addr_ok} net={net_ok} uids={uids_ok} dns={dns_ok}");
        std::process::exit(if addr_ok && net_ok && dns_ok { 0 } else { 1 });
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

    // `--scan` — M2: trigger a WiFi scan via IWificond (the nl80211 service that
    // survives `--no-art`) + print the access points, one per line. Read-only (does
    // not disturb an existing association). Engine half — the WIT `wifi.scan` +
    // arbiter relay is a later slice.
    if args.iter().any(|a| a == "--scan") {
        match wandr_hal_net::scan_networks() {
            Some(aps) => {
                println!("scan count={}", aps.len());
                for ap in &aps {
                    println!(
                        "ap ssid={:?} bssid={} rssi={} freq={} sec={} connected={}",
                        ap.ssid, ap.bssid, ap.rssi_dbm, ap.frequency_mhz, ap.security, ap.connected
                    );
                }
                std::process::exit(0);
            }
            None => {
                eprintln!("scan failed (IWificond unreachable / no client interface)");
                std::process::exit(1);
            }
        }
    }

    // `--connect <ssid> <psk>` — M2: associate to an EXPLICIT network (the
    // generalized ISupplicant path) + full IP bring-up, then monitor (persist the
    // link). Forces re-association even if already connected to another SSID. The
    // engine half of `wifi.connect-new`; the WIT + arbiter relay come later.
    if let Some(pos) = args.iter().position(|a| a == "--connect") {
        let rest = &args[pos + 1..];
        let ssid = rest.first().cloned().unwrap_or_default();
        let psk = rest.get(1).cloned().unwrap_or_default();
        if ssid.is_empty() || psk.is_empty() {
            eprintln!("usage: wandr-net --connect <ssid> <psk>");
            std::process::exit(2);
        }
        match bring_up_with(WifiCreds { ssid: ssid.clone(), psk }, true) {
            Ok(link) => {
                report(&link.status);
                println!(
                    "connect ONLINE ssid={:?} ip={:?} gw={:?}",
                    link.status.ssid, link.status.ip, link.status.gateway
                );
                // Manual `--connect` test path: persist (hold the supplicant alive).
                // It does NOT serve the control socket — the persistent daemon owns
                // that; this is a one-off bring-up tool.
                monitor(Arc::new(Mutex::new(link)));
                return;
            }
            Err(e) => {
                eprintln!("connect failed: {e}");
                std::process::exit(1);
            }
        }
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
            // The persistent daemon owns the link; share it with the control-socket
            // server (task 90 M2 wifi relay) and the monitor loop.
            let shared = Arc::new(Mutex::new(link));
            {
                let s = shared.clone();
                std::thread::spawn(move || serve_control(s));
            }
            monitor(shared);
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
/// learns the current state. Shares the live [`Link`] with the control-socket
/// server (an explicit `connect` swaps it under the same lock).
fn monitor(link: Arc<Mutex<Link>>) {
    let mut ticks: u32 = 0;
    loop {
        std::thread::sleep(Duration::from_secs(5));
        ticks += 1;

        if !has_carrier(WLAN_IF) {
            log::warn!("wandr-net: {WLAN_IF} lost carrier — re-associating");
            report(&LinkStatus::default());
            match bring_up() {
                Ok(new) => {
                    let status = new.status.clone();
                    *link.lock().unwrap_or_else(|e| e.into_inner()) = new;
                    report(&status);
                }
                Err(e) => {
                    log::error!("wandr-net: re-bring-up failed: {e} — retrying next tick");
                }
            }
            continue;
        }

        // Re-announce roughly every 30s so arbiter restarts pick us up.
        if ticks % 6 == 0 {
            let status = link.lock().unwrap_or_else(|e| e.into_inner()).status.clone();
            report(&status);
        }
    }
}
