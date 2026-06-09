---
name: project_keyguard
description: "Keyguard/lockscreen — a Role::Lockscreen surface + wandr.keyguard guest + wandr-arbiter-keyguard module; auto-lock on screen-off, swipe-to-unlock. DONE + device-verified (v1, insecure)."
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**Keyguard/lockscreen v1 (swipe-to-unlock, auto-lock on screen-off) — DONE +
device-verified on the Pixel 2 XL (2026-06-02).** A foreseen system_server
responsibility, modeled (per [[project_arbiter_window_server_design]]) as a special
surface role in the existing z-order model, not a bolted-on special. Ties into the
PowerManager work: auto-locks by reacting to the same `Event::ScreenState` the
`wandr-arbiter-power` poller emits. Commits: `4591600e` (M1+M2), `059e5977` (M3).
6th arbiter module: wm/shell/alarm/notify/power/**keyguard**.

**Architecture (arbiter decides, host applies — the host barely changed):**
- core `Role::Lockscreen` (surface.rs): topmost-but-below-statusbar; excluded from
  `visible_app()`/the task-cycle ring (wandr.keyguard added to shell `CHROME_APP_IDS`).
  bin `apply_role(Lockscreen)` = the foreground mechanism (SIGUSR2 show+focus+present)
  — NO new host signal/role.
- host `OverlayMode::Lock` (`--standalone-overlay-lock`): a FULLSCREEN surface (like
  None's `create()`) at SF layer `0x7000_0000` — above app (`0x4000_0000`) + taskbar
  (`0x6000_0000`), BELOW the status bar / IME (`i32::MAX`) so the status-bar
  clock/battery stays visible on the lock screen. Self-registers via
  `register_chrome_with_arbiter(id, "lock")`.
- `wandr-arbiter-keyguard` module: lock state `{locked, saved_fg}`. Auto-locks on
  `Event::ScreenState{live:false}` (immediate, no grace); `lock`/`unlock` verbs. Lock
  = record `visible_app` → `SetRole(app→Background)` (stops it fighting focus +
  Paused) + `SetRole(keyguard→Lockscreen)` (put_surface). Unlock = `SetRole(keyguard→
  Background)` + `Effect::Foreground{saved_fg}` (proper shell promote). SurfaceRemoved
  of saved_fg → drop it (unlock reveals home).
- `apps/system/wandr.keyguard`: light canvas guest (like wandr.statusbar) — big clock
  (`status::clock-text`) + "swipe up to unlock". Imports `wandr:keyguard/keyguard`
  (M3); a clear upward release (≥12% of screen, not a tap) calls `keyguard::unlock()`
  → host `keyguard_host_impl` forwards `unlock` → arbiter. `kind=system`.
- `run-hybrid-stack.sh` boot-launches the keyguard (`--standalone-overlay-lock`) +
  `lock`s → boot = locked.

**Device proof:** screen timeout/`input keyevent 223` → auto-lock; wake (`224`) →
lock screen renders (clock + hint, status bar on top, app+nav covered); swipe up
(`input swipe 720 2200 720 700`) → guest unlock() → host forward → arbiter UNLOCKED
→ launcher restored to fg. CLI `lock`/`unlock` also work. 8+ unit tests (keyguard 3
+ core/shell). Build host `build-host-android.sh`; deploy `run-hybrid-stack.sh`.

**Deferred (PIN/biometric is the real follow-up):** credential store in `/state` +
keypad UI + verify + lockout + the keypad/IME over the lockscreen (the v1 swipe-lock
has no editor focus so `reconcile_overlay` never engages the IME over the keyguard —
PIN changes that). Also: lockscreen wallpaper, lockscreen notifications, secure-
while-locked (hide the Background app's last frame), lockscreen rotation (it renders
portrait — not fanned orientation as Lockscreen role). Gotcha: the screen times out
(2-min) during long adb sequences → the lockscreen auto-locks + the display is off
(black screencap); wake with `input keyevent 224` before screenshotting.
