# Task 89 — ART-off sensors reliability + panel_on sync (auto-brightness follow-on)

> Status: 🔲 SCOPED. Follow-on to task 85 (ART-off sensors) + task 86 (ART-off
> auto-brightness) + task 81 (ART-off display power). Opened after a user doubt
> "auto-brightness doesn't work" → live verification (2026-06-05) proved the
> **mechanism is correct** but the **experience is flaky** for the reasons below.
> Detail: `[[project_artless_autobrightness]]`, `[[project_artless_sensors]]`.

## What we verified (so this isn't re-litigated)

Auto-brightness **works** — device-confirmed by a cover/uncover light-sensor test
while watching `wart-arbiter sensor-state` (lux) + `/sys/class/leds/lcd-backlight/
brightness`. Across 4 cover/uncover cycles: **covered → lux 0 → backlight ~10–16;
uncovered → lux 5 → backlight ~63–67.** The curve + applier are correct. What
undermines the *experience* are the reliability bugs below.

## Issue 1 (primary) — `wart-sensors` aborts on HAL `DEAD_OBJECT`, no auto-restart — ✅ DONE + device-verified (2026-06-05)

**Verified:** `pkill` the sensors HAL → `wart-sensors` **survived** (same pid, **no new
tombstone** vs the old SIGABRT) + logged `transport error (-2) → reconnected, 3 sensors
re-enabled` + the light feed recovered (6.488 lux). All three sub-fixes below landed:
the C++ shim checks every HIDL `Return<>` (`wart_sensors_hal.cpp`, a-03 rebuilt),
`SensorHal::reopen()` + a `reconnect()` loop re-enable the tracked sensors
(`hal.rs`/`main.rs`), and `run-hybrid-stack.sh` adds a respawn backstop.


**Symptom:** sensors (light → auto-brightness, proximity → screen-off, accel →
auto-rotation) all silently stop; `wart-arbiter sensor-state` shows
`light[holders=0 (no reading)]`; backlight frozen at its last value.

**Root cause (tombstones 27/28/29/40/41, 2026-06-05):** `wart-sensors`
**`SIGABRT`** — `Abort message: 'Failed HIDL return status not checked … 
Status(EX_TRANSACTION_FAILED): DEAD_OBJECT'`, backtrace `wart_sensors_poll+128`
(`libwart_sensors_hal.so`) → `libhidlbase return_status::~return_status` →
`__android_log_default_aborter` → `abort`. I.e. when the sensors HAL
(`android.hardware.sensors@1.0-service`) connection drops — HAL churn across a
`--restore-art`→`--no-art` cycle, or SensorService re-grabbing it — the C++ HIDL
shim's **unchecked `Return<>`** triggers `libhidlbase`'s abort-in-destructor. The
daemon dies and **nothing respawns it**.

**Fix:**
1. **C++ shim (`cpp/`, built on a-03):** in `wart_sensors_poll` (and any other
   HAL call), **check the HIDL `Return<>` status** (`.isOk()` / `.description()`)
   and return an error code to the Rust caller on `DEAD_OBJECT`/transport failure
   instead of letting the `Return` destructor abort.
2. **Rust (`runtime/wart-sensors`):** on a poll error, **re-acquire the HAL**
   (re-`getService`, re-subscribe the enabled sensors) with backoff, rather than
   exiting. (Mirror the host's `media.aaudio` re-resolve-on-dead pattern from
   `[[project-artless-audio]]`.)
3. **Belt-and-suspenders (`run-hybrid-stack.sh`):** supervise/respawn
   `wart-sensors` if it exits under `--no-art` (today it's a one-shot
   `spawn_detached`; add a small respawn loop or a watchdog).

## Issue 2 — `panel_on` not synced to the power-button wake — ⛔ REASSESSED: NOT a real bug (2026-06-05)

**Investigated on-device and could not reproduce a desync.** The power-key path works
end to end: `wart-arbiter power-key` toggles the panel and applies the ambient
backlight (`0 → 91 → 0`), and a physical power press drove `wart-screen:
set_display_power(true)`. The earlier "the key never reaches the arbiter" was a
**logging artifact** — the arbiter's own `log::info` ("arbiter: panel ON …") does not
land in `/data/local/tmp/wart-arbiter.log` (only the `wart-screen:` applier lines do),
so a grep for it returned 0 and misled me. The user's original "had to send a command
to get sensors working" was **Issue 1** (dead `wart-sensors` → no light feed), now
fixed. The one *real* sub-finding here is minor and separate: **the arbiter's
`log::info` is not captured in its logfile** (a debugging annoyance, broader than this
task — `[[project_artless_autobrightness]]` already notes "arbiter log::info → STDERR").
**Recommendation: close Issue 2** pending a clean real-world re-confirm (idle 60 s →
screen off → press power → wakes + brightness tracks + 2nd press off).

### (original hypothesis, kept for context)

**Symptom (user-reported):** pressing the power button lights the panel, but the
light sensor stays disabled until the arbiter's "power on display" command runs
~1 s later — i.e. the arbiter's `panel_on` and the real hardware panel are out of
sync, so the (panel-on-ref-counted) light sensor isn't re-enabled on a power-key
wake alone.

**Fix:** the power-key path (host intercepts `KEYCODE_POWER` → arbiter, task 81)
must flip `panel_on` → on wake, `wart-arbiter-power` re-acquires the light enable
(and applies the current auto value) in the same step that powers the panel. Make
the power-key wake and the `SetDisplayPower(on)` + `panel_on=true` atomic so a
button press alone fully restores auto-brightness. Also surface a clean panel
state query (today `wart-arbiter power-state` = "unknown command"; the stale
`debug.tracing.screen_state` sysprop reads `1` while `backlight=0`).

## Issue 3 (secondary) — screen idles off too fast / coarse sensor — ⛔ REASSESSED: mostly a non-issue

`DEFAULT_SCREEN_OFF_TIMEOUT_MS = 60_000` (60 s) and it resets on real input
(`user-activity`, poked by wart-inputflinger) — a reasonable default, not aggressive.
What looked aggressive in testing was the test not generating input. The light sensor's
coarseness (0/5 indoors) is hardware. **No action needed** beyond leaving the
`screen-timeout <ms|off>` knob available. (Original notes below.)

- The panel blanks aggressively (observed: backlight `0` for ~9 s mid-test while
  lit) → no auto-brightness while off (correct gating, but the short timeout makes
  it *look* dead). Verify/tune the `--no-art` `screen-timeout` (`wart-arbiter
  screen-timeout`) — likely just needs a saner default, or this is intended.
- The light sensor is **coarse** (reports only `0.0` / `5.0` indoors) — hardware
  quantization, not fixable; consider a light **smoothing/hysteresis** only if it
  causes visible flicker (NB: ALS is on-change, don't add EMA lag — see the task-86
  "NO EMA" note in `[[project_artless_autobrightness]]`).

## Files

- `runtime/wart-sensors/cpp/*` (the `wart_sensors_poll` HIDL shim → check `Return`)
  — built on a-03 (`m libwart_sensors_hal` / ninja the soong intermediate).
- `runtime/wart-sensors/src/*` (Rust: re-acquire HAL on poll error + reconnect loop).
- `runtime/wart-arbiter/wart-arbiter-power/*` (panel_on ↔ power-key wake; light
  enable on wake; panel-state query verb).
- `tools/scripts/run-hybrid-stack.sh` (respawn `wart-sensors` under `--no-art`).

## Verification

1. **Crash resilience:** run `wart-sensors`, then kill/restart the sensors HAL
   (or do a `--restore-art`→`--no-art` cycle) → `wart-sensors` must **survive /
   re-acquire** (no tombstone, `sensor-state` light recovers a reading).
2. **Panel sync:** screen idle off → press power button ONLY → light sensor
   re-enables + backlight applies the auto value (no separate arbiter command).
3. **Tracking (regression):** the cover/uncover monitor — covered → backlight
   ~10–16, uncovered → ~63–67 (the proven-good baseline).

## References

- `[[project_artless_autobrightness]]` (the reliability-bug note + curve),
  `[[project_artless_sensors]]` (the shared `wart-hal-sensors` / wart-sensors +
  HIDL shim), `[[project_art_shutdown]]`, `[[project-artless-audio]]`
  (re-resolve-on-dead-handle pattern to mirror), tasks 81/85/86.
