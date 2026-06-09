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
- All `codeberg.org/harryzz/{wart*,war.*}` repo URLs (main repo + per-app repos) —
  repo rename is a separate outward-facing step.
- libsignal-service-rs fork (`external/libsignal-service-rs`): dir `wart-wasi-shims/`,
  crates `wart-step-executor` / `wart-reqwest-shim` / `wart-reqwest-websocket-shim`
  (hyphen + underscore forms), branch `wart-wasi-transport`. In-tree path-deps point at them.
- rsbinder fork branch `wart-recursive` (`.gitmodules`).
- Memory slug `project_wart_step_executor` (it's about the protected fork crate).
- False positives intentionally kept: `wartime` (in this rename's own notes), `mewart`
  ("edit me"→"edit mewart" text-input artifact, tasks/61).

**DEFERRED handoffs (NOT done this session — see also [[project_wandr_rename_a03]] / tasks/99):**
1. **Device rebuild + lockstep redeploy** to `/data/local/tmp/wandr-*` + reinstall apps
   (app_ids changed → on-device `war.*` state incl. Signal link/history is orphaned).
2. **a-03 lineage tree** (`~/android/lineage`, not on this machine): rename
   `wart-inputflinger`/`wart-framework-shim`/`wart-sensormanager` modules +
   `libwart_sensors_hal.so` + ninja rebuild — must precede next deploy (socket/.so
   names already `wandr` in-tree). Full instructions: `tasks/99-wandr-rename-a03-followup.md`.
3. **codeberg repo rename** `…/wart` → `…/wandr` + `git remote set-url` (+ per-app repos).
4. The fork branches (libsignal `wart-wasi-transport`, rsbinder `wart-recursive`) if ever
   desired — separate-repo work.

Reusable rename helper (mask→case-preserving-replace→unmask) was at `/tmp/rename_wandr.pl`.
Relates to [[reference_wandr_apps_root_install]], [[feedback_build_system_warpkgs_wipes_apps_root]].
