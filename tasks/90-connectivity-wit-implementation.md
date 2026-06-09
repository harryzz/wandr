# Task 90 — implement the `wandr:connectivity` WIT (guest-facing connectivity)

> Status: 🟢 M1 DONE + DEVICE-VERIFIED (2026-06-09) — but redesigned: connectivity
> change-notification ships over a NEW GENERIC EVENT BUS (`wandr:events`), not the
> bespoke `wandr:connectivity/network` handler. The connectivity **subsystem**
> (task 88) + the `wifi`-management **contract** (`wit/connectivity.wit`) still
> stand; M2–M4 (wifi scan/connect/saved-networks/settings) are unchanged + pending.
>
> **Why the redesign:** rather than add a bespoke `connectivity-handler` export
> (yet another `*-events` world + bindgen + InboundEvent variant + drain arm), we
> built a single generic publish/subscribe bus so future event types (battery,
> screen, locale, theme…) cost ZERO new WIT/host wiring — just a topic string.
> Researched `wasi:messaging@0.2.0-draft` first: its shape matches, but `message`/
> `client` are **resources** + broker-oriented (Kafka/NATS) + no turnkey host crate,
> so we built `wandr:events` with the proposal's VOCABULARY (a `types.message`, a
> `producer` that publishes, an `incoming-handler.handle` push) using a plain
> **record** (no resources) — forward-compatible, fits the guest wit-parser.
>
> **M1 shipped + device-verified (Pixel 2 XL, --no-art):** `wit/events.wit` +
> generic `wandr-arbiter-events` broker (topic→subscribers + retained value;
> `evt-subscribe`/`evt-publish`/`evt-unsubscribe`; 3 unit tests) + host
> `events_host_impl` (`producer.publish` → `evt-publish`) + `events-incoming` export
> binding (drain → guest `handle`) + `package.toml [events] subscribe=[…]`
> host-config subscription + base64 line-wire. The `wandr-net` daemon publishes
> `net.status` (`online wifi <ssid> <ip>`/`offline`) on every change. Test guest
> `apps/user/wandr.connectivity.test` (dioxus, exports `incoming-handler`) renders
> live link state — verified Online↔Offline push + retained-on-subscribe. See
> `[[project_event_bus]]`, `[[project_artless_network]]`.
>
> **Original M1 plan (superseded by the bus):** wire `wandr:connectivity/network`
> `get-status` + `connectivity-handler` into wandr-host. `get-status` is replaced by
> the bus's retained-delivery-on-subscribe (no separate query); the change handler
> is replaced by `incoming-handler` on the `net.status` topic.

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
