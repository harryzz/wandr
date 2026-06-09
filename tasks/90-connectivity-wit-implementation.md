# Task 90 — implement the `wandr:connectivity` WIT (guest-facing connectivity)

> Status: 🔲 SCOPED. Spun out of task 88 so we can return to task 87 (ART-off
> audio bug fixes) first. The connectivity **subsystem** is built + device-verified
> end to end under `--no-art` (task 88: `IWifi` power-up → `ISupplicant` associate
> → pure-Rust DHCP → `INetd` route → `IDnsResolver` DNS, all over binder as uid
> system). The **guest-facing WIT contract** is drafted + validated
> (`wit/connectivity.wit`, commit `6a36f2b1`) but **not wired into wandr-host** — no
> guest can use it yet. This task does that wiring + implements the `wifi`
> management interface. See `[[project_artless_network]]`.

## What exists (don't rebuild)

- **`wit/connectivity.wit`** — the contract (passes `wasm-tools component wit`):
  - `network` (unprivileged): `get-status` + `connectivity-handler.on-connectivity-change`.
  - `wifi` (privileged): radio (`set-enabled`/`is-enabled`), `scan`, saved-network
    CRUD (`list-saved`/`add-network`/`update-network`/`remove-network`/
    `set-auto-connect`), connection (`connect`/`connect-new`/`disconnect`/
    `forget-current`); types `wifi-config`, `ip-config = dhcp | manual(static-ip)`,
    `security-kind`, `scan-result`, `saved-network`.
  - Worlds: `connectivity-host`/`connectivity-events` (status), `wifi-host`/
    `wifi-settings` (management).
- **`runtime/wandr-arbiter/wandr-arbiter-net`** — status of record (`net-status`)
  + `on-connectivity-change` fan-out (`net-changed`, via `Effect::HostLine`);
  verbs `report-net-state` / `net-status` / `net-subscribe`. Unit-tested, wired.
- **`runtime/wandr-net` + `runtime/wandr-hal-net`** — the daemon + HAL binder
  clients already drive `IWifi`/`ISupplicant`/`INetd`/`IDnsResolver` (uid system).
- **`runtime/wandr-host`** — guests already reach the network via `wasi:sockets`
  (`inherit_network`); this task adds the *control* WIT, not a data path.

## The pattern to mirror

`wandr:alarm` is the canonical arbiter-facing WIT host path — copy it:
- `runtime/wandr-host/src/lib.rs` — `bindgen!` the `*-host` + `*-events` worlds.
- `runtime/wandr-host/src/alarm_host_impl.rs` — `impl Host` forwarding to the
  arbiter socket (`crate::arbiter_sock`); `runtime/wandr-host/src/app_loader.rs`
  `add_to_linker`; the standalone loop delivers the arbiter's push to the guest
  export. (`docs/architecture-host-guest-boundary.md`.)

## Scope (phased)

### M1 — `network` status wiring (small; backing is fully built)
The differentiated product surface (`registerDefaultNetworkCallback`):
1. `wandr-host`: `bindgen!` `connectivity-host` + `connectivity-events`; new
   `connectivity_host_impl.rs` — `get-status` → arbiter `net-status`, parse the
   reply into `network-status`.
2. `add_to_linker` `network` onto every guest's linker (status is unprivileged).
3. The arbiter already fans `net-changed <state>` to subscribed hosts → deliver
   it to the guest's `on-connectivity-change` in the standalone loop (mirror
   `alarm-fired`). Register the guest as a subscriber (`net-subscribe <pid>`) when
   it exports the handler.
4. A tiny test guest (or wire an existing one) that logs online↔offline.
DoD: a guest's `on-connectivity-change` fires when wandr-net flips the link.

### M2 — `wifi` scan + connect (privileged)
1. **`scan`** — bind **`IWificond`** (`android.net.wifi.nl80211.IWificond`,
   service `wifinl80211`, survives `--no-art`) in `wandr-hal-net` (same rsbinder
   pattern as `IWifi`/`ISupplicant`); trigger a scan + return `scan-result`s.
   New daemon verb / `wandr-net --scan`; arbiter relays.
2. **`connect`/`connect-new`** — generalize the built `supplicant_hal::associate`
   to take an arbitrary `wifi-config` (not just the one saved cred), driven from
   the WIT through the arbiter → daemon.
3. **`set-enabled`** — `wifi_chip::ensure_chip_up` / `stop_chip` (built).
4. Host: `bindgen!` `wifi-host`; `connectivity_wifi_impl.rs` forwarding to the
   arbiter; **privilege-gate** `add_to_linker` for `wifi` (trusted guests only —
   the Settings/wifi-picker chrome; see Open questions).

### M3 — saved-network store + auto-connect
1. **Credential storage decision** (Open question below) — a wandr-owned store vs
   the framework's `WifiConfigStore.xml`. Build the chosen store.
2. `wandr-arbiter-net` becomes the **WifiConfigManager**: saved-network registry,
   `auto-connect` policy, selection/failover (pick which known net to join).
   `list-saved`/`add`/`remove`/`update`/`set-auto-connect` land here.
3. `wandr-net` auto-joins a known network at bring-up from the store (replacing the
   single-`WifiConfigStore.xml`-entry read it does today).

### M4 — `ip-config` manual (static IP) + a Settings/wifi-picker chrome app
- `manual(static-ip)`: `apply_address` + `configure_netd` already take the same
  fields; thread a static config from the WIT instead of the DHCP lease.
- A `wandr.settings.wifi` guest (separate wandrpkg) consuming `wifi-settings`.

## Open questions (decide before M3)
1. **Credential storage** (`--no-art`): wandr-owned encrypted store, vs reuse
   `keystore2` (survives), vs read/write the framework `WifiConfigStore.xml`.
   Today the daemon reads ONE plaintext PSK entry from `WifiConfigStore.xml`.
2. **Privilege model for `wifi`**: how the host decides a guest is "the Settings
   app" (manifest capability flag? a `package.toml` field? app-id allowlist?).
   `network` is open to all; `wifi` must be gated.
3. **Scan ownership under `--no-art`**: confirm `IWificond` scan works as uid
   system without WifiService (it's registered + survives — verify like the
   `IWifi`/`ISupplicant` probes).

## References
- `wit/connectivity.wit` (the contract) + commit `6a36f2b1`.
- `tasks/88-connectivity-service.md` (the subsystem: M1 + M2, what's built).
- `[[project_artless_network]]` (the binder map, uid-system finding, recipe).
- `wandr:alarm` host wiring (the pattern) + `docs/architecture-host-guest-boundary.md`.
- `[[feedback_read_source_first]]` — read the AIDL (`IWificond`) + the `wandr:alarm`
  host path end-to-end before patching.
