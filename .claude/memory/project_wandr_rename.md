---
name: project_wandr_rename
description: WART→WANDR project rename — in-tree rename DONE (8 staged commits on branch rename-wart-to-wandr); device redeploy + a-03 lineage tree + codeberg repos deferred.
metadata: 
  node_type: memory
  type: project
  originSessionId: f2cd0f9c-e8eb-4040-bff2-1d910574f75e
---

**WART → WANDR rename — IN-TREE COMPLETE (2026-06-09).** Done on branch
`rename-wart-to-wandr` as 8 staged, revertable commits (one per area), case-preserving
`WART/Wart/wart → WANDR/Wandr/wandr` (+ `war.`/`war:`/`warpkg` → `wandr.`/`wandr:`/`wandrpkg`).
Tree-wide residual is zero except deliberately-protected tokens (below). cargo
metadata resolves for the wandr-arbiter workspace + wandr-call; `git submodule status`
clean (all 9 wandr-host vendor submodules relocated, pointers unchanged).

**Stages (commit order):** 1 docs/markdown · 2 WIT namespaces (`war:`/`wart:`→`wandr:`,
files `wandr-ime-keyboard.wit`/`wandr-app.wit`) · 3 app-ids + 14 app dirs
(`apps/{system,user}/wandr.*`, `wandr-app`) + warpkg · 4 crate dirs + Cargo graph
(`crates/wandr-call`, `runtime/wandr-{arbiter,net,sensormanager,sensors,inputflinger,
framework-shim,hal-*}` + nested `wandr-arbiter-*`) · 5 `runtime/wandr-host`
(submodule-aware: git mv + `.gitmodules` path+section-name edits + local `.git/modules`
relocation + gitlink/back-pointer fixups) · 6 tools/repros/scripts
(`build-system-wandrpkgs.sh`, `install/uninstall-wandr-stack-magisk.sh`,
`smoke-wandrpkg.sh`, `tools/wandr-launch`) · 7 a-03 in-tree artifacts
(`wandr_*.cpp`, `libwandr_sensors_hal.so`) + `tasks/99` lineage note · 8 memory +
task/log filenames + final sweep.

**Protected — left as `wart` ON PURPOSE (separate repos / deferred, do NOT "fix"):**
- Per-app `codeberg.org/harryzz/{wart-app,wart-app-md-smoke,wart-arbiter,wart-host,
  wart-leak-repro,war.ime.keyboard,war.lang.bg,war.lang.fr}` repo URLs — those repos
  aren't renamed yet. (The MAIN repo `harryzz/wart`→`harryzz/wandr` IS done — remote
  set-url'd + in-tree URLs updated 2026-06-09.)
- ✅ RENAMED 2026-06-14 (no longer protected): libsignal-service-rs fork shims →
  `wandr-wasi-shims/`, crates `wandr-step-executor` / `wandr-reqwest-shim` /
  `wandr-reqwest-websocket-shim` (+ underscore forms), branch `wandr-wasi-transport`;
  consumers (wandr.signal engine, repros/signal-link, signal-phase0) + Cargo.lock +
  memory slug `project_wandr_step_executor` all updated. Fork commit sits on
  `wandr-wasi-transport`; PUSH the fork + rename the codeberg branch when publishing.
- rsbinder fork branch `wart-recursive` (`.gitmodules`) — still `wart` (separate fork).
- False positives intentionally kept: `wartime` (in this rename's own notes), `mewart`
  ("edit me"→"edit mewart" text-input artifact, tasks/61).

**DEVICE REDEPLOY — DONE + device-verified (2026-06-09).** Full `--no-art` stack rebuilt
+ deployed under `wandr-*`/`wandr.*` and rendering; all system + user apps published;
a-03 modules renamed+rebuilt; Signal state copied old→new (preserved link+history). Four
non-obvious bugs the bring-up surfaced (fix each next time):
1. **`war_` wit-bindgen accessors** (`war_ime_ime`→`wandr_ime_ime`, war_alarm/notify/
   audio-focus/background): the `war:` WIT rename means bindgen emits `wandr_*`; the
   `war.`/`war:`/`war-` replace rules MISS the `war_`(underscore) caller refs. Host wouldn't compile.
2. **Stale `libsf_surface.so`** (a "cosmetic" reuse) registered `wart`/`wart-overlay`
   input windows + looked up the **`wart.windowreg`** binder service, but the rebuilt
   `wandr-inputflinger` registers `wandr.windowreg` → window-registration handshake
   silently fails → input dead. MUST rebuild libsf_surface from renamed `sf_surface.cpp`.
3. **`wandr.keyguard` was never in `build-system-wandrpkgs.sh`** → the topmost locked
   overlay had no cwasm → rendered empty navy over everything (looked like a total render
   break). Added it to the script. ‼️ **`screencap` does NOT capture the `--no-art`
   SurfaceControl overlay** — every shot was byte-identical blank even while the panel
   rendered fine; check `dumpsys SurfaceFlinger` frame counts + ASK THE USER what the
   panel shows, don't trust screencap.
4. **Mis-renamed fork crate** `wandr_step_executor`/`wandr-reqwest-*` in signal-engine +
   signal-phase0 Cargo.lock: the underscore protection was added in stage 6, so stage-3
   (apps) wrongly renamed them; restore `wandr_*`→`wart_*` for the libsignal fork crates.

a-03 build for new modules: `m` regenerates soong (modules land in sharded
`out/soong/build.aosp_arm64.N.ninja`) but dies in kati `dex_preopt_check`; build by
**soong output path through the COMBINED ninja** (`-k 0`, skips lsdump) — see tasks/99.

**STILL DEFERRED (separate repos):** codeberg per-app repos (wart-app/wart-arbiter/
wart-host/war.* — user said these are old/archived/moved, low priority); rsbinder fork
branch `wart-recursive` (libsignal `wart-wasi-transport`→`wandr-wasi-transport` now DONE).
The tracked `runtime/wandr-host/prebuilt/libsf_surface.so` is still the stale wart-named artifact.

Reusable rename helper (mask→case-preserving-replace→unmask) was at `/tmp/rename_wandr.pl`.
Relates to [[reference_wandr_apps_root_install]], [[feedback_build_system_wandrpkgs_wipes_apps_root]].
