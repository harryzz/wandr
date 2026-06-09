//! wandr-hal-net — ART-off WiFi/IP bring-up primitives (task 88, M1).
//!
//! The "applies" half of the connectivity triad: drives the native survivors to
//! turn a powered-but-idle WiFi chip into a working default route under
//! `--no-art`. Three pieces, each a submodule:
//!   * [`supplicant`] — spawn the vendor `wpa_supplicant` + nudge its ctrl socket
//!     to associate to a saved network (the proven recipe).
//!   * [`dhcp`] — a small pure-Rust DHCPv4 client (the one missing native binary).
//!   * this module — read saved creds + apply the leased address/route via the
//!     on-device `ip` tool (exactly the demo's `ip addr` / `ip route` commands).
//!
//! No binder/AIDL in M1 (route is rtnetlink via `ip`; DNS is handled by the
//! daemon). Off-android the binary entry points are inert so callers stay
//! `cfg`-free — but the pure logic (DHCP parse, cred parse) builds everywhere.

pub mod dhcp;
pub mod supplicant;

#[cfg(target_os = "android")]
mod netd;

#[cfg(target_os = "android")]
mod supplicant_hal;

#[cfg(target_os = "android")]
mod wifi_chip;

#[cfg(target_os = "android")]
mod wificond_scan;

/// One scanned access point (the engine-half result; the WIT `scan-result`
/// mapping is a later slice). `security` is `open|owe|wpa-psk|sae|wpa-eap`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanEntry {
    pub ssid: String,
    pub bssid: String,
    pub frequency_mhz: u32,
    pub rssi_dbm: i32,
    pub security: String,
    pub connected: bool,
}

/// Trigger a WiFi scan via `IWificond` (the nl80211 service, survives `--no-art`)
/// and return the access points. `None` off-android / on binder failure. Read-only
/// w.r.t. the link (a scan doesn't disturb an existing association).
pub fn scan_networks() -> Option<Vec<ScanEntry>> {
    #[cfg(target_os = "android")]
    {
        wificond_scan::scan()
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// Cold-boot chip power-up: drive the `IWifi` HAL to power the chip + create the
/// STA interface (what WifiService does). Requires uid `system`. Returns the STA
/// iface name on success. `None` off-android / on failure.
pub fn ensure_chip_up(iface: &str) -> Option<String> {
    #[cfg(target_os = "android")]
    {
        if wifi_chip::ensure_chip_up(iface) {
            Some(iface.to_string())
        } else {
            None
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = iface;
        None
    }
}

/// Power the WiFi chip down via the `IWifi` HAL (test / teardown). uid `system`.
pub fn stop_chip() -> bool {
    #[cfg(target_os = "android")]
    {
        wifi_chip::stop_chip()
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Remove the STA interface via the chip (genuine cold state for testing; clean
/// teardown). uid `system`.
pub fn remove_sta_iface(iface: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        wifi_chip::remove_sta_iface(iface)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = iface;
        false
    }
}

/// M2 probe: can this process reach the vendor ISupplicant HAL over binder (as
/// whatever uid it runs)? Read-only. `false` off-android.
pub fn probe_supplicant() -> bool {
    #[cfg(target_os = "android")]
    {
        supplicant_hal::probe()
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Associate `iface` to `ssid`/`psk` via the ISupplicant AIDL HAL (the
/// Android-native path). Requires the HAL-managed supplicant
/// ([`supplicant::spawn_hal`]) and uid `system`. Triggers `select()`; the caller
/// confirms via [`has_carrier`]. `false` off-android / on failure.
pub fn associate_supplicant(iface: &str, ssid: &str, psk: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        supplicant_hal::associate(iface, ssid, psk)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (iface, ssid, psk);
        false
    }
}

/// The network (on-link) CIDR for an address + prefix, e.g. `192.168.1.179`/`22`
/// → `"192.168.0.0/22"`. Used as netd's connected route.
pub fn subnet_cidr(ip: Ipv4Addr, prefix: u8) -> String {
    let mask: u32 = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix as u32) };
    let net = u32::from(ip) & mask;
    format!("{}/{}", Ipv4Addr::from(net), prefix)
}

/// Create + default a netd physical network for `netid` over `iface` with the
/// connected route `subnet_cidr` and the default route via `gateway` (the
/// netd-native replacement for the `ip rule/route` bypass). **Must be called as
/// uid `system`.** No-op / `false` off-android.
pub fn setup_network(netid: i32, iface: &str, subnet_cidr: &str, gateway: Ipv4Addr) -> bool {
    #[cfg(target_os = "android")]
    {
        netd::setup_network(netid, iface, subnet_cidr, gateway)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (netid, iface, subnet_cidr, gateway);
        false
    }
}

/// Apply the leased IPv4 address to `iface` via `INetd` (binder — replaces the
/// `ip addr flush`/`ip addr add` shell-outs). **Must be called as uid `system`**
/// (netd rejects root for interface config). No-op / `false` off-android.
pub fn apply_address_netd(iface: &str, ip: Ipv4Addr, prefix: u8) -> bool {
    #[cfg(target_os = "android")]
    {
        netd::apply_address(iface, ip, prefix)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (iface, ip, prefix);
        false
    }
}

/// Configure the netd DNS resolver for `netid` (binding it to `iface`) with the
/// given IPv4 servers. The only way to make `getaddrinfo` resolve under
/// `--no-art` (the resolver moved to the `dnsresolver` binder; no `ndc` command).
/// **Must be called as uid `system`** (dnsresolver rejects root). No-op /
/// `false` off-android.
pub fn configure_resolver(netid: i32, iface: &str, servers: &[Ipv4Addr], domains: &[String]) -> bool {
    #[cfg(target_os = "android")]
    {
        netd::set_resolver(netid, iface, servers, domains)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (netid, iface, servers, domains);
        false
    }
}

use std::io;
use std::net::Ipv4Addr;
use std::process::Command;

pub use dhcp::DhcpLease;
pub use supplicant::WifiCreds;

/// Default WiFi station interface on this device family.
pub const WLAN_IF: &str = "wlan0";

/// Where the framework persists saved networks (plaintext PSK on this device —
/// no keystore EncryptedData; root-readable under `--no-art`).
pub const WIFI_CONFIG_STORE: &str = "/data/misc/apexdata/com.android.wifi/WifiConfigStore.xml";

/// The netId of the single wandr-managed physical network under `--no-art`. netd
/// keys every network by an integer handle; ConnectivityService (dead here)
/// normally allocates these. We own exactly one network, so one named constant is
/// the single source of truth (the no-hardcoding rule's sanctioned form). 100 is
/// in netd's app-network range (`>= 100`) and cannot collide because no other
/// networks exist with CS gone. Multi-network allocation is M2.
pub const WANDR_NETID: i32 = 100;

/// Priority of the catch-all `from all lookup <iface-table>` ip-rule. netd routes
/// per-network traffic by **fwmark** (rules from priority 16000), which
/// ConnectivityService assigns per-UID — dead under `--no-art`, so unmarked
/// (fwmark 0) traffic finds no network and falls to `32000: unreachable`. This
/// one rule points all unmarked traffic at the netd per-network table (named
/// after the interface), the deliberate single-network bypass of CS's per-UID
/// fwmark policy. Must sit *below* netd's first fwmark rule (16000) and above
/// `local` (0); the one justified source of truth for this policy.
pub const WANDR_RULE_PREF: u32 = 15000;

/// A connectivity snapshot the daemon reports up to the arbiter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkStatus {
    pub up: bool,
    pub ssid: Option<String>,
    pub ip: Option<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
}

/// Parse `WifiConfigStore.xml` for saved WPA-PSK networks. Returns the
/// `(ssid, psk)` pairs in file order; the daemon tries them in turn. The store
/// holds each network's SSID and PreSharedKey as XML-escaped quoted strings:
/// `<string name="SSID">&quot;MyNet&quot;</string>` /
/// `<string name="PreSharedKey">&quot;secret&quot;</string>`.
pub fn parse_saved_creds(xml: &str) -> Vec<WifiCreds> {
    // Pull the value of a `<string name="FIELD">VALUE</string>` element, with the
    // surrounding `&quot;` and XML entities decoded. Kept dependency-free (no XML
    // crate) — the store is flat enough that a positional scan is robust.
    fn field<'a>(block: &'a str, name: &str) -> Option<String> {
        let key = format!("name=\"{name}\">");
        let start = block.find(&key)? + key.len();
        let end = block[start..].find("</string>")? + start;
        Some(unescape(&block[start..end]))
    }
    fn unescape(s: &str) -> String {
        s.replace("&quot;", "")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&apos;", "'")
            .trim()
            .to_string()
    }

    let mut out = Vec::new();
    // Each saved network is a `<Network> … </Network>` (or `<WifiConfiguration>`)
    // block; split on the close tag and scan each chunk.
    for block in xml.split("</Network>") {
        let ssid = field(block, "SSID");
        let psk = field(block, "PreSharedKey");
        if let (Some(ssid), Some(psk)) = (ssid, psk) {
            if !ssid.is_empty() && !psk.is_empty() {
                out.push(WifiCreds { ssid, psk });
            }
        }
    }
    out
}

/// Read + parse the on-device WifiConfigStore (root-readable under `--no-art`).
pub fn read_saved_creds() -> io::Result<Vec<WifiCreds>> {
    let xml = std::fs::read_to_string(WIFI_CONFIG_STORE)?;
    Ok(parse_saved_creds(&xml))
}

/// Configure the leased IPv4 **address** on the interface (bring the link up,
/// flush stale addresses, set the leased one). Needs root (`CAP_NET_ADMIN`).
/// Routing is owned by netd ([`setup_network`], run as uid `system`), not here —
/// this only puts the address on the link so netd can build the connected route.
/// Link must be `up` before `addr add` so the kernel installs the connected route.
pub fn apply_address(ifname: &str, lease: &DhcpLease) -> io::Result<()> {
    run_ip(&["link", "set", ifname, "up"])?;
    run_ip(&["addr", "flush", "dev", ifname])?;
    run_ip(&["addr", "add", &format!("{}/{}", lease.ip, lease.prefix), "dev", ifname])?;
    log::info!("link: address {}/{} on {ifname}", lease.ip, lease.prefix);
    Ok(())
}

/// Install the catch-all routing rule (see [`WANDR_RULE_PREF`]) so unmarked
/// traffic uses netd's per-network table for `ifname` (netd names the table after
/// the interface). Needs root. Idempotent: delete-then-add so re-runs don't stack
/// duplicates. Pairs with [`setup_network`], which populates that table.
pub fn install_default_rule(ifname: &str) -> io::Result<()> {
    let pref = WANDR_RULE_PREF.to_string();
    let _ = run_ip(&["rule", "del", "pref", &pref]); // best-effort (absent first run)
    run_ip(&["rule", "add", "pref", &pref, "from", "all", "lookup", ifname])?;
    log::info!("link: catch-all rule {pref} → table {ifname}");
    Ok(())
}

/// True if the interface has L2 carrier (associated) — `operstate == up` or
/// `/sys/class/net/<if>/carrier == 1`.
pub fn has_carrier(ifname: &str) -> bool {
    std::fs::read_to_string(format!("/sys/class/net/{ifname}/carrier"))
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

fn run_ip(args: &[&str]) -> io::Result<()> {
    let out = Command::new("ip").args(args).output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("`ip {}` failed: {}", args.join(" "), stderr.trim()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_creds_extracts_ssid_psk() {
        // Abbreviated WifiConfigStore.xml shape (two networks).
        let xml = r#"
        <WifiConfigStoreData>
         <NetworkList>
          <Network>
           <WifiConfiguration>
            <string name="SSID">&quot;HomeNet&quot;</string>
            <string name="PreSharedKey">&quot;hunter2pass&quot;</string>
           </WifiConfiguration>
          </Network>
          <Network>
           <WifiConfiguration>
            <string name="SSID">&quot;Cafe&amp;Bar&quot;</string>
            <string name="PreSharedKey">&quot;latte&quot;</string>
           </WifiConfiguration>
          </Network>
         </NetworkList>
        </WifiConfigStoreData>
        "#;
        let creds = parse_saved_creds(xml);
        assert_eq!(creds.len(), 2);
        assert_eq!(creds[0].ssid, "HomeNet");
        assert_eq!(creds[0].psk, "hunter2pass");
        assert_eq!(creds[1].ssid, "Cafe&Bar");
        assert_eq!(creds[1].psk, "latte");
    }

    #[test]
    fn parse_creds_skips_open_networks() {
        // A network with SSID but no PreSharedKey (open) is skipped in M1.
        let xml = r#"<Network><string name="SSID">&quot;Open&quot;</string></Network>"#;
        assert!(parse_saved_creds(xml).is_empty());
    }
}
