// Trimmed by wandr — method ORDER preserved (transaction codes), real signature
// only for getWifiScannerImpl=4; placeholders for the rest. Task 90 M2.
package android.net.wifi.nl80211;

import android.net.wifi.nl80211.IWifiScannerImpl;

interface IClientInterface {
    void reserved0();                                  // code 0 (getPacketCounters)
    void reserved1();                                  // code 1 (signalPoll)
    void reserved2();                                  // code 2 (getMacAddress)
    void reserved3();                                  // code 3 (getInterfaceName)
    @nullable IWifiScannerImpl getWifiScannerImpl();   // code 4
}
