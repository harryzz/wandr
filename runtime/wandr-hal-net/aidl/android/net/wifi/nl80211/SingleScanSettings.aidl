// Trimmed by wandr — structured form of wificond's `parcelable SingleScanSettings
// cpp_header "..."`. Field order mirrors the C++ SingleScanSettings::writeToParcel
// (scanType:i32, enable6ghzRnr:bool, channelSettings list, hiddenNetworks list,
// vendorIes byte[]) so an all-empty value (full scan: default type, no channel
// restriction, no hidden SSIDs, no vendor IEs) marshals wire-identically. Task 90 M2.
package android.net.wifi.nl80211;

import android.net.wifi.nl80211.ChannelSettings;
import android.net.wifi.nl80211.HiddenNetwork;

parcelable SingleScanSettings {
    int scanType;
    boolean enable6ghzRnr;
    List<ChannelSettings> channelSettings;
    List<HiddenNetwork> hiddenNetworks;
    byte[] vendorIes;
}
