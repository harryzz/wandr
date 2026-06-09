// Trimmed by wandr — structured form of wificond's HiddenNetwork. Only ever sent
// as part of an EMPTY list in a full scan, so the field set is minimal. Task 90 M2.
package android.net.wifi.nl80211;

parcelable HiddenNetwork {
    byte[] ssid;
}
