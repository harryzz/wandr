# Task 79 — touch suppression during proximity blank (task-78 follow-on)

> Status: ✅ DONE — device-verified on Pixel 2 XL (2026-06-04). While the panel is
> blanked for proximity (call at the ear), touch input is dropped so a cheek/ear
> can't trigger taps; restored the instant the panel comes back. Touch-only —
> hardware keys (volume) stay live.

## What this delivers

Task 78 turns the panel off during a call when the proximity sensor is covered,
but the touch controller keeps reporting — a cheek could hit mute/hang-up. This
suppresses touch for the duration of the blank, tied to the exact same trigger +
fail-safes (so it can never get stuck on).

## Implementation (reuses existing per-host push plumbing; no new crate, no core change)

- **Host gate** (`wandr-host/src/input.rs`): a process-global `TOUCH_SUPPRESSED`
  atomic + `set_touch_suppressed(on)`; `dispatch_pointer`/`dispatch_pointer_v2`
  (the single touch choke point — standalone + winit loops both route through it)
  return early when set. A first-drop-per-episode log makes a real intercepted
  touch visible. Keys (`dispatch_android_key`) are untouched.
- **Host control verb** (`wandr-host/src/ime_inbound.rs`): parse `input-suppress
  <0|1>` → `input::set_touch_suppressed` (applied directly — the gate is a global
  atomic read on the render thread, no InboundEvent plumbing).
- **Arbiter** (`wandr-arbiter-power/src/lib.rs`): `set_panel_blanked(blank, ctx)`
  folds the panel-power effect + the suppress fan so they move together — requests
  `Effect::SetDisplayPower{on:!blank}` and fans `input-suppress <0|1>` to **every**
  tracked host (`apps_snapshot()`; a cheek can land on chrome too). Used by the
  `ProximityChanged` blank path and `ensure_unblanked`, so all three task-78
  fail-safes (far reading / call-end / call-host death) now lift suppression too.

## Verification

- Unit (10 power tests, host): a blank emits `SetDisplayPower{on:false}` **and**
  `input-suppress 1` to each tracked host; unblank + each fail-safe emit
  `input-suppress 0`.
- Device (self-driven via the `report-sensor` sim verb during an active call):
  - cover → `panel OFF + touch suppressed` → `input-suppress` fanned to all 6
    running hosts (each logged `touch SUPPRESSED`);
  - injected `adb shell input tap` while suppressed → host logged `dropped touch
    while suppressed` (the tap reached the input channel and the gate dropped it —
    proving suppression, not the OS, stops it);
  - uncover → `panel ON + touch resumed` on all hosts; post-uncover tap dispatched
    normally;
  - fail-safe: `audio-call-end` while covered → `panel ON + touch resumed` + sensor
    disabled (battery).

## Out of scope (follow-ons)

- Stylus/hover/non-pointer gating (only `dispatch_pointer*` is gated).
- Host-side watchdog to auto-clear a stuck suppression if a clear-push is lost
  (same reliability class as the existing `doze`/geometry pushes; the arbiter
  guarantees a `0` fan on every unblank via the fail-safes).
- Disabling the touch device at the InputFlinger/kernel level (the dispatch-gate
  is sufficient and process-scoped).
