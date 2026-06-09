//! DNS resolver configuration via the `android.net.IDnsResolver` binder (task 88,
//! M1b). The DNS resolver moved out of netd's command interface into the
//! `dnsresolver` mainline service — there is no `ndc` command for it — so the
//! only way to configure name resolution under `--no-art` is this binder call.
//!
//! CALLER UID MATTERS: dnsresolver (like netd) accepts **uid `system`**, not
//! root (its permission check special-cases `AID_SYSTEM`). The process making
//! these calls must therefore run as uid 1000 (e.g. via `wandr-launch` or
//! `su 1000`) — calling as root returns a permission failure, which is exactly
//! why the hand-bring-up demo (run as root) couldn't configure DNS.

#![cfg(target_os = "android")]
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, clippy::all)]

mod binder_aidl {
    include!(concat!(env!("OUT_DIR"), "/dns_bindings.rs"));
}
mod inetd_aidl {
    include!(concat!(env!("OUT_DIR"), "/inetd_bindings.rs"));
}

use binder_aidl::android::net::{
    IDnsResolver::IDnsResolver, ResolverParamsParcel::ResolverParamsParcel,
};
use inetd_aidl::android::net::INetd::INetd;
use std::net::Ipv4Addr;
use std::sync::OnceLock;

/// INetd.PERMISSION_NONE — a regular (unrestricted) physical network.
const PERMISSION_NONE: i32 = 0;

// Standard resolver tuning (the values ConnectivityService passes). Named
// constants — the one justified source of truth for resolver policy.
const SAMPLE_VALIDITY_SECONDS: i32 = 1800;
const SUCCESS_THRESHOLD: i32 = 25;
const MIN_SAMPLES: i32 = 8;
const MAX_SAMPLES: i32 = 8;
const BASE_TIMEOUT_MSEC: i32 = 5000;
const RETRY_COUNT: i32 = 3;
/// IDnsResolver.TRANSPORT_WIFI.
const TRANSPORT_WIFI: i32 = 1;

fn ensure_process_state() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        if !std::path::Path::new("/dev/binder").exists() {
            log::warn!("wandr-hal-net: /dev/binder absent — dnsresolver unavailable");
            return;
        }
        if rsbinder::ProcessState::init_default().is_ok() {
            rsbinder::ProcessState::start_thread_pool();
        }
    });
}

fn service() -> Option<rsbinder::Strong<dyn IDnsResolver>> {
    ensure_process_state();
    match rsbinder::hub::get_interface::<dyn IDnsResolver>("dnsresolver") {
        Ok(svc) => Some(svc),
        Err(e) => {
            log::warn!("wandr-hal-net: get dnsresolver failed: {e:?}");
            None
        }
    }
}

/// Configure the DNS resolver for `netid` with `servers`, binding it to
/// `iface`. Creates the per-net cache first (idempotent). Returns true on
/// success. MUST be called as uid `system`.
pub fn set_resolver(netid: i32, iface: &str, servers: &[Ipv4Addr], domains: &[String]) -> bool {
    let Some(svc) = service() else { return false };

    // The resolver needs a cache for the netId before configuration; creating an
    // existing one throws ServiceSpecificException (EEXIST) — ignore that.
    match svc.r#createNetworkCache(netid) {
        Ok(_) => log::info!("dns: created resolver cache for netId {netid}"),
        Err(e) => log::debug!("dns: createNetworkCache({netid}) -> {e:?} (ok if already exists)"),
    }

    let params = ResolverParamsParcel {
        r#netId: netid,
        r#sampleValiditySeconds: SAMPLE_VALIDITY_SECONDS,
        r#successThreshold: SUCCESS_THRESHOLD,
        r#minSamples: MIN_SAMPLES,
        r#maxSamples: MAX_SAMPLES,
        r#baseTimeoutMsec: BASE_TIMEOUT_MSEC,
        r#retryCount: RETRY_COUNT,
        r#servers: servers.iter().map(|s| s.to_string()).collect(),
        r#domains: domains.to_vec(),
        r#tlsName: String::new(),
        r#tlsServers: Vec::new(),
        r#transportTypes: vec![TRANSPORT_WIFI],
        r#interfaceNames: vec![iface.to_string()],
        ..Default::default()
    };

    match svc.r#setResolverConfiguration(&params) {
        Ok(_) => {
            log::info!(
                "dns: setResolverConfiguration netId={netid} iface={iface} servers={servers:?} OK"
            );
            true
        }
        Err(e) => {
            log::warn!("dns: setResolverConfiguration failed: {e:?}");
            false
        }
    }
}

/// True if the dnsresolver service answers (liveness probe).
pub fn is_alive() -> bool {
    service().and_then(|s| s.r#isAlive().ok()).unwrap_or(false)
}

// ── INetd (network create / interface / route / default) ─────────────────────

fn netd_service() -> Option<rsbinder::Strong<dyn INetd>> {
    ensure_process_state();
    match rsbinder::hub::get_interface::<dyn INetd>("netd") {
        Ok(svc) => Some(svc),
        Err(e) => {
            log::warn!("wandr-hal-net: get netd failed: {e:?}");
            None
        }
    }
}

/// Apply the leased IPv4 address to `iface` via `INetd` (the binder replacement
/// for `ip addr flush` + `ip addr add`). Clears existing addresses first, then
/// adds `ip/prefix`. MUST be called as uid `system` (netd rejects root for
/// interface config). `false` on failure.
pub fn apply_address(iface: &str, ip: Ipv4Addr, prefix: u8) -> bool {
    let Some(svc) = netd_service() else { return false };
    if let Err(e) = svc.r#interfaceClearAddrs(iface) {
        log::debug!("netd: interfaceClearAddrs({iface}) -> {e:?} (ok if none)");
    }
    match svc.r#interfaceAddAddress(iface, &ip.to_string(), prefix as i32) {
        Ok(_) => {
            log::info!("netd: interfaceAddAddress {ip}/{prefix} on {iface}");
            true
        }
        Err(e) => {
            log::warn!("netd: interfaceAddAddress {ip}/{prefix} on {iface} -> {e:?}");
            false
        }
    }
}

/// Assign every UID to network `netid` via `INetd.networkAddUidRanges` — the
/// fwmark/UID-range mechanism ConnectivityService uses so that a UID's *unmarked*
/// traffic routes through a network (the per-UID `ip rule … uidrange … lookup`
/// netd installs match by UID, so fwmark-0 sockets are covered). Assigning all
/// UIDs is the binder replacement for the blunt catch-all `from all lookup` rule.
/// uid `system`. `false` on failure. (Unblocked by the `@JavaOnlyImmutable`
/// rsbinder fix that lets `UidRangeParcel` generate its fields.)
pub fn add_all_uid_ranges(netid: i32) -> bool {
    let Some(svc) = netd_service() else { return false };
    let ranges = vec![inetd_aidl::android::net::UidRangeParcel::UidRangeParcel {
        r#start: 0,
        r#stop: 99999,
    }];
    match svc.r#networkAddUidRanges(netid, &ranges) {
        Ok(_) => {
            log::info!("netd: networkAddUidRanges({netid}, 0..99999) OK");
            true
        }
        Err(e) => {
            log::warn!("netd: networkAddUidRanges({netid}) -> {e:?}");
            false
        }
    }
}

/// Create (if needed) a physical network `netid` over `iface`, install the
/// connected route for `subnet_cidr` plus the default route via `gateway`, and
/// make it the system default network — the netd-native equivalent of the `ip
/// rule/route` bypass. MUST be called as uid `system`. Returns true if the
/// network ended up the default (treating "already exists" as success).
///
/// The connected route is added explicitly because we configure the interface
/// address out-of-band (via `ip`, as root), so `networkAddInterface` doesn't seed
/// the per-net table with it — and without it the default-via-gateway add fails
/// "Network is unreachable" (the gateway isn't yet reachable in that table).
pub fn setup_network(netid: i32, iface: &str, subnet_cidr: &str, gateway: Ipv4Addr) -> bool {
    let Some(svc) = netd_service() else { return false };

    // createPhysical throws if the netId already exists — tolerate it.
    match svc.r#networkCreatePhysical(netid, PERMISSION_NONE) {
        Ok(_) => log::info!("netd: created physical network {netid}"),
        Err(e) => log::debug!("netd: networkCreatePhysical({netid}) -> {e:?} (ok if exists)"),
    }
    if let Err(e) = svc.r#networkAddInterface(netid, iface) {
        log::debug!("netd: networkAddInterface({netid},{iface}) -> {e:?} (ok if already added)");
    }
    // Connected (on-link) route — nextHop "" means directly connected. Must
    // precede the default so the gateway becomes reachable in this table.
    if let Err(e) = svc.r#networkAddRoute(netid, iface, subnet_cidr, "") {
        log::debug!("netd: networkAddRoute connected {subnet_cidr} -> {e:?} (ok if exists)");
    }
    // Default IPv4 route via the gateway.
    if let Err(e) = svc.r#networkAddRoute(netid, iface, "0.0.0.0/0", &gateway.to_string()) {
        log::warn!("netd: networkAddRoute default via {gateway} -> {e:?}");
    }
    match svc.r#networkSetDefault(netid) {
        Ok(_) => {
            log::info!("netd: network {netid} (iface {iface}) set as system default");
            true
        }
        Err(e) => {
            log::warn!("netd: networkSetDefault({netid}) -> {e:?}");
            false
        }
    }
}
