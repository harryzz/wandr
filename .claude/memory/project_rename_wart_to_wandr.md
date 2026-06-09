---
name: project_rename_wart_to_wandr
description: "PENDING TASK — rename the whole project WART→WANDR; large blast radius (504 files), several non-mechanical danger areas. Do step-by-step in a fresh session."
metadata: 
  node_type: memory
  type: project
  originSessionId: 7ef4e6b8-b380-4a69-9e9d-1f758a1d9c33
---

**Pending task (user-requested 2026-06-09, to be done in a NEW session, step by step,
"very carefully"):** rename the project from **WART → WANDR** everywhere the word
appears. Case-preserving: `WART→WANDR`, `Wart→Wandr`, `wart→wandr`.

**Blast radius (read-only scope, 2026-06-09):** 504 tracked files / ~13,149 lines
contain "wart" (74 files have `WART`, 16 `Wart`, 497 `wart`). This is NOT a one-shot
sed — most lines are safe text (docs/`tasks/`/memory/comments) but these categories
need care and are easy to get wrong:

1. **Device/runtime paths** — `/data/local/tmp/wart-*` (~18 families: `wart-host`,
   `wart-arbiter`, `wart-apps`, `wart-host-<pid>.sock`, `wart-inputflinger`,
   `wart-sensormanager`, `wart-net`, `wart-launch`, `wart-screen`, `wart-stack`,
   `wart-zygote`, `wart-magisk-sweep`, …). Renaming these **breaks the live stack
   until the WHOLE stack is rebuilt+redeployed in lockstep** (the device has running
   processes + installed apps under `wart-apps` incl. Signal `/state`). Stage: change
   all path strings + deploy scripts together, then one clean redeploy. Don't half-do it.
2. **Crate/dir names** — `runtime/wart-*` (host, arbiter, framework-shim, inputflinger,
   net, sensormanager, sensors, hal-{display,lights,net,sensors}) + `crates/wart-call`.
   Each = `git mv` + every `Cargo.toml` path-dep + `[package] name` + every `use`/module
   path. Binary names too (`wart-host` bin is `wasm-android-host` — check the actual bin
   names vs dir names).
3. **`war.` package-id prefix + "warpkg"** — JUDGMENT CALL: `war` is NOT literally `wart`
   (the user said "everywhere the word WART appears"). `war.signal`, `war.launcher`,
   `war.ime.keyboard`, etc. + the "warpkg" term may or may not be in scope. **Confirm with
   the user FIRST** before touching these (renaming app_ids also orphans installed app
   state on-device). Likely answer: leave `war.`/`warpkg` OR rename to `wandr.`/`wandrpkg`
   — user decides.
4. **External repo rename** — remote is `codeberg.org/harryzz/wart.git`. Renaming the repo
   is an outward-facing action (rename on codeberg + `git remote set-url`). The wart-named
   things are in-tree; the wart-specific submodules are NOT wart-named (audioclient-rs,
   opus-rs, rsbinder) so they're unaffected. Confirm before renaming the remote.
5. **a-03 AOSP modules** — `wart-inputflinger`, `wart-sensormanager`, `libwart_sensors_hal.so`,
   `libsf_surface`/`wart-audioclient-ref` live in the lineage tree on a-03 with their own
   `Android.bp`/`.mk` module names → renaming needs a-03 edits + rebuilds (see
   [[reference_a03_ninja_build]]).
6. **False positives** — word-boundary the replace (avoid "Stewart"/"wartime"/substrings;
   and remember `war.` ≠ `wart`). Check `.task-state` (one-line `wart` refs), CLAUDE.md,
   `docs/`, `tasks/`.

**Suggested staging (fresh session):** (a) settle policy with the user — does `war.`/
`warpkg`/the codeberg repo rename too? (b) safe text first (docs/tasks/memory/comments);
(c) code identifiers + crate-dir `git mv` (keep it compiling each step); (d) device paths
+ deploy scripts in lockstep → one full rebuild+redeploy + device verify; (e) a-03 modules
+ rebuild; (f) external repo rename last. Commit per stage so each is revertable. Relates
to [[reference_wart_apps_root_install]] (the apps-root path is one of the device paths),
[[feedback_build_system_warpkgs_wipes_apps_root]] (don't wipe live app state during the
path migration).
