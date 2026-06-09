// Trimmed by wandr — structured form of wificond's `parcelable RadioChainInfo
// cpp_header "..."`, fields in the C++ readFromParcel order (chain_id, level) so
// rsbinder marshals it wire-identically to the device's wificond. Task 90 M2.
package android.net.wifi.nl80211;

parcelable RadioChainInfo {
    int chainId;
    int level;
}
