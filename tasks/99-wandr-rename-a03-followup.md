# Task 99 — WANDR rename: a-03 lineage-tree follow-up

Status: **OPEN (handoff)** — created by the WART→WANDR rename (2026-06-09).

The whole in-tree project was renamed WART→WANDR. The a-03 native modules that
live in the **separate LineageOS build tree** (`~/android/lineage`, *not present on
this machine*) still carry their old `wart-*` / `libwart_*` names. Their in-tree
*references* (Rust dlopen strings, abstract-socket name, `Android.bp` module names
under `runtime/wandr-*`, the prebuilt `runtime/wandr-sensors/libwandr_sensors_hal.so`
filename) were already renamed to `wandr-*` in lockstep. **The lineage-tree sources
must be renamed + rebuilt to match, before the next on-device deploy** — otherwise
the host will dlopen / connect to names that the (stale) device binaries don't export.

## What to rename in `~/android/lineage` (on the machine that hosts that tree)

External module dirs + their soong/make module names + sources:

| Lineage path | Rename to | Module `name:` / artifact |
|---|---|---|
| `external/wart-inputflinger/` | `external/wandr-inputflinger/` | `Android.bp` `name: "wart-inputflinger"` → `"wandr-inputflinger"`; `name: "wart_wininfo_probe"` → `"wandr_wininfo_probe"`; srcs `wart_inputflinger.cpp` → `wandr_inputflinger.cpp`, `wart_wininfo_probe.cpp` → `wandr_wininfo_probe.cpp` |
| `external/wart-framework-shim/` | `external/wandr-framework-shim/` | `name: "wart-framework-shim"` → `"wandr-framework-shim"`; src `wart_framework_shim.cpp` → `wandr_framework_shim.cpp` |
| `external/wart-sensormanager/` (or wherever it lives) | `external/wandr-sensormanager/` | `name: "wart-sensormanager"` → `"wandr-sensormanager"`; src `wart_sensormanager.cpp` → `wandr_sensormanager.cpp` |
| `external/sf_surface/` (sensor HAL shim) | (name unchanged — `sf_surface` not wart-named) | produces `libwart_sensors_hal.so` → **`libwandr_sensors_hal.so`**; the standalone sensors HAL C++ shim symbols/strings carrying `wart` → `wandr` |

Also grep the lineage tree for any `wart`/`WART`/`Wart` (sources, `.bp`, `.mk`,
SELinux `.te`, init `.rc`) using the same case-preserving rule as the in-tree rename
(see the rename helper / commit history on the `rename-wart-to-wandr` branch).

## The runtime contracts that must stay in lockstep

These already say `wandr` in-tree; the lineage binaries must match after rebuild:

- **Abstract socket** `@wandr-inputflinger` — host `runtime/wandr-host/src/arbiter_sock.rs`
  ↔ the a-03 `wandr-inputflinger` binary. Mismatch = no input under `--no-art`.
- **dlopen** `libwandr_sensors_hal.so` — host/arbiter sensor driver loads it by this name.
- **Device paths** `/data/local/tmp/wandr-*` — deploy scripts push the a-03 binaries
  here under their new names.

## Build

After renaming in the lineage tree, rebuild via the fast-ninja path (see
`reference_a03_ninja_build` memory): only sources changed (plus `Android.bp` module
names → graph changed → use `m`), e.g.
`prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja wandr-inputflinger`
(use `m <module>` when the `.bp` module name/graph changed). Re-pull the rebuilt
`.so`/binaries into `runtime/wandr-*/` (the in-tree prebuilt slots) and redeploy the
whole stack in one lockstep push (see the rename plan's handoff checklist).

## Related deferred items (same rename, separate repos)

- **codeberg main repo rename DONE** — `codeberg.org/harryzz/wart` → `…/wandr`; local
  `origin` `git remote set-url`'d + in-tree main-repo URLs updated. STILL PENDING: the
  per-app repos `wart-app`, `wart-app-md-smoke`, `wart-arbiter`, `wart-host`,
  `wart-leak-repro`, `war.ime.keyboard`, `war.lang.{bg,fr}` — those `codeberg.org/harryzz/`
  URLs are intentionally **left as-is** in-tree until each repo is renamed.
- **libsignal-service-rs fork** (`external/libsignal-service-rs`, branch
  `wart-wasi-transport`) — `wart-wasi-shims/{wart-step-executor,wart-reqwest-shim,
  wart-reqwest-websocket-shim}` kept their names; in-tree path-deps point at them.
- **rsbinder fork** branch `wart-recursive` — left as-is.
