---
name: project_signal_bg_receipt
description: "Signal background-receipt subsystem — three generic host/arbiter primitives (hidden wake, background-execution bg-tick, notifications) + Signal wiring. M1-M3 done+verified+pushed; M4 (Signal) needs the user."
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**Signal background-receipt follow-up** ([[project_signal_client_architecture]] said
background delivery must be a GENERIC host capability, never a Signal daemon; the alarm
primitive [[project_alarm_manager]] was the timed-wake half). User chose the **full
subsystem**: persistent background socket + wake-from-dead + a notification primitive.
Built as ordered milestones, each device-verified (Pixel 2 XL) + pushed to codeberg main.
Plan: `~/.claude/plans/cat-task-state-steady-stallman.md`.

- **M1 — hidden wake** (`55f7ee02`): `Effect::Launch` (alarm/keep-alive wake) now
  `apply_role(Background)` right after launch → the relaunched GUI guest comes up HIDDEN
  (SIGUSR1, no foreground steal). Fixes the foreground-steal bug from the alarm increment.
- **M2 — background-execution primitive** (`8ff96df6`): `wit/background.wit`
  (`war:background/background.bg-tick: func()->u32`) + manifest `background = true`
  (`app_loader::background_service()`). `run_cwasm_loop` calls `bg-tick` IN PLACE of
  render_frame while the guest is a backgrounded background-service (no hidden-surface
  render; guest-authored cadence, host-clamped 16..=IDLE_CAP). This is the ONLY guest entry
  for a wake-from-dead service relaunched straight into Background. Verified: bg-tick pumps
  continuously while backgrounded, no renders, resumes on foreground.
- **M3a — notification primitive** (`2fef4244`): `wit/notify.wit` (`notifier` import,
  `notify-handler` export); `wart-arbiter-notify` module (post/cancel/list/click verbs,
  in-memory Store list, percent-decodes title/body off the control line); `Effect::Foreground`
  (binary re-enters the `foreground` verb); host `notify_host_impl` forwards posts;
  conditional `notify_events` `.ok()` binding → `on-notification-click`.
- **M3b — status-bar surfacing** (`5d366567`): `notify-feed` interface (`list-active`,
  `click`) the host answers by querying the arbiter LIVE (`notify-list`) — no host cache.
  war.statusbar imports it, draws a `● N` badge between clock+battery, tap → `notify-feed.click`
  → arbiter foregrounds owner + delivers on-notification-click. Verified with a real `input tap`.

**Test harness:** `apps/user/war.alarm.test` is now the kitchen-sink guest — exports
alarm-handler + bg-tick + notify-handler, imports scheduler + notifier (`background=true`).

**M4 — Signal wiring** (`b18a3198`): DONE + **fully device-verified by the user 2026-06-01**
— backgrounded Signal received a message sent from a 2nd device, the status-bar notification
appeared, and tapping it opened the right thread. (`/state`/link preserved across deploy.) `package.toml background=true`; world
exports background/notify-handler/alarm-handler + imports notifier/scheduler (deps under
`ui/wit/deps/`); the export Guest traits are impl'd on dioxus-canvas's `__DioxusCanvasGuest`
right AFTER the `wire!` macro (export! resolves them crate-wide — the integration seam).
`bg-tick`→`pump()` with an **idle-ramp** (`34afc910`): 250 ms active → 500 → 1000 ms (=host
cap) as the socket goes quiet, reset to 250 on inbound — 4× fewer wakeups when idle, no
hidden-surface render; new inbound (not open thread) →
`notifier::post` (one per thread, FNV id, title via `resolve_thread`); open/read → cancel;
`on-notification-click`→`PENDING_OPEN`→`app()` navigates; `on-alarm`→`pump()`; init schedules
a **15-min** keep-alive alarm (`KEEPALIVE_MS=900_000` — Android's periodic-job floor; the
alarm only does real work when Signal is dead, so a long interval saves wakeups). Verified on launch: all 3 exports bind, alarm armed
(repeat=300000ms), engine handshakes chat.signal.org. NOTE: the Signal UI already
background-pumped via render@1Hz when paused (post-task-64 on-demand) — M4 adds the
notification ALERT + clean bg-tick pump + wake-from-dead, the actual new value.

**Doze (PowerManager):** now a proper **`wart-arbiter-power` module** (`5aa52868`,
refactored from the v0 host-local `351394a8` after the user flagged that host-side
policy diverged from "arbiter decides, host applies"). The arbiter polls
`debug.tracing.screen_state` (`spawn_screen_poller`, 2s) → `Event::ScreenState` →
PowerModule applies the 60s grace + fans `doze <cadence-ms>` to every tracked
host; the host is a dumb applier (`ime_inbound` `Doze` → stretches its
render+bg-tick cadence to the pushed value). **Class-based (`dabddd76`):** the
cadence is per-app by power class — bg-service → maintenance 10s (keeps
receiving), everyone else → suspend 60s. The loader reports the class up
(`report-power-class <pid> <bg-service|normal>`, from the manifest `background`
flag) → PowerModule owns the bg-service pid set (cleaned on SurfaceRemoved) +
fans per-class. "Host reads/reports, arbiter owns/decides" — the arbiter does NOT
parse the manifest (future PackageManager). Foundation for user per-app profiles
(restricted/optimized/unrestricted) + wakelock exemptions. Device-verified:
Signal→doze 10000, launcher→doze 60000. Device-verified: arbiter logs doze
ENTER/EXIT, host applies within ~60ms; 10× fewer pumps when screen off, resumes on
on. Known minor gap: a host launched mid-doze defaults normal until the next
transition (fan is on-change). It SLOWS not suspends It SLOWS, not suspends: a single keep-alive
`on-alarm` is one engine step, far short of an async reconnect, and userspace can't
suspend the SoC — so a coarse cadence (socket still serviced within Signal's
~30-55s keepalive; msgs within ~10s when off) is the correct simple win, no
maintenance-window state machine. Screen state = the existing
`debug.tracing.screen_state` watcher. **Key insight: do NOT use `IPowerManager`/
`IDisplayManager` — they're present on-device but are the system_server ART layer
we drop ([[feedback_no_art_layer_dependencies]]); `IPower` (HAL, survives) is
perf-hints not screen-state; SurfaceFlinger (survives) knows powerMode but doesn't
cleanly expose it over binder for READ — it surfaces it as the SF-sourced sysprop
we already read.** The policy consumes a screen on/off bool, so the source is
swappable in boot-model (task 33) without touching it. **Device fact (Pixel 2 XL /
LineageOS): on power-press/timeout it goes to AOD/Doze (`screen_state`=3, SF
`powerMode=Doze`, dim — NOT fully black), and the sysprop transition LAGS ~2-5s.
`is_live()` (On|Vr only) treats Doze as not-live, so doze engages correctly —
verified end-to-end with a REAL power-key screen-off (not just a faked setprop):
doze ENTER ~65s after power-off, EXIT on wake.** The user's "screen never goes
off" was a 30-min `screen_off_timeout` + AOD-not-black, not a doze bug. A `wart-arbiter-power`
module is deferred until richer policy (wakelocks, idle stages, SoC-wakelock deep
doze) needs it (same discipline as audio-focus).

**Gotchas burned this session (all real):**
- `bindgen!` is NOT re-triggered by a `.wit`-only edit — `touch` the file holding the macro
  (host `lib.rs`) or you get a STALE cached expansion that still compiles but mismatches at
  link time ("function implementation is missing"). Cost ~30 min on M3b.
- A top-level `record` in a WIT package is INVALID — types must live INSIDE an `interface`.
  (Compiled only because of the stale-bindgen cache above.)
- `adb push <dir> <target>` NESTS into `<target>/<dir>` if target exists → installer reads a
  stale manifest. `rm -rf` the device dir first.
- A guest `wit/` dir importing a cross-package dep needs the VERSIONED import
  (`war:notify/notify-feed@0.1.0`) + `generate_all` in `wit_bindgen::generate!`.
- Build host with `tools/scripts/build-host-android.sh` (sources env-android.sh); a bare
  `cargo build` lacks the NDK clang env (zstd-sys fails). Arbiter: `cargo build --release`.
  Deploy zygote+arbiter restart keeps chrome alive; statusbar is a `--standalone-overlay-top`
  process (not a zygote child) — relaunch it separately. NEVER build-system-warpkgs.sh.
