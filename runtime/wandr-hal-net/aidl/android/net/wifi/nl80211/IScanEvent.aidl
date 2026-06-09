// wificond's scan-completion callback (task 90 M2). We implement this as a Bn
// (server) object and pass it to IWifiScannerImpl.subscribeScanEvents; wificond
// calls OnScanResultReady when a triggered scan's neighbor list is ready (the
// WifiService coordination pattern — a blind scan() without a subscribed handler
// is rejected). Method order = transaction codes (wificond calls our object).
package android.net.wifi.nl80211;

interface IScanEvent {
    oneway void OnScanResultReady();             // code 0
    oneway void OnScanFailed();                  // code 1
    oneway void OnScanRequestFailed(int errorCode); // code 2
}
