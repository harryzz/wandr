//! WiFi SCAN via `IWificond` (task 90 M2) — the nl80211 service `wifinl80211`,
//! which survives `--no-art` (WifiService doesn't, but wificond is a native daemon).
//!
//! Flow (all binder, no shell): `IWificond.GetClientInterfaces()` (or
//! `createClientInterface` if none) → `IClientInterface.getWifiScannerImpl()` →
//! `scan(SingleScanSettings)` to trigger a fresh single-shot scan → after a short
//! settle, `getScanResults()` returns `NativeScanResult[]` synchronously (no Bn
//! callback — the scoped no-callback choice, same as the ISupplicant path). Each
//! result's `infoElement` IEs give the security type (RSN/WPA AKM suites).
//!
//! The wificond parcelables are `cpp_header` unstructured upstream; we re-declared
//! the scan ones as structured AIDL with fields in the exact C++ wire order
//! (`aidl/.../NativeScanResult.aidl`), so rsbinder marshals them identically.

#![cfg(target_os = "android")]
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, clippy::all)]

mod binder_aidl {
    include!(concat!(env!("OUT_DIR"), "/iwificond_bindings.rs"));
}

use binder_aidl::android::net::wifi::nl80211::{
    IClientInterface::IClientInterface, IScanEvent, IWificond::IWificond,
};
use rsbinder::Interface;
use std::sync::mpsc::{Receiver, Sender};

/// Outcome wificond reports through our `IScanEvent` callback.
enum ScanOutcome {
    Ready,
    Failed,
}

/// Our `IScanEvent` Bn (server) object — wificond calls it when a triggered scan's
/// neighbor list is ready (`OnScanResultReady`) or fails. It just forwards the
/// outcome to the scan thread over a channel.
struct ScanEventHandler {
    tx: Sender<ScanOutcome>,
}
impl Interface for ScanEventHandler {}

#[async_trait::async_trait]
impl IScanEvent::IScanEventAsyncService for ScanEventHandler {
    async fn r#OnScanResultReady(&self) -> rsbinder::status::Result<()> {
        let _ = self.tx.send(ScanOutcome::Ready);
        Ok(())
    }
    async fn r#OnScanFailed(&self) -> rsbinder::status::Result<()> {
        let _ = self.tx.send(ScanOutcome::Failed);
        Ok(())
    }
    async fn r#OnScanRequestFailed(&self, _error_code: i32) -> rsbinder::status::Result<()> {
        let _ = self.tx.send(ScanOutcome::Failed);
        Ok(())
    }
}

/// Minimal `BinderAsyncRuntime` — `block_on` a future to completion by polling with
/// a no-op waker. Our `IScanEvent` service futures only set a flag, so they're
/// ready on the first poll; this avoids pulling a full async runtime (tokio) into
/// the stack just to dispatch a oneway callback.
struct BlockingRuntime;
impl rsbinder::BinderAsyncRuntime for BlockingRuntime {
    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn raw() -> RawWaker {
            fn noop(_: *const ()) {}
            fn clone(_: *const ()) -> RawWaker {
                raw()
            }
            RawWaker::new(std::ptr::null(), &RawWakerVTable::new(clone, noop, noop, noop))
        }
        let waker = unsafe { Waker::from_raw(raw()) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(future);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }
}
use std::sync::OnceLock;
use std::time::Duration;

use crate::ScanEntry;

/// The servicemanager name wificond registers (NOT the interface descriptor):
/// `wifinl80211: [android.net.wifi.nl80211.IWificond]`.
const SVC_NAME: &str = "wifinl80211";

/// wificond's high-accuracy single scan (`IWifiScannerImpl.SCAN_TYPE_HIGH_ACCURACY`).
const SCAN_TYPE_HIGH_ACCURACY: i32 = 2;

fn ensure_process_state() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        if rsbinder::ProcessState::init_default().is_ok() {
            rsbinder::ProcessState::start_thread_pool();
        }
    });
}

/// Trigger a scan + return the access points wificond reports. `None` on a binder
/// failure (service missing / SELinux / no client interface).
pub fn scan() -> Option<Vec<ScanEntry>> {
    ensure_process_state();
    let wificond = match rsbinder::hub::get_interface::<dyn IWificond>(SVC_NAME) {
        Ok(w) => w,
        Err(e) => {
            log::error!("wificond-scan: get_interface({SVC_NAME}) failed: {e:?}");
            return None;
        }
    };

    // Existing client interface (created during association) — else create one.
    let client = match get_client_interface(&wificond) {
        Some(c) => c,
        None => {
            log::error!("wificond-scan: no client interface for wlan0");
            return None;
        }
    };

    let scanner = match client.r#getWifiScannerImpl() {
        Ok(Some(s)) => s,
        Ok(None) => {
            log::error!("wificond-scan: getWifiScannerImpl returned null");
            return None;
        }
        Err(e) => {
            log::error!("wificond-scan: getWifiScannerImpl failed: {e:?}");
            return None;
        }
    };
    // Subscribe a scan-completion callback FIRST — wificond rejects a blind scan
    // (the WifiService coordination pattern); with a subscribed IScanEvent it runs
    // the scan + signals OnScanResultReady when the neighbor list is ready.
    let (tx, rx): (Sender<ScanOutcome>, Receiver<ScanOutcome>) = std::sync::mpsc::channel();
    let handler = IScanEvent::BnScanEvent::new_async_binder(ScanEventHandler { tx }, BlockingRuntime);
    if let Err(e) = scanner.r#subscribeScanEvents(&handler) {
        log::warn!("wificond-scan: subscribeScanEvents failed: {e:?}");
    }

    // wificond's NativeScanResult/SingleScanSettings are `cpp_header` parcelables
    // (no structured length-prefix header), so the generated typed scan()/
    // getScanResults() marshalling is offset. We raw-transact instead, writing/
    // reading the fields exactly as the C++ {write,read}FromParcel do.
    // Build the scan frequency list from wificond's available channels (WifiService
    // scans an explicit channel set, never "all channels").
    let freqs = enumerate_channels(&wificond);
    log::info!("wificond-scan: scanning {} channels", freqs.len());

    let binder = scanner.as_binder();
    let Some(remote) = binder.as_remote() else {
        log::error!("wificond-scan: scanner is not a remote proxy");
        return None;
    };

    raw_scan_trigger(remote, &freqs);

    // Wait for OnScanResultReady (nl80211 scans take ~1.5–4 s). On failure / timeout
    // fall through to whatever wificond has cached.
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(ScanOutcome::Ready) => log::info!("wificond-scan: OnScanResultReady"),
        Ok(ScanOutcome::Failed) => log::warn!("wificond-scan: scan reported failed"),
        Err(_) => log::warn!("wificond-scan: scan-ready timeout — using cached results"),
    }

    let results = raw_get_scan_results(remote);
    let _ = scanner.r#unsubscribeScanEvents();
    results
}

/// Raw `scan(SingleScanSettings)` — transaction code `FIRST_CALL_TRANSACTION + 3`.
/// Writes the cpp_header SingleScanSettings raw with an EXPLICIT channel list
/// (`freqs`, MHz) — matching `WificondScannerImpl`, which always scans a concrete
/// frequency set (an empty list is treated as "no channel to scan" = failure).
fn raw_scan_trigger(remote: &dyn rsbinder::RemoteProxy, freqs: &[i32]) {
    let mut data = match remote.prepare_transact(true) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("wificond-scan: prepare scan transact failed: {e:?}");
            return;
        }
    };
    // The arg is a parcelable read via readParcelable → a leading int32(1)
    // non-null/presence marker precedes the fields (without it the stub reads null
    // → EX_NULL_POINTER). Then SingleScanSettings::writeToParcel order: scanType,
    // enable6ghzRnr, channel_settings (typed list), hidden_networks, vendor_ies.
    let _ = data.write(&1i32); // non-null parcelable presence marker
    let _ = data.write(&SCAN_TYPE_HIGH_ACCURACY);
    let _ = data.write(&false); // enable_6ghz_rnr
    let _ = data.write(&(freqs.len() as i32)); // channel_settings.size()
    for f in freqs {
        let _ = data.write(&1i32); // leading presence (writeTypedList)
        let _ = data.write(f); // ChannelSettings.frequency
    }
    let _ = data.write(&0i32); // hidden_networks.size()
    let _ = data.write(&Vec::<u8>::new()); // vendor_ies (empty byte vector)
    // scan() (code 3) → bool accepted. With the IScanEvent subscribed (now at the
    // correct code 4) the trigger is honored; results land via OnScanResultReady.
    const SCAN_CODE: rsbinder::TransactionCode = rsbinder::FIRST_CALL_TRANSACTION + 3;
    match remote.submit_transact(SCAN_CODE, &data, rsbinder::FLAG_CLEAR_BUF) {
        Ok(Some(mut reply)) => {
            let accepted = reply
                .read::<rsbinder::Status>()
                .ok()
                .filter(|s| s.is_ok())
                .and_then(|_| reply.read::<bool>().ok())
                .unwrap_or(false);
            log::info!("wificond-scan: scan() accepted={accepted}");
        }
        Ok(None) => log::warn!("wificond-scan: scan() empty reply"),
        Err(e) => log::warn!("wificond-scan: scan() transact failed: {e:?}"),
    }
}

/// Raw `getScanResults()` — transaction code `FIRST_CALL_TRANSACTION + 0`. Reads
/// the AIDL status header, then `int32 count`, then each NativeScanResult raw
/// (cpp_header layout). Returns `None` on a transact/parse failure.
fn raw_get_scan_results(remote: &dyn rsbinder::RemoteProxy) -> Option<Vec<ScanEntry>> {
    const CODE: rsbinder::TransactionCode = rsbinder::FIRST_CALL_TRANSACTION + 0;
    let data = remote.prepare_transact(true).ok()?;
    let mut reply = remote
        .submit_transact(CODE, &data, rsbinder::FLAG_CLEAR_BUF)
        .ok()??;
    let status = reply.read::<rsbinder::Status>().ok()?;
    if !status.is_ok() {
        log::warn!("wificond-scan: getScanResults status not ok: {status:?}");
        return None;
    }
    let count = reply.read::<i32>().ok()?;
    if !(0..=512).contains(&count) {
        log::warn!("wificond-scan: implausible result count {count}");
        return None;
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        // Each element is prefixed with a `1` presence marker (writeTypedVector
        // convention), confirmed from the on-device reply framing.
        let _leading: i32 = reply.read().ok()?;
        match read_native_scan_result(&mut reply) {
            Some(e) => out.push(e),
            None => break,
        }
    }
    Some(out)
}

/// Read one NativeScanResult in the cpp_header wire order + map to a [`ScanEntry`].
fn read_native_scan_result(reply: &mut rsbinder::Parcel) -> Option<ScanEntry> {
    let ssid: Vec<u8> = reply.read().ok()?;
    let bssid: Vec<u8> = reply.read().ok()?;
    let info_element: Vec<u8> = reply.read().ok()?;
    let frequency: i32 = reply.read().ok()?;
    let signal_mbm: i32 = reply.read().ok()?;
    let _tsf: i64 = reply.read().ok()?;
    let capability: i32 = reply.read().ok()?;
    let associated: bool = reply.read().ok()?;
    // radioChainInfos: int32 count, then per element int32(leading==1) + (chainId, level).
    let n: i32 = reply.read().ok()?;
    for _ in 0..n.clamp(0, 16) {
        let _leading: i32 = reply.read().ok()?;
        let _chain_id: i32 = reply.read().ok()?;
        let _level: i32 = reply.read().ok()?;
    }
    Some(ScanEntry {
        ssid: String::from_utf8_lossy(&ssid).to_string(),
        bssid: bssid.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"),
        frequency_mhz: frequency as u32,
        rssi_dbm: signal_mbm / 100,
        security: security_from_ies(&info_element, capability as u16).to_string(),
        connected: associated,
    })
}

/// All scannable channel frequencies (MHz) wificond reports — 2.4 GHz + 5 GHz
/// (non-DFS + DFS). The explicit set `WificondScannerImpl` passes to `scan()`.
fn enumerate_channels(wificond: &rsbinder::Strong<dyn IWificond>) -> Vec<i32> {
    let mut freqs = Vec::new();
    if let Ok(Some(ch)) = wificond.r#getAvailable2gChannels() {
        freqs.extend(ch);
    }
    if let Ok(Some(ch)) = wificond.r#getAvailable5gNonDFSChannels() {
        freqs.extend(ch);
    }
    if let Ok(Some(ch)) = wificond.r#getAvailableDFSChannels() {
        freqs.extend(ch);
    }
    freqs
}

/// Existing client interface for wlan0 (cast the first `GetClientInterfaces`
/// handle), or create one via `createClientInterface`.
fn get_client_interface(
    wificond: &rsbinder::Strong<dyn IWificond>,
) -> Option<rsbinder::Strong<dyn IClientInterface>> {
    if let Ok(binders) = wificond.r#GetClientInterfaces() {
        for b in binders {
            if let Ok(c) = <dyn IClientInterface as rsbinder::FromIBinder>::try_from(b) {
                return Some(c);
            }
        }
    }
    match wificond.r#createClientInterface("wlan0") {
        Ok(Some(c)) => Some(c),
        _ => None,
    }
}

/// Derive the security type from the 802.11 information elements + the capability
/// field's Privacy bit. RSN IE (id 48) → WPA2/WPA3 by AKM suite; WPA vendor IE
/// (id 221, OUI 00-50-F2 type 1) → WPA; Privacy-only → WEP (mapped to wpa-psk, the
/// closest WIT kind); none → open.
fn security_from_ies(ie: &[u8], capability: u16) -> &'static str {
    const RSN_ID: u8 = 48;
    const VENDOR_ID: u8 = 221;
    const WPA_OUI: [u8; 4] = [0x00, 0x50, 0xf2, 0x01]; // Microsoft OUI + WPA type 1
    // AKM suite selectors (OUI 00-0F-AC + type).
    const AKM_OUI: [u8; 3] = [0x00, 0x0f, 0xac];

    let mut has_wpa = false;
    let mut i = 0;
    while i + 2 <= ie.len() {
        let id = ie[i];
        let len = ie[i + 1] as usize;
        let body = &ie[i + 2..(i + 2 + len).min(ie.len())];
        if id == RSN_ID {
            if let Some(akms) = rsn_akm_suites(body) {
                // Priority: SAE (WPA3) > OWE > EAP > PSK.
                if akms.iter().any(|&t| t == 8) {
                    return "sae";
                }
                if akms.iter().any(|&t| t == 18) {
                    return "owe";
                }
                if akms.iter().any(|&t| t == 1 || t == 5) {
                    return "wpa-eap";
                }
                if akms.iter().any(|&t| t == 2 || t == 6) {
                    return "wpa-psk";
                }
            }
            return "wpa-psk"; // RSN present but unparsed AKM → treat as PSK
        }
        if id == VENDOR_ID && body.len() >= 4 && body[0..4] == WPA_OUI {
            has_wpa = true;
        }
        i += 2 + len;
    }
    if has_wpa {
        return "wpa-psk";
    }
    if capability & 0x0010 != 0 {
        return "wpa-psk"; // Privacy bit, no RSN/WPA → WEP; nearest WIT kind
    }
    let _ = AKM_OUI;
    "open"
}

/// Parse the AKM suite *types* (last byte of each 00-0F-AC selector) from an RSN
/// IE body: version(2) + group(4) + pairwiseCount(2) + pairwise(4×n) +
/// akmCount(2) + akm(4×m). Returns the m AKM type bytes; `None` if truncated.
fn rsn_akm_suites(body: &[u8]) -> Option<Vec<u8>> {
    let mut p = 2usize; // skip version
    p += 4; // group cipher
    if p + 2 > body.len() {
        return None;
    }
    let pcount = u16::from_le_bytes([body[p], body[p + 1]]) as usize;
    p += 2 + 4 * pcount;
    if p + 2 > body.len() {
        return None;
    }
    let acount = u16::from_le_bytes([body[p], body[p + 1]]) as usize;
    p += 2;
    let mut out = Vec::with_capacity(acount);
    for _ in 0..acount {
        if p + 4 > body.len() {
            break;
        }
        out.push(body[p + 3]); // suite type = 4th byte
        p += 4;
    }
    Some(out)
}
