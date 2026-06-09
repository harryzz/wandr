// Trimmed by wandr — structured form of wificond's ChannelSettings (C++
// writeToParcel writes only `frequency`). Only ever sent as part of an EMPTY list
// in a full scan, so the field set is minimal. Task 90 M2.
package android.net.wifi.nl80211;

parcelable ChannelSettings {
    int frequency;
}
