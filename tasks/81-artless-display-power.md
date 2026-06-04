# Task 81 — ART-less display power ownership

> Status: 🔲 in progress. Found in human testing of `--no-art` (2026-06-04): the
> device wedged (black screen, power button + touch dead) because **nothing owns
> display power when ART is off**.

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

The wart stack **owns display power** when ART is off: the panel stays on, the power
button wakes/sleeps it, and doze/keyguard react to our own screen state — never to
the dead PMS sysprop. Never wedge.

## Approach (build on task 78's `wart-hal-display` setPowerMode)

1. **Power-key → toggle panel** (the un-wedge). The host intercepts `KEYCODE_POWER`
   (26) in its input loop — like it already intercepts volume (24/25) — and forwards
   a `power-key` command to the arbiter instead of the guest. The arbiter toggles
   `SetDisplayPower` (task 78) and tracks `panel_on`. EventHub reads the power key
   even with the panel off, so this always wakes.
2. **Arbiter owns screen state under ART-off.** A `WART_NO_ART=1` env (set by
   `run-hybrid-stack --no-art`) tells the arbiter to (a) **gate the sysprop screen
   poller** (`spawn_screen_poller` — its source is stale/dead), and (b) drive
   `Event::ScreenState` from its own `panel_on` (power-key) instead. Under ART-up,
   keep the existing poller.
3. **Force panel ON at startup** under `--no-art` (arbiter sets `SetDisplayPower{on}`
   at boot / `run-hybrid-stack` sends a `screen-on` after the framework stop).
4. (later) idle dim/off policy owned by the arbiter (a real screen-off timeout that
   blanks via setPowerMode), replacing PMS's role fully.

## Files
- `runtime/wart-host/src/standalone.rs` (+`lib.rs`) — intercept `KEYCODE_POWER` →
  forward `power-key` (next to the volume-key intercept at the input loop).
- `runtime/wart-arbiter/wart-arbiter-power/src/lib.rs` — `power-key` verb + `panel_on`
  + drive `ScreenState` from it; force-on at boot.
- `runtime/wart-arbiter/wart-arbiter-bin/src/main.rs` — gate `spawn_screen_poller`
  under `WART_NO_ART`; CLI passthrough for `power-key`/`screen-on`.
- `tools/scripts/run-hybrid-stack.sh` — `--no-art` sets `WART_NO_ART=1` + panel-on.
- Reuse: `wart_hal_display::set_display_power` (task 78), the volume-key intercept
  pattern, `Effect::SetDisplayPower`.

## Verification (device, `--no-art`)
- Panel comes up ON and stays on; no spurious auto-lock from the stale sysprop.
- Press power → screen off; press again → screen on (no wedge, no adb needed).
- Touch works after a power-cycle of the screen.

## Related
`[[project_art_shutdown]]`, task 78 (`[[project_proximity_screen_off]]`), task 80
(standalone input). Key routing (volume 6×) split to task 82.
