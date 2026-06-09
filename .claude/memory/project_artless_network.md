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

**WORKING `--no-art` bring-up recipe (device-proven, the manual version of a future wandr-net):**
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
IWifi + ISupplicant HALs, rild. Same "drive the surviving HALs" pattern as wandr-sensors.

**PROPOSED `wandr-net` (mirrors wandr-sensors; not built yet — scoping deferred to user):** an
rsbinder client driving `ISupplicant` AIDL to associate (replacing the wpanudge ctrl-socket
hack) + a small bundled **DHCP client** (the one genuinely-missing binary; or renew the lease)
+ `INetd` AIDL to createNetwork/addRoute and **set the DNS resolver** (the DNS piece, with
proper perms). Launched by run-hybrid-stack --no-art. See [[project_art_shutdown]] (KEEP/
REIMPLEMENT strategy), [[feedback_no_art_layer_dependencies]].

**Device state after the demo:** WiFi left UP (standalone wpa_supplicant pid running, route
added); plaintext-PSK conf removed; wpanudge left at /data/local/tmp. A run-hybrid-stack
--restore-art → --no-art cycle resets all of this.

**TASK 88 M1 PRODUCTIZATION (2026-06-05, code-complete, pending --no-art device verify):**
Built the wandr triad mirroring sensors/audio. Decisions: ctrl-socket associate NOW (ISupplicant
AIDL later), **pure-Rust DHCPv4 up front** (no lease-reuse stopgap), thinnest slice.
- `runtime/wandr-hal-net` (pure-Rust, NO binder in M1): `dhcp.rs` DHCPv4 DISCOVER/OFFER/REQUEST/ACK
  (UDP 0.0.0.0:68 + SO_BINDTODEVICE + broadcast; unit-tested parse) + `supplicant.rs` (write
  world-readable conf, spawn `/vendor/bin/hw/wpa_supplicant -i wlan0 -Dnl80211 -c <conf> -O
  /data/vendor/wifi/wpa/sockets`, AF_UNIX SOCK_DGRAM ctrl SELECT_NETWORK 0/REASSOCIATE/STATUS) +
  `lib.rs` (WifiConfigStore.xml cred parse, `ip addr/route` applier). 6 tests pass.
- `runtime/wandr-net` daemon: associate→DHCP→apply→DNS→`report-net-state`; `--once` investigation
  mode; respawn-supervised via run-hybrid-stack --no-art (runs as ROOT via su — nl80211/route/port68).
- `runtime/wandr-arbiter/wandr-arbiter-net` module: `report-net-state`/`net-status`/`net-subscribe`
  verbs + `on-connectivity-change` fan-out (Effect::HostLine `net-changed …`). Wired 1-line. 5 tests.
- `wit/connectivity.wit` (wandr:connectivity): get-status import + on-connectivity-change export.
  Host bindgen wiring + test guest = M1b (NOT wired into wandr-host yet).
KEY CAVEATS for device verify: (a) **chip must be POWERED first** — WiFi ON under ART before
--no-art (IWifi HAL powers chip + creates STA iface; cold power-up = M2). Device recon: wlan0 exists
but DOWN when wifi off. (b) **DNS still the open gap** — `dns.rs` tries resolv.conf/net.dns*/ndc
best-effort; the device run decides what makes guest getaddrinfo resolve (host inherit_network →
bionic → netd dnsproxyd). Fallback if none work: vendor `android.net.INetd`+`android.net.IDnsResolver`
(NOT vendored — only the OEM-subset `android.system.net.netd.INetd` is) + call as privileged uid +
create a netId + set default network. ISupplicant AIDL tree IS already vendored under wandr-host/vendor.

**M1 DEVICE-VERIFIED 2026-06-05 (Pixel 2 XL, live --no-art session):** wandr-net auto-brought-up
WiFi end-to-end: associate zah004 → pure-Rust DHCP lease 192.168.1.179/22 gw .1.1 → applied →
**ping 8.8.8.8 3/3 0% loss**. TWO on-device fixes baked into wandr-hal-net: (1) leaked supplicants
(std Child drop does NOT kill child!) + stale ctrl socket → `cleanup_stale` (pkill wpa_supplicant +
rm socket) + connect-retry-on-ECONNREFUSED with PING/PONG probe. (2) **THE ROUTING GOTCHA**: a
default route in `main` is IGNORED — Android's `ip rule`s (set by the dead ConnectivityService) send
all unmarked traffic through fwmark per-network tables (16000+) then `32000: unreachable`, never
consulting `main`. FIX = `ip rule add pref 15000 from all lookup main` + add default route `onlink`
(the gw-reachability check at insert honors the rules, which don't yet see the connected route).
This is why the demo's `ip route replace default` "worked" only with a particular pre-existing rule
state. **DNS DEFINITIVELY DIAGNOSED = M1b:** IP works but getaddrinfo fails. A15 bionic ALWAYS uses
netd dnsproxyd — `ANDROID_DNS_MODE=local` is GONE (tested, dead), /etc/resolv.conf is read-only
/system, net.dns* props ignored. netd has no resolver bc `ndc network create`/createPhysicalNetwork
is PERMISSION-REJECTED EVEN AS ROOT (400 failed — netd's IPermissionController check fails --no-art).
M1b DNS paths: (a) task-87 permission stub → INetd networkCreatePhysical+networkSetDefault +
IDnsResolver.setResolverConfiguration as privileged caller; (b) custom UDP resolver in wandr-host
wasi:sockets/ip-name-lookup (bypass bionic/netd). Also: `from all lookup main` rule lingers after
restore-art (could interfere w/ framework routing) — clean it on teardown (TODO).

**DNS SOLVED + PRODUCTIZED 2026-06-05 (netd over binder as uid SYSTEM):** the demo + my first
attempts failed bc run as ROOT — **netd AND dnsresolver special-case AID_SYSTEM (uid 1000), not
root** (checkAnyPermission special-cases AID_SYSTEM). As `su 1000`: ndc network create + the INetd
binder calls + IDnsResolver.setResolverConfiguration ALL succeed. Productized in wandr-net:
- root (link): associate + DHCP + `ip addr add` (address only).
- re-exec `su 1000 wandr-net --netd-config`: drive over binder INetd networkCreatePhysical(netId,
  PERMISSION_NONE=0)/networkAddInterface/networkAddRoute(connected subnet THEN default via gw)/
  networkSetDefault + IDnsResolver createNetworkCache + setResolverConfiguration(ResolverParamsParcel
  {netId,servers,interfaceNames=[wlan0],transportTypes=[WIFI]}).
- root: ONE catch-all `ip rule add pref 15000 from all lookup <iface>` — netd routes per-netId by
  fwmark (CS assigns per-UID, dead here) so unmarked traffic (fwmark 0) → 32000 unreachable; this
  points it at netd's per-net table (named after the iface). Single-network bypass.
GOTCHAS: (1) networkAddRoute default fails "Network is unreachable" unless the CONNECTED subnet route
is added to the per-net table FIRST (we set the addr via ip out-of-band, so networkAddInterface
doesn't seed it). (2) rsbinder-aidl: full real android.net.INetd.aidl parses+generates+COMPILES in
ASYNC mode as-is (sync codegen broken — BnX never emitted); IDnsResolver needs a TRIMMED copy
(methods 0-7, positional transaction codes preserved) bc its listener-callback methods break async
dyn-compat. Vendored submodules: aosp-packages-modules-connectivity (INetd) +
aosp-packages-modules-dnsresolver (IDnsResolver). Device: daemon auto → ping 8.8.8.8 0% + curl
http://example.com 200 + https://codeberg.org 200 (DNS+TLS). PROCESS NOTE: I patch-and-cycled this
on-device instead of reading netd's permission/routing source first (violated [[feedback_read_source_first]]).
