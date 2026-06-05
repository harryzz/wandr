---
name: project-artless-network
description: "ART-off networking: WiFi+internet CAN come up under --no-art using native survivors (netd/wificond/rild/HALs) — device-demonstrated (ping 8.8.8.8 0% loss, TCP 443 open). The Java loss = WifiService/ConnectivityService orchestration + NetworkStack DHCP."
metadata: 
  node_type: memory
  type: project
  originSessionId: e78b8106-7150-4010-98db-27565b460222
---

**LIVE-DEMONSTRATED 2026-06-05: full IPv4 internet works under `--no-art`** (Pixel 2 XL,
system_server stopped). Brought WiFi up by hand and got ping 8.8.8.8 = 0% loss + TCP 443
to 1.1.1.1 open. Connectivity is NOT torn down by stopping the framework — the native
stack survives; only the Java orchestration + DHCP are lost.

**What SURVIVES `--no-art` (all native binder services, verified alive):** `netd`
(`android.net.INetd` — routes/iptables/iface/DNS-resolver, the workhorse; `ndc` responds),
`wificond` (`android.net.wifi.nl80211.IWificond`), WiFi HAL (`android.hardware.wifi.IWifi/
default`), `rild` (modem), DnsResolver (libnetd_resolv, inside netd). Kernel netstate
persists (wlan0 keeps its IP + subnet route). **DIES with the framework:** `wpa_supplicant`
(service `disabled`+`oneshot`, started on-demand by WifiService → 802.11 assoc drops →
`wlan0` carrier=0/operstate=down, IP lingers but useless); NetworkStack APK (IpClient +
**DhcpClient** + captive-portal); WifiService + ConnectivityService + telephony-data (Java,
system_server). **No userspace DHCP client is shipped** (udhcpc/dhcpcd/dhcptool all MISSING;
toybox has no dhcp applet) — DHCP lived in NetworkStack/Java.

**HOW ART does WiFi (modern AIDL-HAL stack):** WifiService(system_server)→WifiNative→ IWifi
HAL (power chip, STA iface) + IWificond (scan) + start wpa_supplicant→ISupplicant AIDL
(WifiService pushes SSID/PSK from WifiConfigStore.xml → associate) → ConnectivityService→
NetworkStack.IpClient→DhcpClient (DHCP→IPv4) → netd (createNetwork/addRoute/setDefaultNetwork
+ DnsResolver.setResolverConfig). Cellular: telephony(system_server)→rild→rmnet→netd.

**WORKING `--no-art` bring-up recipe (device-proven, the manual version of a future wart-net):**
1. Start supplicant from the surviving vendor binary: `/vendor/bin/hw/wpa_supplicant -i wlan0
   -Dnl80211 -c <conf> -O /data/vendor/wifi/wpa/sockets`. nl80211/driver access works fine via
   `su` — **SELinux did NOT block it** (the early failures were a config-file perm bug, below).
2. Config from saved creds: PSK is **plaintext** in `/data/misc/apexdata/com.android.wifi/
   WifiConfigStore.xml` (`<string name="PreSharedKey">&quot;..&quot;</string>`, root-readable
   under --no-art; no keystore EncryptedData on this device). conf = ctrl_interface + a
   network{ ssid psk key_mgmt=WPA-PSK } block.
3. **Trigger association** — Android's wpa_supplicant LOADS the network but won't auto-connect;
   it waits for an external nudge (normally ISupplicant AIDL). Send `SELECT_NETWORK 0` +
   `REASSOCIATE` to the per-iface ctrl socket `/data/vendor/wifi/wpa/sockets/wlan0` →
   `CTRL-EVENT-CONNECTED`, carrier=1, 4-way handshake done. (No wpa_cli on device; used a
   ~40-line AF_UNIX SOCK_DGRAM client `wpanudge` at /data/local/tmp/wpanudge. Needs SO_RCVTIMEO
   — the reply can't be written back to a /data/local/tmp bind path, but the command still
   delivers; no AVC.)
4. IP: **reuse the still-valid DHCP lease** (kernel never flushed wlan0's IP) — short-term, no
   DHCP client needed. `ip route replace default via <gw> dev wlan0` → ping gw + 8.8.8.8 0% loss.

**GOTCHAS:** (a) wpa_supplicant drops to user `wifi` → the `-c` conf must be world-readable
(root `600` → `Failed to open config file: Permission denied`, the first dead-end). (b)
Android wpa needs the external assoc trigger (above). (c) `wificond` contention was a RED
HERRING — driver access was fine standalone. (d) **DNS is the one real gap**: `netd` survives
but `ndc network create / resolver setnetdns` FAILED (`createPhysicalNetwork() failed`,
`resolver` cmd unknown) — netd's per-netId resolver config needs the framework's caller perms
(ConnectivityService normally does it via INetd AIDL). IP path is fully up; only name
resolution is unwired. Tie-in: netd/CS permission checks hit the same `permission`/
IPermissionController service we stubbed for audio ([[project-artless-audio]]) — likely
relevant here too.

**NATIVE C++ WE KEEP (no reimplementation):** netd, DnsResolver, wificond, wpa_supplicant,
IWifi + ISupplicant HALs, rild. Same "drive the surviving HALs" pattern as wart-sensors.

**PROPOSED `wart-net` (mirrors wart-sensors; not built yet — scoping deferred to user):** an
rsbinder client driving `ISupplicant` AIDL to associate (replacing the wpanudge ctrl-socket
hack) + a small bundled **DHCP client** (the one genuinely-missing binary; or renew the lease)
+ `INetd` AIDL to createNetwork/addRoute and **set the DNS resolver** (the DNS piece, with
proper perms). Launched by run-hybrid-stack --no-art. See [[project_art_shutdown]] (KEEP/
REIMPLEMENT strategy), [[feedback_no_art_layer_dependencies]].

**Device state after the demo:** WiFi left UP (standalone wpa_supplicant pid running, route
added); plaintext-PSK conf removed; wpanudge left at /data/local/tmp. A run-hybrid-stack
--restore-art → --no-art cycle resets all of this.
