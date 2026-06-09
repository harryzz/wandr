# Task 99 — WANDR rename: a-03 lineage-tree follow-up

Status: **DONE (2026-06-09)** — a-03 lineage modules renamed + rebuilt + redeployed in lockstep.

The a-03 native modules live in the **separate LineageOS build tree**
(`~/android/lineage`, reachable as `ssh a-03`). They were renamed to `wandr-*` and
rebuilt; the resulting binaries (with the renamed `@wandr-inputflinger` socket) were
pulled into the in-tree deploy slots `runtime/wandr-{inputflinger,framework-shim,sensormanager}/`.

## What was done in `~/android/lineage/external/` (DONE)

Renamed dirs + soong `cc_binary` module `name:` + sources (case-preserving):
- `wart-inputflinger/` → **`wandr-inputflinger/`** (modules `wandr-inputflinger`, `wandr_wininfo_probe`; srcs `wandr_inputflinger.cpp`, `wandr_wininfo_probe.cpp`)
- `wart-framework-shim/` → **`wandr-framework-shim/`** (`wandr-framework-shim`; `wandr_framework_shim.cpp`)
- `wart-sensormanager/` → **`wandr-sensormanager/`** (`wandr-sensormanager`; `wandr_sensormanager.cpp`)
- `wart-audioclient-ref/` → **`wandr-audioclient-ref/`** (kept — live C++ byte-diff reference for the audioclient-rs work, task 98)

**Deleted as dead** (doc-confirmed retired/superseded, NOT renamed):
- `wart-activityms/` — retired into `wandr-framework-shim` (task 96).
- `wart-sensors/` (`libwart_sensors_hal`) — the direct-HAL `wandr-sensors` daemon was deleted (task 94); sensors now go via `wandr-sensormanager`'s `ISensorManager`; the `.so` is dlopen'd nowhere. In-tree `runtime/wandr-sensors/` slot also removed.
- `wart_input_spike/`, `wart_inputflinger_spike/` — experiments that proved path A (task 83), now implemented in `wandr-inputflinger`.

Result: `external/` has **0 `wart` dirs, 4 `wandr` dirs**.

`sf_surface/` (module `libsf_surface`, not wart-named) had its sources updated to the
renamed `Wandr*` identifiers; exported `sf_*` ABI unchanged.

## Runtime contract (verified in lockstep)

- **Abstract socket** `@wandr-inputflinger` — baked into the rebuilt a-03 binary AND the
  arbiter default (`wandr-arbiter-bin/src/main.rs`). Verified via `strings`.

## Build procedure that worked (a-03, new modules)

`m` regenerates the soong analysis (the new modules land in the sharded
`out/soong/build.aosp_arm64.N.ninja`) but **dies in LineageOS kati** (`dex_preopt_check.mk`
`$(error)`), leaving the combined ninja a stub. The fix (per `reference_a03_ninja_build` +
`tasks/33`): build by **soong output path** through the **COMBINED** ninja (it defines
`highmem_pool` + subninjas the soong shards — the soong ninja standalone fails
`unknown pool 'highmem_pool'`), with `-k 0` to skip the unrelated `*.lsdump` ABI side-tasks:

```bash
ssh a-03 'cd ~/android/lineage && prebuilts/build-tools/linux-x86/bin/ninja -k 0 \
  -f out/combined-aosp_arm64.ninja \
  out/soong/.intermediates/external/wandr-inputflinger/wandr-inputflinger/android_arm64_armv8-a/wandr-inputflinger'
```
Then `scp` the stripped intermediate into `runtime/wandr-<mod>/…` and `adb push`.

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
