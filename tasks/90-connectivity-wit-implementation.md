# Task 90 — implement the `wandr:connectivity` WIT (guest-facing connectivity)

> Status: 🟢 M1+M2+M3 DONE + DEVICE-VERIFIED (M3 2026-06-09). M1 connectivity
> change-notification ships over the GENERIC EVENT BUS (`wandr:events`). M2 = the
> privileged wifi control plane (scan/connect/radio via host→arbiter→wandr-net.sock
> + privilege gate). M3 = the WifiConfigManager (wandr-owned 0600 JSON saved-network
> store + auto-connect, in `wandr-arbiter-net`; `wandr-net` auto-joins from it).
> **M4 pending:** the `wandr.settings.wifi` picker app + manual static-IP.
> Follow-up: a daemon supplicant-disconnect verb to un-stub `disconnect`/
> `forget-current`.
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

### M2 — `wifi` scan + connect (privileged) — 🟢 DONE (host half wired)
1. **`scan`** — ✅ bound **`IWificond`** (`android.net.wifi.nl80211.IWificond`,
   `wifinl80211`, survives `--no-art`) in `wandr-hal-net::wificond_scan`; engine
   verb `wandr-net --scan` returns 14 neighbour APs (IScanEvent callback fires).
   Key gotcha: the parcelable arg needs a leading `int32(1)` presence marker
   (else `EX_NULL_POINTER`). Device-verified.
2. **`connect`/`connect-new`** — ✅ `wandr-net --connect <ssid> <psk>` generalizes
   `supplicant_hal::associate` + full IP bring-up (force re-associate).
3. **`set-enabled`** — ✅ `--power-chip`/`--stop-chip` (IWifi HAL, uid system).
4. **Host half (FULL, the chosen scope)** — ✅ wired:
   - **`wandr-net` control socket** (`/data/local/tmp/wandr-net.sock`, env
     `WANDR_NET_SOCK`): the single live daemon serves `scan` / `connect <b64ssid>
     <b64psk>` / `set-enabled <0|1>` / `is-enabled` (so connect goes *through* the
     supplicant owner, not a competing process). SSID/PSK base64'd on the wire.
   - **Arbiter relay** (`wandr-arbiter-bin`): `wifi-scan` / `wifi-connect` /
     `wifi-set-enabled` / `wifi-is-enabled` verbs pass through to the daemon socket
     and pipe the reply back (CLI test forms + the host's raw-socket path). The
     arbiter is the coordinator so M3's WifiConfigManager can intercept `connect`.
   - **Host WIT** (`wandr-host`): `bindgen!` `wifi-host` + `connectivity_wifi_impl.rs`
     (forwards `scan`/`connect-new`/`set-enabled`/`is-enabled` to the arbiter;
     `list-saved`/`add`/`update`/`remove`/`set-auto-connect`/`connect(id)`/
     `disconnect`/`forget-current` return an explicit "M3" error / no-op).
   - **Privilege gate** (`LoadedApp::wifi_privileged`): `wifi` is `add_to_linker`d
     ONLY for a guest that is BOTH system-install class (`system-apps/`) AND opts
     in via `wifi-control = true` in `package.toml` (least-privilege; no hardcoded
     app-id list). A non-privileged guest importing `wifi` fails to instantiate —
     that *is* the denial.
   - Guest consumer = M4 (`wandr.settings.wifi`). Until then the relay path is
     device-verifiable via `wandr-arbiter wifi-scan` / `wifi-connect …`.

### M3 — saved-network store + auto-connect — 🟢 DONE + device-verified
1. **Credential storage** — ✅ DECIDED: **wandr-owned JSON store**
   (`/data/local/tmp/wandr-wifi-networks.json`, root-only **0600**; `ssid`/`psk`
   base64'd so values are JSON-safe + the hand-rolled no-serde writer is trivially
   correct). keystore2-wrapping is a noted hardening follow-up (2026-06-09).
2. **`wandr-arbiter-net` = the WifiConfigManager** — ✅ verbs `wifi-saved-list` /
   `wifi-saved-add` / `wifi-saved-update` / `wifi-saved-remove` /
   `wifi-saved-auto-connect` / `wifi-saved-creds` / `wifi-auto-network`. Pure module
   (registry + persistence via an injected `store_path`; `None` = in-memory for
   tests); monotonic ids; add keys a net by (ssid, security) so a re-add updates.
   6 unit tests (CRUD, dup-update, creds, auto-network, JSON round-trip incl.
   special chars). The bin registers `NetModule::with_store(WIFI_STORE_PATH)` and
   adds a `wifi-connect-saved <id>` handler (module resolves id→creds, bin relays
   `connect` to the daemon — the orchestration step).
3. **`wandr-net` auto-joins from the store** — ✅ `bring_up` now queries the arbiter
   `wifi-auto-network` first (the wandr store), falling back to
   `WifiConfigStore.xml` when the store has no auto-connect net or the arbiter is
   unreachable (boot ordering).
4. **Host** — ✅ un-stubbed `list-saved`/`add-network`/`update-network`/
   `remove-network`/`set-auto-connect`/`connect(id)` forwarding to the arbiter.
   `disconnect`/`forget-current` remain stubbed (need a daemon supplicant-disconnect
   verb — a small follow-up; `set-enabled false` is too blunt).

   **Device-verified (CLI, `--no-art`):** add → `id=1`/`id=2`; `wifi-saved-list`
   shows both with b64 ssid + flags; `wifi-auto-network` returns the auto net's
   creds; toggle moves the auto selection; `wifi-saved-creds`/`update`/`remove`
   correct (`not-found` on a bad id); JSON persisted **0600**; a PSK with
   space/comma/quote round-trips through b64. Live `connect(id)` + auto-join restart
   were skipped (they trigger associations) — both ride M2-verified relay/engine
   paths and the resolve/read halves are CLI-verified.

### M4 — `ip-config` manual (static IP) + a Settings/wifi-picker chrome app
- `manual(static-ip)`: `apply_address` + `configure_netd` already take the same
  fields; thread a static config from the WIT instead of the DHCP lease.
- A `wandr.settings.wifi` guest (separate wandrpkg) consuming `wifi-settings`.

## Open questions
1. ✅ RESOLVED (2026-06-09): **Credential storage** = a **wandr-owned JSON store**
   (root-only 0600, ssid/psk base64'd). Chosen over keystore2-wrapping (needs
   keystore2 AIDL + key mgmt, and "keystore from su domain" is unproven `--no-art`)
   and over read/writing the framework `WifiConfigStore.xml` (fragile internal
   schema, framework not managing it under `--no-art`). No worse than the device's
   already-plaintext `WifiConfigStore.xml`; keystore2-wrap = a hardening follow-up.
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
