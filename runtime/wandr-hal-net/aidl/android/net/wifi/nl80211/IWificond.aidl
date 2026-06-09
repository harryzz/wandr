// Trimmed by wandr — method ORDER preserved (transaction codes), real signatures
// for createClientInterface=1 (fallback creator) + GetClientInterfaces=5 (existing
// client ifaces); placeholders for the rest. The service registers as
// `wifinl80211` and survives `--no-art` (verified). Task 90 M2.
package android.net.wifi.nl80211;

import android.net.wifi.nl80211.IClientInterface;

interface IWificond {
    void reserved0();                                                              // code 0 (createApInterface)
    @nullable IClientInterface createClientInterface(@utf8InCpp String ifaceName); // code 1
    void reserved2();                                                              // code 2 (tearDownApInterface)
    void reserved3();                                                              // code 3 (tearDownClientInterface)
    void reserved4();                                                              // code 4 (tearDownInterfaces)
    List<IBinder> GetClientInterfaces();                                           // code 5
    void reserved6();                                                              // code 6 (GetApInterfaces)
    // Available channels as frequencies (MHz). WifiService builds its scan freq
    // list from these (WificondScannerImpl always scans an explicit channel set).
    @nullable int[] getAvailable2gChannels();                                      // code 7
    @nullable int[] getAvailable5gNonDFSChannels();                                // code 8
    @nullable int[] getAvailableDFSChannels();                                     // code 9
}
