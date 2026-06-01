---
name: project_alarm_manager
description: "Arbiter Inc. 3c — AlarmManager/JobScheduler timed-wake primitive. Guest schedules via war:alarm; arbiter fires on a timer, delivering on-alarm (alive) or relaunching (dead). Done + device-verified with a test guest."
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**Arbiter Inc. 3c — AlarmManager / JobScheduler (timed-wake). DONE + fully
device-verified (Pixel 2 XL, 2026-06-01).** The design doc's flagged "real gap"
([[project_arbiter_window_server_design]]); enables (not yet wired) **Signal
background message receipt**. A guest asks to be woken later/periodically; the
arbiter fires it on a timer and delivers via the guest's `on-alarm` export —
whether it's alive (callback) or dead (relaunch). New `ArbiterModule` + the
`war:alarm` WIT, mirroring `war:ime` throughout.

**Commits (codeberg main, after foreground-only-orient a8ff578a):**
`af39a1ad` (arbiter side), `2f125869` (WIT + pid-resolution), `3256da1a` (host
wiring), + the test guest (this session). Pushed through 3256da1a.

**Architecture:**
- `wit/alarm.wit` (`war:alarm@0.1.0`, separate package — no skiko-gfx WIT-sync
  churn): `scheduler` import `schedule(id, delay-ms, repeat-ms)`/`cancel(id)` +
  `alarm-handler` export `on-alarm(id)`; worlds `alarm-client`/`alarm-host`/
  `alarm-events`.
- core `alarm.rs`: `Alarm{app_id, alarm_id, next_fire_ms, repeat_ms, wake_kind,
  pending_deliver}` on `Store.alarms` + persistence (`to_json`/`restore_from_json`
  — survives arbiter restart). `Event::AlarmTick`. `LaunchKind::as_wire/from_wire`.
- `wart-arbiter-alarm` crate: `schedule-alarm <owner> <id> <when-unix-ms>
  <repeat-ms> [kind]` / `cancel-alarm <owner> <id>` (owner = a bare-int pid →
  resolved via the registry, OR an app-id for the CLI). `on_event(AlarmTick)`
  fires due alarms — **alive** owner → `Effect::HostLine "alarm-fired <id>"`;
  **dead** owner → `Effect::Launch{wake_kind}` + `pending_deliver`, delivered the
  next ~1 s tick once up. Repeats reschedule to `now+repeat`, one-shots drop.
  Registered with one line.
- bin: timer thread (~1 s, **skips when no alarms** — no idle bus churn) emits
  `AlarmTick`; **wired `Effect::Launch`** (was a task-74 stub) → zygote launch +
  registry insert + surface. CLI passthrough for the verbs.
- host: `alarm_host_impl.rs` (scheduler import → forward `schedule-alarm
  <getpid> <id> <now+delay-ms> <repeat> gui`; absolute `when` from the device
  clock; **pid self-report avoids app-id threading**; `wake_kind=gui`).
  `alarm_host_bindings`(alarm-host)/`alarm_events_bindings`(alarm-events) bindgen;
  `AlarmHost::add_to_linker` (both instantiate paths); `InstantiatedApp.alarm_events`
  (`.ok()` probe, like `ime_events` — `None`/inert for non-alarm guests);
  `ime_inbound` `AlarmFired{id}`; standalone drain → `call_on_alarm`.

**One delivery mechanism (the `on-alarm` export) for both paths.** A dead-app
relaunch uses `wake_kind=gui` (relaunch as a Background GUI guest → its render
loop drains the delivered `alarm-fired` → `on-alarm`). A headless poll kind for
Signal is the follow-up.

**Test guest `apps/user/war.alarm.test`** (minimal Rust wasm32-wasip2 canvas
guest, like war.statusbar): imports scheduler + exports alarm-handler; trimmed
`my:skiko-gfx` in `wit/world.wit` + `war:alarm` in `wit/deps/alarm/`; on first
frame `schedule(1, 5000, 5000)`; `on-alarm` bumps a counter drawn as a bar.
`wit_bindgen::generate!` needs `generate_all` for the cross-package import. Build
`cargo build --target wasm32-wasip2 --release`; pack a warpkg (components/ui.wasm
+ package.toml, `kind` OMITTED = user app); install `LD_LIBRARY_PATH=/data/local/tmp
WART_APPS_ROOT=… wart-host --install <warpkg>` (per-app — NEVER build-system-warpkgs.sh).

**Device proof (all 5):** schedule (guest→arbiter, pid 13480→app resolved);
alive-deliver (`on-alarm(1)` at 5 s + 10 s); dead-relaunch (kill→`alarm waking
dead…via relaunch`→`Launch effect → pid 13849`→resumes); persistence (arbiter
restart → restored from state.json → keeps firing the surviving guest, which did
NOT re-schedule); cancel (`removed=true`, 0 dispatches after). No regression to
chrome/IME/launcher. Unit tests across core+alarm.

**Follow-up (Signal):** decide poll (alarm relaunch headless) vs persistent
background-service; needs the Signal engine/ui split ([[project_signal_client_architecture]]).
Build host: `build-host-android.sh`; deploy `run-hybrid-stack.sh`. Plan:
`~/.claude/plans/cat-task-state-steady-stallman.md`.
