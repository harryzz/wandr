// Trimmed by wandr — structured form of wificond's `parcelable NativeScanResult
// cpp_header "wificond/scanning/scan_result.h"`. Field order is LOAD-BEARING: it
// mirrors the C++ NativeScanResult::readFromParcel exactly (ssid, bssid,
// infoElement, frequency:u32, signalMbm:i32, tsf:u64, capability:u32, associated,
// radioChainInfos as a typed list) so rsbinder marshals it wire-identically to the
// device's wificond. AIDL `int` is i32 / `long` is i64 — the unsigned C++ fields
// are reinterpreted in Rust. Task 90 M2.
package android.net.wifi.nl80211;

import android.net.wifi.nl80211.RadioChainInfo;

parcelable NativeScanResult {
    byte[] ssid;
    byte[] bssid;
    byte[] infoElement;
    int frequency;
    int signalMbm;
    long tsf;
    int capability;
    boolean associated;
    List<RadioChainInfo> radioChainInfos;
}
