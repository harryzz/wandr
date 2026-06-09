---
name: project_chrome_coherence
description: Arbiter Increment 3a — statusbar/taskbar are arbiter-tracked Role::Chrome surfaces; arbiter is the single orientation authority for all overlays; the orient-lock FILE is retired. Done + device-verified.
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**Arbiter Increment 3a — chrome-coherence. DONE + device-verified (Pixel 2 XL,
2026-06-01). Commit `1da7adb0` on main (after task 74).** Builds on the task-74
surface/role model ([[project_task74_surface_role_model]]); the design doc's flagged
next step ([[project_arbiter_window_server_design]]), now additive.

**Problem:** status bar + taskbar were host-spawned by `run-hybrid-stack.sh`
(`wandr-host --standalone-overlay-{top,bottom-bar}`), NOT arbiter-tracked, so the
arbiter couldn't push to them. Orientation lock was a cross-process **file**
(`/data/local/tmp/wandr-orient-lock`): the foreground app wrote `1/0`, every chrome
overlay polled it + ran its own device-sensor. That file is now **retired**.

**What shipped:**
- Chrome stays host-spawned but **self-registers**: on startup each overlay sends
  `register-chrome <app-id> <pid>` to the arbiter → shell inserts an `AppState` +
  `Role::Chrome` surface (inert for AM/IME — excluded from `visible_app`/cycle ring;
  pruned by the existing 5 s liveness poller, since chrome isn't a zygote child).
- The arbiter is the **single orientation authority** — there is ONE *system
  orientation* every visible surface displays at: `effective_orient = locked ? 0 :
  decided` (`wandr-arbiter-wm`, `DisplayGeometry.orientation_locked`). `geometry_line`
  uses it; `push_system_orientation` pushes it to the focused editor + visible app +
  every `Role::Chrome` surface + the `active_ime` pid (hidden IME stays orient-fresh
  → no stale-on-engage). Both `OrientationChanged` and `set-orientation-lock <0|1>`
  call it. **The lock gates the FOREGROUND APP too, not just chrome** — gating only
  chrome (commit 1da7adb0) let a locked launcher rotate while the bars stayed
  portrait (user-reported on physical rotation); fixed in **commit `fd258fb3`**. The
  arbiter pushes only the content `orient`; the **anchor stays host-side**
  (`overlay_rect` flip in standalone.rs).
- `register-chrome` + `set-orientation-lock` are host→arbiter verbs but are also on
  the `wandr-arbiter` CLI now (testing/debugging). NOTE: an arbiter-only restart drops
  the runtime-registered Chrome surfaces + active_ime + lock (none persisted) — chrome
  comes back as plain Background apps from `state.json`; re-establish with
  `wandr-arbiter register-chrome <app-id> <pid>` + `set-ime` + `set-orientation-lock`,
  or do a full `run-hybrid-stack.sh` restart (chrome self-registers + launcher reports
  lock at boot). A backgrounded auto app used to still report its sensor (drives the decided
  orient); **DONE — foreground-only reporting** (commit `fd45fe53`): the host gates the
  sensor poll + report-orientation on `app_role::role()==Foreground`, so only the
  visible app drives orientation (backgrounded apps skip the sensor; resume on
  foreground via the arbiter's ForegroundChanged orient push).
- Host (`wandr-host/src/standalone.rs`): chrome self-registers (retries, best-effort);
  the foreground app reports `set-orientation-lock` instead of writing the file; the
  orientation block is restructured so an **overlay's target orient comes ONLY from
  arbiter `geometry` pushes** (no sensor, no file) while **fullscreen is unchanged**
  (sensor → `report-orientation` → arbiter-decided / local fallback). Overlays no
  longer open a sensor handle. `run_cwasm_loop` gained an `app_id: Option<&str>` param
  (the render loop didn't have it). Deleted `ORIENT_LOCK_PATH` /
  `publish_orientation_lock` / `orientation_lock_active`.

**Device proof:** `wandr-arbiter list` shows wandr.statusbar + wandr.taskbar tracked
(no `[fg]`). With an UNLOCKED app fg, one `report-orientation 1` fans `orient=4` to
statusbar + taskbar + the hidden IME + the fullscreen app — each does its
`overlay rect flip`. With the LOCKED launcher fg, a landscape report leaves chrome at
`orient=0`. The orient-lock file stays ABSENT. Arbiter unit tests 22/22.

**Testing note:** physical rotation can be **simulated** without moving the device via
`wandr-arbiter report-orientation <0|1|2|3>` (the CLI verb the host sensor normally
sends) — drives the WM fan-out; watch the chrome hosts' `geometry … orient=N` +
`overlay rect flip` logcat. (Visual on-panel confirmation still needs the user's eyes.)
Launcher is `orientation=locked`; most apps are `auto` ([[reference_wandrpkg_manifest_convention]]).

**Deferred / additive on this:** real chrome lifecycle via the arbiter (launch chrome
through `launch-overlay-{top,bottom-bar}` instead of self-register) was the rejected
alternative; chrome-as-`Chrome`-surface also unblocks coherent inset authoring later.
Build: `build-host-android.sh` (wandr-host changed) + `run-hybrid-stack.sh`. Plan:
`~/.claude/plans/cat-task-state-steady-stallman.md`.
