---
name: project_task_manager
description: Task 92 wandr:task-manager (running-apps / task manager for guests) — DONE + device-verified
metadata: 
  node_type: memory
  type: project
  originSessionId: a2edab94-9d77-4289-807e-6fabf67af25c
---

Task 92 — `wandr:task-manager@0.1.0` (running-apps / task manager for guests). ✅ DONE + device-verified 2026-06-09 (Pixel 2 XL, --no-art).

**Shape (mirrors wandr:alarm/notify):** arbiter is the authority on the app set + roles; host implements the WIT by forwarding to the arbiter + enriching from `/proc`. Polling model (guest re-calls `list-apps` on a timer; cpu‰ is 0 on first poll, delta-based after).

**Where it lives:**
- Arbiter: `wandr-arbiter-shell/src/am.rs` `cmd_task_list` (emits `app=<id> pid= state=<fg-mapped> uptime_ms=` per app; `role_to_state` flattens Role) + `cmd_task_kill` (protected guard = `CHROME_APP_IDS` + `store.home()` → `ERR protected`; unknown → `ERR not-found`). Verbs registered in shell `lib.rs` verbs()+on_command; CLI verbs in `wandr-arbiter-bin/src/main.rs` (task-kill in needs_arg). 2 unit tests.
- Host: `runtime/wandr-host/src/task_manager_host_impl.rs` — `query_full` (reply-to-EOF, unlike launcher's 64-byte cap), per-pid `/proc/{stat(utime+stime),status(VmRSS/Threads),smaps_rollup(Pss)}` + `/proc/meminfo`; cpu‰ from module-`static CPU_SAMPLES: Mutex<HashMap<pid,(ticks,Instant)>>` (host is per-guest so correctly scoped). bindgen `task-manager-host` world in `lib.rs` + **empty `types::Host` impl required** (bindgen emits a marker Host for the types-only interface; add_to_linker needs it). `add_to_linker` in BOTH linker paths of `app_loader.rs`.

**Key decision (deviation from task-file sketch):** `kind` (system/user) + `label` are derived HOST-side from the install layout (`apps/<id>` vs `system-apps/<id>` under `app_loader::apps_root()`, label = flat `label` key in package.toml), NOT threaded through arbiter `AppState`. Avoids touching core + 8 insert_app sites; the host already owns layout knowledge.

**Test guest:** `apps/user/wandr.taskmanager` = dioxus-canvas (the Signal-guest combined-`generate!`+`wire!` pattern for an extra host import, NOT one-line `launch!`). Polls from `pre_frame`, `mark_dirty` on change, idle `set_min_frame_delay(1500)`. UI: scale **1.5** (2.0 crowds name into usage column), `wandr.` namespace stripped from name+id, state abbreviated fg/bg, usage+kill columns `flex-shrink:0` + name `min-width:0;overflow:hidden` so the End button never clips.

**Deploy (no apps-root wipe):** `cargo build --target wasm32-wasip2 --release` → wasip2 output IS already a component (no embed/adapter for Rust guests) → pack components/ui.wasm+package.toml → `LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=… wandr-host --install <pkg>` → `run-hybrid-stack.sh --wandr-only` to restart the wandr layer with the new host+arbiter binaries. See [[reference_wandr_apps_root_install]], [[feedback_build_system_wandrpkgs_wipes_apps_root]].

Follow-up (out of scope): push `on-apps-changed` events world (mirror [[project_signal_bg_receipt]]'s handler pattern).
