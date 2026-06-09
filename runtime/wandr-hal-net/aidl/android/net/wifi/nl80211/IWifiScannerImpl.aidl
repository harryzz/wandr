// Trimmed by wandr — method ORDER preserved (transaction codes are positional:
// FIRST_CALL_TRANSACTION + declaration index), real signatures only for the calls
// we make (getScanResults=0, scan=3), `void reservedN()` placeholders for the rest.
// See the IDnsResolver trimming in build.rs for the same pattern. Task 90 M2.
package android.net.wifi.nl80211;

import android.net.wifi.nl80211.NativeScanResult;
import android.net.wifi.nl80211.SingleScanSettings;
import android.net.wifi.nl80211.IScanEvent;

interface IWifiScannerImpl {
    NativeScanResult[] getScanResults();                  // code 0
    void reserved1();                                     // code 1 (getPnoScanResults)
    void reserved2();                                     // code 2 (getMaxSsidsPerScan)
    boolean scan(in SingleScanSettings scanSettings);     // code 3
    void reserved4();                                     // code 4 (scanRequest)
    oneway void subscribeScanEvents(in IScanEvent handler); // code 5
    oneway void unsubscribeScanEvents();                  // code 6
}
