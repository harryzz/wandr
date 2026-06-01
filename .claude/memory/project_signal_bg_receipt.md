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

**M4 — Signal wiring (NEXT; NEEDS THE USER — real Signal account + a 2nd device to send
from).** Signal UI: add `background=true`; export `bg-tick` → `chat::poll-events` + for each
new inbound `notifier::post`; export `on-notification-click` → open that thread; schedule a
coarse keep-alive alarm (crashed/rebooted Signal → hidden relaunch → reconnect → resident
bg-service). Engine already pumps+persists+reconnects per poll-events.

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
