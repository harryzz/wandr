# Task 88 — wart connectivity service (ART-off networking, productized)

> Status: 🚧 M1 — full IP **+ DNS** DEVICE-VERIFIED + productized (`--no-art`).
> Follow-on to the `--no-art` networking analysis + live demo in
> `[[project_artless_network]]` (WiFi+internet brought up by hand: ping 8.8.8.8
> 0% loss, TCP 443 open). This task turns that one-off recipe into a first-class
> wart subsystem. Binder analysis: `[[project_artless_network]]` + the session
> that found `connectivity`/`wifi` are dead system_server binders while
> `netd`/`dnsresolver`/`IWifi`/`ISupplicant` survive natively.

## M1 implementation status (2026-06-05)

Thinnest end-to-end slice (decisions: ctrl-socket associate now / ISupplicant
AIDL later; **pure-Rust DHCPv4 up front**; DNS mechanism = on-device
investigation). Built + builds clean (desktop tests + aarch64-android cross):

- **`runtime/wart-hal-net`** (pure-Rust, no binder in M1) — `dhcp.rs` (DHCPv4
  DISCOVER/OFFER/REQUEST/ACK, the one missing native binary; unit-tested parse),
  `supplicant.rs` (write conf + spawn vendor `wpa_supplicant` + ctrl-socket
  `SELECT_NETWORK`/`REASSOCIATE` nudge), `lib.rs` (WifiConfigStore.xml cred
  parse + `ip`-based addr/route applier). 6 unit tests pass.
- **`runtime/wart-net`** (daemon) — orchestrates associate→DHCP→apply→DNS, reports
  `report-net-state` to the arbiter, monitors carrier + re-associates. `--once`
  mode = on-device bring-up/DNS investigation harness. `dns.rs` tries
  resolv.conf / `net.dns*` props / `ndc resolver setnetdns` (best-effort, logged).
- **`runtime/wart-arbiter/wart-arbiter-net`** (module) — status of record
  (`report-net-state` / `net-status`) + `on-connectivity-change` fan-out to
  subscribed guests (`net-subscribe`). 5 unit tests pass; wired into the arbiter
  binary (one line) + cross-compiles clean.
- **`wit/connectivity.wit`** (`war:connectivity`) — `get-status` import +
  `on-connectivity-change` export contract. **Host bindgen/linker wiring + a
  guest that exports the handler = the immediate next step (M1b)**; not yet wired
  into wart-host this session.
- **`tools/scripts/run-hybrid-stack.sh`** — pushes + launches `wart-net` under
  `--no-art` (respawn-supervised, as root via `su`) next to `wart-sensors`;
  teardown/restore paths stop it so WifiService can reclaim `wlan0`.

### Device verification (2026-06-05, Pixel 2 XL, live `--no-art` session)

✅ **IP path fully verified, end-to-end automated.** `wart-net` (run as root)
brought WiFi up with zero manual steps: associate to `zah004` via the ctrl-socket
nudge → **pure-Rust DHCPv4 lease** (`192.168.1.179/22` gw `192.168.1.1` dns
`192.168.1.1`, 600s) → applied addr + `default via … onlink` + the
`lookup main` rule → **`ping 8.8.8.8` 3/3 0% loss (~18 ms)**.

Two fixes found on-device (now in the code): (a) leaked supplicants + stale ctrl
socket → `cleanup_stale` (kill `wpa_supplicant` + rm socket before spawn) +
connect-retry-on-`ECONNREFUSED` with a `PING`/`PONG` probe; (b) **policy
routing** — Android's `ip rule`s (set by the dead ConnectivityService) never
consult `main`, so a default route there is ignored (all traffic → `32000:
unreachable`); fixed by installing `ip rule pref 15000 from all lookup main` +
adding the default route `onlink` (gateway-reachability check honors the
not-yet-consulted rules). `MAIN_LOOKUP_RULE_PREF` is the one named constant.

✅ **DNS SOLVED — netd over binder as uid `system`** (the demo's one open gap).
Root cause of the demo's failure: on Android 15 bionic *always* routes name
lookups to netd's `dnsproxyd` (the in-process `ANDROID_DNS_MODE=local` resolver
is gone, `/etc/resolv.conf` is read-only `/system`, `net.dns*` ignored), and netd
has no resolver because `ndc`/`networkCreate` were run **as root** — but **netd +
dnsresolver special-case `AID_SYSTEM` (uid 1000), not root**. Driving them as uid
system works: `ndc network create` and the binder calls all succeed. So:
- Vendored `android.net.INetd` (`packages/modules/Connectivity`) +
  `android.net.IDnsResolver` (`packages/modules/DnsResolver`) AIDLs as submodules.
- `wart-hal-net` codegens both via rsbinder (the full real INetd parses/compiles
  as-is in async mode; IDnsResolver needs a trimmed copy — its listener callbacks
  break codegen).
- `wart-net` (root: link/DHCP/address) re-execs `--netd-config` under `su 1000`
  to drive, over binder: **INetd** `networkCreatePhysical`/`networkAddInterface`/
  `networkAddRoute` (connected + default)/`networkSetDefault`, then **IDnsResolver**
  `createNetworkCache` + `setResolverConfiguration`. One catch-all `ip rule from
  all lookup <iface>` (root) bridges netd's per-UID fwmark routing (CS-managed,
  dead here) for the single-network case.
- Device-verified end to end: daemon auto-brings-up → `ping 8.8.8.8` 0% loss,
  `http://example.com` HTTP 200, **`https://codeberg.org` HTTP 200** (DNS+TLS).

### Remaining for M1 done
1. **`war:connectivity` host wiring** — bindgen `connectivity-host`/`-events`,
   forward `get-status` to arbiter `net-status`, deliver `net-changed` → guest
   `on-connectivity-change`; a test guest. (Arbiter `wart-arbiter-net` module is
   built + unit-tested; the device's *running* arbiter is still the old binary —
   integration verifies on the next full `run-hybrid-stack --no-art`.)
2. **Cold chip power-up** (M2 proper, but blocks a clean-boot M1): the verify
   reused a powered chip (WiFi was on under ART before `--no-art`). A cold
   `--no-art` boot needs the IWifi HAL to power the chip + create the STA iface
   first (today done by the dead WifiService).

## M2 — supplicant over binder (STARTED — reconnaissance ✅)

Replace the ctrl-socket nudge with the Android-native path: drive
`android.hardware.wifi.supplicant.ISupplicant` over binder (what `WifiNative`
uses). Reconnaissance done + device-verified under `--no-art`:

- **rsbinder codegen**: the full real `ISupplicant.aidl` (134 files) parses +
  generates (26457 lines) + **compiles** as-is in async mode — only needs the
  framework-type stubs already in `vendor/aidl-stubs` (`PersistableBundle`,
  `ParcelFileDescriptor`). No trimming, unlike `IDnsResolver`.
- **Runtime reachability (the real unknown) = GREEN**: as uid `system` a process
  `get_interface`s `…ISupplicant/default` over the *default* `/dev/binder` +
  servicemanager (it's registered there, like `netd` — visible in `service list`
  under `--no-art`); **no SELinux/AVC denial**; `listInterfaces()` returns `wlan0`
  (STA). `IWifi/default` is likewise registered (the cold chip power-up path).
- Vendored AIDL is already present (`aosp-hardware-interfaces/wifi/supplicant`).
- Scaffolding landed: `wart-hal-net` codegens ISupplicant + `supplicant_hal::probe`;
  `wart-net --probe-supplicant` (run as `su 1000`).
- Nuance (not a blocker): `getStaInterface("wlan0")` → `ServiceSpecific(1)` because
  our supplicant created the iface via legacy `-i/-c` config, not the HAL's
  `addStaInterface()`; M2 either calls `addStaInterface` or starts it HAL-managed.

M2 remaining: drive associate via `ISupplicantStaIface` (`addNetwork` →
`setSsid`/`setPsk`/`setKeyMgmt` → `reassociate`) + cold chip power-up via `IWifi`.

## Goal

A wart-native connectivity layer that replaces Android's `ConnectivityService` +
`WifiService` + per-transport managers (all Java, in `system_server`, dead under
`--no-art`) by driving the **surviving native binders** — `ISupplicant`,
`INetd`, `IDnsResolver`, `IWifi`, `wificond`, `rild`. It must:

1. **Enumerate available connection methods** — wifi-STA, ethernet/LAN, cellular,
   bluetooth-PAN, wifi-AP(hotspot), VPN — and which are present/enabled.
2. **Set up / tear down** each (a common per-transport interface).
3. **Monitor** each link (state, IP, signal, metered, validated-internet).
4. **Pick the active/default network** (policy + failover) and apply the kernel
   default route + DNS.
5. **Expose status + control + change-events to guests** via WIT (guests still
   move data over WASI sockets → kernel directly; this layer is control/monitor,
   NOT a data path — see "Why not a data path" below).

## Architecture (the wart triad, mirrors audio/sensors)

```
guest (WASI sockets = data path)            guest (war:connectivity WIT = control/status)
        │                                            │
        ▼                                            ▼
   kernel ──────── netd (INetd, routes/iptables) ── wart-arbiter-net  ← DECIDES (policy brain)
                   dnsresolver (IDnsResolver)              │  transport registry, default-net
                   (native survivors)                      │  selection, validation, metered
                                                           ▼
                                              wart-hal-net  ← APPLIES (drives survivors)
                                              ISupplicant / INetd / IDnsResolver /
                                              IWifi / wificond / rild  +  DHCP client
```

- **`runtime/wart-hal-net`** (shared HAL crate, like `wart-hal-sensors`): thin
  rsbinder clients to the native survivors + a bundled **DHCPv4 client** (the one
  genuinely-missing native binary — Android did DHCP in NetworkStack/Java). The
  per-transport "drivers" live here (associate, get-ip, set-route, set-dns).
- **`runtime/wart-arbiter/wart-arbiter-net`** (arbiter module, the
  ConnectivityService brain): transport registry + availability, network-selection
  policy, default-network/route/DNS orchestration, reachability validation,
  metered/cost policy, persistence/auto-reconnect, status aggregation. Decides;
  HAL applies (same "arbiter decides, host/HAL applies" contract as audio).
- **`wit/war-connectivity.wit`** + guest bindings (`war:connectivity`): status,
  enumerate, connect/disconnect (privileged), and an `on-change` export callback.
- Launched by `run-hybrid-stack --no-art` (after netd/wifi HAL are up), like
  `wart-sensors`. May reuse the `permission` stub from task 87 (netd's privileged
  ops — createNetwork/setResolver — do an `IPermissionController` check).

## The per-transport interface (setup + monitor)

A common `Transport` abstraction, implemented per medium:

| Method | wifi-STA | ethernet | cellular | bt-PAN |
|---|---|---|---|---|
| `availability()` | chip present, enabled | iface present (dock) | SIM present | paired PAN dev |
| `scan()/enumerate()` | SSIDs + RSSI + security | link up? | operators/APN | devices |
| `connect(params)` | ssid/psk/eap → ISupplicant | (auto/DHCP) | APN → rild data call | bnep |
| `disconnect()` | ✓ | ✓ | ✓ | ✓ |
| `status()` | state, bssid, rssi, link-speed, ip, gw, dns, ipv6, metered, validated | link, ip… | signal, ip, metered=true | ip |
| events | up/down, ip-change, signal, captive, validated | up/down | up/down, signal | up/down |

`state` enum: `disconnected → scanning → associating → obtaining-ip → connected →
{validated | captive-portal | no-internet} | failed`.

## Orchestrator (`wart-arbiter-net`) responsibilities

- **Transport registry + availability** = "enumerate available connection methods".
- **Network-selection policy**: priority (ethernet > wifi > cellular default; or
  explicit/user pin), auto-connect to known networks, **failover** on loss.
- **Default-network management**: set kernel default route + DNS for the chosen
  net (via `INetd`/`IDnsResolver`); switch atomically on failover.
- **Reachability validation**: HTTP-204 captive-portal probe → mark
  validated / captive / no-internet (connected ≠ internet).
- **DNS** via `IDnsResolver` (the concrete gap left by the demo — `ndc` failed on
  netd's permission model; the AIDL called as a privileged uid is the right path).
- **Metered / data-saver** awareness (cellular = metered).
- **Persistence + auto-reconnect** under `--no-art` (remember known nets + creds;
  reconnect at bring-up — replaces WifiConfigManager + framework auto-connect).
- **Status aggregation** → guests (`on-change`) + the wart status bar (wifi/signal
  icon, like the existing chrome).

## WIT — `war:connectivity` (control/status/monitor, NOT data)

- `get-status() -> network-status` — online?, active-transport, metered, captive
- `list-transports() -> list<transport-info>` — kind + availability + status
- `scan-wifi() -> list<wifi-network>` (privileged)
- `connect(kind, params) / disconnect(kind)` (privileged)
- `request-transport(kind)` — "I need cellular" (wake radio) [later]
- **export** `on-connectivity-change()` — guest notified online↔offline (the
  single most-used ConnectivityManager feature — Signal/apps reconnect on it).

## Why not a data path (key design point)

`ConnectivityService`'s `connectivity`/`IConnectivityManager` binder is the
**app-facing control/policy API**, not the packet path — packets go kernel↔netd.
wart guests use **WASI sockets → kernel directly**, so they never touch
`IConnectivityManager`; the live demo proved a plain default route gives full
internet with no ConnectivityService at all. So we do **not** stub/reimplement
the connectivity binder — we drive the native survivors and expose our own thin
WIT. (Android's per-UID fwmark/permission routing is CS policy we deliberately
bypass with a single default route in the main table.)

## Suggested phasing

- **M1 — WiFi-STA end to end** (productize the demo): `wart-hal-net` ISupplicant
  associate + the bundled DHCP client + `INetd`/`IDnsResolver` route+DNS;
  `wart-arbiter-net` minimal (one transport, connect/status/`on-change`);
  captive-portal validation; `run-hybrid-stack --no-art` auto-connects a known net.
  DoD: phone boots `--no-art`, auto-joins wifi, DNS resolves, a guest gets online.
- **M2 — multi-transport + policy**: ethernet (USB-C dock), selection priority +
  failover, persistence/auto-reconnect, status-bar wifi icon, metered scaffold.
- **M3 — cellular** (`rild` data call + APN + metered) — bigger lift (telephony).
- **M4 — later**: bluetooth-PAN, SoftAP/hotspot, VPN, per-transport QoS metrics.

## Other suggestions / open questions (for discussion)

1. **Change notifications are the priority feature**, not connect UX — apps mostly
   just need online/offline + metered (`registerDefaultNetworkCallback`).
2. **Airplane mode / radio power** control (battery; ties to `wart-arbiter-power`).
3. **Keep-wifi-alive during background work / screen-off** (mirror the audio comms
   keep-alive in `wart-arbiter-power`; otherwise idle may drop the link).
4. **Security/permissions**: status = any guest; scan/connect = privileged.
5. **Credential storage**: where do known-network creds live ART-off? (keystore2
   survives; or a wart-owned store). Today: plaintext in `WifiConfigStore.xml`.
6. **IPv6** is basically free (already saw global v6 addrs) — include in `status()`.
7. **A wifi-picker chrome app** (war.settings.wifi) for first-time connect UX —
   separate guest, later.
8. **DHCP client choice**: bundle a static `udhcpc`/`dhcpcd`, or a small pure-Rust
   DHCPv4 in `wart-hal-net` (no extra binary, fits the WASI/rust-everywhere ethos).

## Risks / unknowns

- DHCP client is net-new (the one missing piece). Pure-Rust DHCPv4 is small/clean.
- `ISupplicant` AIDL is a sizable interface to bind via rsbinder (like other HALs).
- Cellular/BT pull in whole stacks (telephony / bluetooth process) — deferred.
- netd privileged-op permission checks (the `permission`/`IPermissionController`
  tie-in from task 87) — confirm whether the stub or a privileged uid is needed.
- The supplicant won't auto-connect standalone (needs the ISupplicant trigger) —
  `wart-hal-net` must drive `selectNetwork`/`reassociate` (demo used a ctrl-socket
  `wpanudge`; the AIDL call replaces it).

## References

- `[[project_artless_network]]` — survivors/dead list, the working bring-up recipe,
  the binder map (`connectivity`/`wifi` dead; `netd`/`dnsresolver`/HALs alive).
- `[[project_art_shutdown]]`, `[[feedback_no_art_layer_dependencies]]` — the
  KEEP-native-survivors strategy.
- `[[project_arbiter_audio]]` / `[[project_arbiter_sensors]]` — the
  arbiter-module + HAL-crate pattern to mirror.
- `[[project-artless-audio]]` — the `permission` (`IPermissionController`) stub.
