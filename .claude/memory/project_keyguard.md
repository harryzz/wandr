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

**Power menu (task 110, 2026-06-16, device-verified):** a 2nd modal overlay
(`wandr.powermenu`, 2×2 card: Emergency/Lockdown/Power off/Restart) in the SAME
`wandr-arbiter-keyguard` module reusing the `wandr:keyguard/keyguard` channel
(added `pm-*` verbs → no new host bindings). Long-press fires WHILE HELD (>1s):
arbiter arms a 1s timer thread on power-down, shows if still held; short press =
panel toggle on release. gen-counter (POWER_GEN) dedups the multi-host power-key
fan-in. Wakes from screen-off via `panel on` first. ⚠️ **GOTCHA — modal over the
lock screen:** keyguard AND powermenu both sit on `Role::Lockscreen` → identical
input-z in `wm::input_window_block` → the dispatcher (first-match wins) routed ALL
touch to the keyguard's swipe area, menu buttons dead. Fix: `do_show_menu` demotes
the keyguard to `Background` (drops it out of the input window list) while the menu
is up so the menu is the sole focusable Lockscreen surface; restored on dismiss
(`menu_covered_keyguard` flag). Any future "modal over lockscreen" must do the same
— equal-role surfaces share input-z and first-wins. Power off/Restart real
(`/system/bin/reboot [-p]`); Emergency fake. Commits `0b01cbd3` + earlier `1999573a`.

**⚠️ FBE / CE storage under --no-art (2026-06-16):** `/data/media/0`, `/data/user/0`,
`/data/data` are FBE **Credential-Encrypted** (`ro.crypto.type=file`); they show
ciphertext filenames (base64-ish gibberish) until user 0's CE key is installed in
the kernel keyring (`sys.user.0.ce_available=true`). Normally LockSettingsService
drives `vold` to install it after lockscreen auth. **The boot-default Magisk module
(`service.sh`) kills system_server**, so if it kills BEFORE the unlock lands, the
WHOLE stack (music/photos/app data) runs on ciphertext — data is NOT lost, just
locked. Fix shipped: `service.sh` waits (bounded 20s) for `sys.user.0.ce_available
=true` BEFORE stopping the framework. Crucial nuance: with **NO lockscreen
credential** system_server unlocks user 0 automatically early in boot and the
in-kernel fscrypt key SURVIVES system_server being killed → wait-then-kill works.
With a **credential set**, ce_available never flips without authentication → CE
stays locked under --no-art (the wait times out, proceeds). Unlocking CE *with* a
PIN under --no-art = the deferred keyguard-PIN task below (collect PIN → Gatekeeper/
Weaver verify → synthetic password → `vold` install CE key). Confirm data intact as
ROOT (`adb root`; shell uid gets EPERM on `/data/media/0` even when decrypted).

**Deferred (PIN/biometric is the real follow-up):** credential store in `/state` +
keypad UI + verify + lockout + the keypad/IME over the lockscreen (the v1 swipe-lock
has no editor focus so `reconcile_overlay` never engages the IME over the keyguard —
PIN changes that). Also: lockscreen wallpaper, lockscreen notifications, secure-
while-locked (hide the Background app's last frame), lockscreen rotation (it renders
portrait — not fanned orientation as Lockscreen role). Gotcha: the screen times out
(2-min) during long adb sequences → the lockscreen auto-locks + the display is off
(black screencap); wake with `input keyevent 224` before screenshotting.
