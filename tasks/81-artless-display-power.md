# Task 81 — ART-less display power ownership

> Status: 🟡 implemented, device-verify pending (2026-06-04). Found in human testing
> of `--no-art`: the device wedged (black screen, power button + touch dead) because
> **nothing owns display power when ART is off**. Implemented: power-key→panel toggle,
> arbiter owns screen state under `WANDR_NO_ART`, setPowerMode runs as uid system via
> `wandr-launch wandr-screen` (root HANGS on SF's permission check with ART off), boot
> force-on, sysprop poller gated. 13 power-module unit tests pass; aarch64 build OK.
> Device `--no-art` power-cycle verification is the remaining step.

## Why (the wedge)

With the Java framework stopped (`--no-art`), `PowerManagerService` is gone. PMS
normally (a) keeps the panel powered, (b) wakes it on the power button, (c) feeds
`debug.tracing.screen_state`. With it gone:
- the panel went **off** (`screen_state` → 1) and nothing turned it back on;
- the **power button did nothing** (power-key→wake is a PMS function; our host reads
  the key off evdev but ignores it);
- the arbiter's screen poller reads the now-**stale** `debug.tracing.screen_state`
  sysprop → spurious `doze ENTER` + keyguard auto-lock.

Recovery required `adb shell input keyevent 224` (WAKEUP) after restoring ART.

## Goal

The wandr stack **owns display power** when ART is off: the panel stays on, the power
button wakes/sleeps it, and doze/keyguard react to our own screen state — never to
the dead PMS sysprop. Never wedge.

## Approach (build on task 78's `wandr-hal-display` setPowerMode)

1. **Power-key → toggle panel** (the un-wedge). The host intercepts `KEYCODE_POWER`
   (26) in its input loop — like it already intercepts volume (24/25) — and forwards
   a `power-key` command to the arbiter instead of the guest. The arbiter toggles
   `SetDisplayPower` (task 78) and tracks `panel_on`. EventHub reads the power key
   even with the panel off, so this always wakes.
2. **Arbiter owns screen state under ART-off.** A `WANDR_NO_ART=1` env (set by
   `run-hybrid-stack --no-art`) tells the arbiter to (a) **gate the sysprop screen
   poller** (`spawn_screen_poller` — its source is stale/dead), and (b) drive
   `Event::ScreenState` from its own `panel_on` (power-key) instead. Under ART-up,
   keep the existing poller.
3. **Force panel ON at startup** under `--no-art` (arbiter sets `SetDisplayPower{on}`
   at boot / `run-hybrid-stack` sends a `screen-on` after the framework stop).
4. (later) idle dim/off policy owned by the arbiter (a real screen-off timeout that
   blanks via setPowerMode), replacing PMS's role fully.

## What shipped (implementation)
- `runtime/wandr-host/src/{standalone.rs,audio_policy_impl.rs}` — intercept
  `KEYCODE_POWER` (26) → `forward_power_key()` sends `power-key <pid>` to the arbiter
  (mirror of the volume-key intercept). [committed d6a01360]
- `runtime/wandr-arbiter/wandr-arbiter-power/src/lib.rs` — `panel_on` field +
  `power-key` (toggle) / `panel <on|off>` (explicit) verbs + `set_panel_on()` which
  requests `SetDisplayPower` AND emits `Event::ScreenState` (so doze grace + keyguard
  auto-lock react exactly as to a real power transition). Proximity uncover now
  restores to `panel_on` (not unconditionally on). +3 unit tests.
- `runtime/wandr-arbiter/wandr-screen/` — NEW workspace member: `wandr-screen on|off`
  calls `wandr_hal_display::set_display_power`. Runs as a separate process so the
  arbiter can launch it via `wandr-launch` (uid system).
- `runtime/wandr-arbiter/wandr-arbiter-bin/src/main.rs` — `no_art()` + `sibling_bin()`
  + `apply_display_power()` (ART-up: inline hal; ART-off: `wandr-launch wandr-screen`);
  `Effect::SetDisplayPower` routes through it; `spawn_screen_poller` gated under
  `WANDR_NO_ART` + boot force-on; `power-key`/`panel` added to the CLI allowlist.
- `tools/scripts/run-hybrid-stack.sh` — pushes `wandr-launch` + `wandr-screen`;
  `--no-art` sets `WANDR_NO_ART=1` on the `--daemon` start.
- Reuse: `wandr_hal_display::set_display_power` (task 78), `wandr-launch` (task 83),
  `Effect::SetDisplayPower`, the volume-key intercept pattern.

## Verification (device, `--no-art`)
- Panel comes up ON and stays on; no spurious auto-lock from the stale sysprop.
- Press power → screen off; press again → screen on (no wedge, no adb needed).
- Touch works after a power-cycle of the screen.

## Related
`[[project_art_shutdown]]`, task 78 (`[[project_proximity_screen_off]]`), task 80
(standalone input). Key routing (volume 6×) split to task 82.
