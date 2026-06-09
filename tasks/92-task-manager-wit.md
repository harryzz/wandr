# Task 92 — wandr:task-manager WIT (running-apps / task manager for guests)

> Status: ✅ DONE + DEVICE-VERIFIED (2026-06-09). A guest enumerates running apps,
> sees kind (system/user) + state + CPU/mem, and kills an app. Polling model (no
> push events — decided 2026-06-06).
>
> **Implemented:** arbiter `task-list`/`task-kill` verbs (`wandr-arbiter-shell/src/am.rs`,
> role→state map + protected guard = CHROME_APP_IDS + home; CLI verbs in
> `wandr-arbiter-bin/src/main.rs`; 2 unit tests). Host `task_manager_host_impl.rs`
> (forwards to arbiter, enriches per-pid from `/proc/{stat,status,smaps_rollup}` +
> `/proc/meminfo`; CPU‰ from a module-static delta map; bindgen `task-manager-host`
> world + empty `types::Host` impl + `add_to_linker` in both linker paths of
> `app_loader.rs`). **Decision:** `kind`+`label` derived host-side from the install
> layout (`apps/` vs `system-apps/` under `apps_root()`), NOT threaded through the
> arbiter `AppState`. Test guest `apps/user/wandr.taskmanager` = dioxus-canvas
> (combined `generate!` + `wire!`, polls `list-apps` from `pre_frame`; scale 1.5;
> `wandr.` stripped + `fg/bg` abbreviations; flex-shrink:0 usage+kill columns so the
> kill button never clips). Deployed per-app via `wandr-host --install` (NOT the
> apps-root-wiping build script); stack restarted with `run-hybrid-stack.sh
> --wandr-only`. Device-verified: rows for all chrome/launcher/keyguard (system) +
> signal/taskmanager (user), live CPU deltas, PSS<RSS, protected kills refused,
> user-app kill works.

## What it is

`wandr:task-manager@0.1.0` (`wit/task-manager.wit`) — a separate package (like
`wandr:alarm`/`wandr:connectivity`, so no `my:skiko-gfx` sync churn). One import
interface `task-manager`:

- `list-apps() -> list<app-info>` — every running app, enriched.
- `system-mem() -> system-memory` — device RAM (total / available / wandr PSS sum).
- `kill-app(app-id) -> result<_, kill-error>`.

`app-info` = `app-id`, `label`, `pid`, `kind` (`system|user`), `state`
(`foreground|background|overlay|headless`), `uptime-ms`, `usage`. `resource-usage`
= `cpu-permille` (recent CPU in ‰ of one core, delta-based), `cpu-time-ms`
(cumulative), `mem-rss-kb`, `mem-pss-kb` (honest footprint given zygote COW),
`threads`. `kill-error` = `not-found | protected | failed(string)` (`protected` =
launcher/keyguard/chrome the arbiter would relaunch). Worlds: `task-manager-client`
(guest import), `task-manager-host` (host linker).

## Implementation plan (follow-up)

Mirror the alarm/connectivity host-forwards-to-arbiter pattern:

1. **Arbiter** — extend the existing `cmd_list` (`wandr-arbiter-shell/src/am.rs:129`)
   or add a `task-list` verb that emits, per app, the fields the WIT needs:
   `app_id`, `pid`, `kind`, `state` (from `Role`), `uptime_ms`. Source: the
   `AppState` registry + the surface `Role`. `kind` from the install class
   (system-apps/ vs apps/) — thread it through `AppState` at launch (the loader
   knows). `kill-app` → existing arbiter `kill` path (return `protected` for
   home/keyguard/chrome the arbiter relaunches).
2. **Host** — `task_manager_host_impl.rs`: implement `task-manager`. `list-apps`
   queries the arbiter (`task-list`), then for each pid samples `/proc/<pid>/stat`
   (utime+stime → cpu-time-ms; delta vs the last sample → cpu-permille; keep a
   `pid → (cpu_ticks, instant)` map in the host for the delta), `/proc/<pid>/status`
   (VmRSS, Threads), and optionally `/proc/<pid>/smaps_rollup` (Pss). `system-mem`
   reads `/proc/meminfo` (MemTotal/MemAvailable) + sums PSS. `kill-app` forwards
   to the arbiter. `bindgen!` the two worlds in `lib.rs`; `add_to_linker` in
   `app_loader.rs`. (Host runs as root → can read any `/proc/<pid>`.)
3. **WIT-sync** — mirror `wit/task-manager.wit` into the test guest's `wit/deps/`
   per the sync rule (it's a separate package, so no skiko-gfx mirror needed).
4. **Test guest** — a small `apps/user/wandr.taskmanager` GUI guest (dioxus-canvas
   or rust-canvas) that polls `list-apps` every ~1.5 s and renders rows
   (label, kind badge, state, CPU‰, mem) + a kill button; device-verify under
   `--no-art` (the arbiter/host already run there).

## Notes / decisions
- **Polling, not push** (user choice 2026-06-06): the guest refreshes on a timer;
  `cpu-permille` is 0 on the first poll for a freshly seen pid (needs two samples).
  A push `on-apps-changed` events world can be added later if wanted (mirror
  `wandr:connectivity`'s handler) — out of scope here.
- `kill` by `app-id` (the arbiter's key), not pid.
- PSS is the honest per-app number under the zygote COW model (RSS overcounts the
  ~180 MB shared working set); sample it but allow `0` when skipped for cost.

See `wit/task-manager.wit`, `wit/alarm.wit` (pattern), `wandr-arbiter-shell/src/am.rs`
(`cmd_list`/`kill`), `[[feedback_wandr_zygote_fork_survival]]` (PSS vs RSS).
