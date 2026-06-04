---
name: project-artless-autobrightness
description: "ART-off auto-brightness (task 86): light sensor → backlight in wart-arbiter-power; why no SensorAcquire + sysfs-only applier"
metadata: 
  node_type: memory
  type: project
  originSessionId: 023c2492-85e0-4052-bd04-4dc23f02fd88
---

**✅ DONE + device-verified (task 86, 2026-06-04).** Under `--no-art` there is no
DisplayManager auto-brightness, so the ambient light sensor → backlight policy lives
in **`wart-arbiter-power`** (it already owns `panel_on`/`blanked` + is the
display-power/backlight authority — single source of truth, no duplicated screen
state). The 3rd `wart-arbiter-sensors` consumer the task-77 design anticipated, after
proximity (task 78) + accel→orientation (task 85).

**Pipeline:** `wart-sensors` enables light (android type 5; `hal.rs TYPE_LIGHT`),
pushes `report-sensor-descriptor light <max_range> <res>` once (taimen ALS
max_range=32767 lux) + streams `report-sensor light <lux>` (de-duped, debug-logged) →
arbiter `Event::SensorReading{Light}` → power module `on_light` → new
`Effect::SetBacklight{level:f32}` (normalized 0–1 fraction) → binary `apply_backlight`
maps fraction→raw via cached `max_brightness` (255) → sysfs
`/sys/class/leds/lcd-backlight/brightness`. No C++/shim change (the HIDL shim already
polls all enabled sensors + exposes max_range/resolution).

**Policy (no-hardcoding — named consts in wart-arbiter-power):** curve
`lux_to_fraction = clamp(MIN_FRACTION, 1.0, log10(lux+1)/log10(full_scale_lux+1))` —
log (perceptual). **CEILING = `FULL_SCALE_LUX=600` (a PERCEPTUAL full-brightness
reference, NOT the sensor's max_range).** First cut used the ALS max_range (32767 =
direct sunlight) as the ceiling → the whole indoor lux band (0–500) squeezed into the
bottom of the curve → cover/uncover barely moved (user-caught: "no big visual
effect"). 600 lux = bright-indoor/overcast spreads indoor lux across the full
backlight: covered ~0.4 lux→node 15, lit room ~250 lux→node 219 (device-measured).
`MIN_FRACTION=0.04` legibility floor; `MIN_STEP=0.03` hysteresis dead-band.
**NO EMA / cross-sample smoothing** — the ALS is ON-CHANGE (one HAL event per settled
lux), so an EMA only ever steps ~20% toward target then FREEZES (2nd user-caught bug:
lux 0.4 stuck at node 76 instead of 15). Each reading maps STRAIGHT to its target
(`on_light` snaps `light_frac`); MIN_STEP is the sole anti-flicker filter (fine since
on-change is already discrete). Gated on `panel_on && !blanked &&
manual_frac.is_none()`; re-asserts on wake (`set_panel_on(true)`) + uncover
(`ensure_unblanked`) since `SetDisplayPower(on)` only sets the boot default
(`DEFAULT_ON_FRACTION=0.6`). Verbs: `brightness <auto|0.0..1.0>` (manual override) +
`brightness-scale <lux>` (LIVE-tune the ceiling, recomputes from last lux + reapplies
immediately — dial in the feel with no rebuild). Both in the CLI passthrough allowlist.

**TWO DELIBERATE DEVIATIONS from the task scope (both correctness):**
1. **NO `Event::SensorAcquire{Light}`.** The plan said acquire it (battery contract,
   like power acquires Proximity on CommsActive). But: under ART-up the in-process
   `sensor_driver` only emits `SensorReading` for sensors enabled via a `SetSensor`
   effect (gated by `ENABLED`), so NOT acquiring = no light readings = auto-brightness
   silent = the framework's own auto-brightness is never fought. Under `--no-art`
   `wart-sensors` force-enables the ALS directly (the `sensor_driver` is dead — it uses
   the framework SensorManager), so the acquire would be vestigial anyway. Acquiring
   would have powered the ALS + fought the framework under ART-up = a regression.
2. **sysfs-only applier (NOT inline SF `setDisplayBrightness` try-first).** The arbiter
   runs as PLAIN ROOT and a bare-root SurfaceFlinger call HANGS on the permission check
   under `--no-art` (same reason `apply_display_power` shells to `wart-launch
   wart-screen` as uid system). Per-reading `wart-launch` spawns would be far too heavy
   for a brightness stream. And on taimen SF `setDisplayBrightness` is `IllegalState`
   regardless (HWC unsupported, task-86 device-confirmed). So the hot path writes sysfs
   (exactly what the Lights HAL writes here — same endpoint, not a hack). SF stays
   reachable/tested via `wart-screen brightness <f>` (uid system) for a future device
   whose HWC supports it. `apply_backlight` is also gated to `no_art()` (ART-up =
   DisplayManager owns the backlight).

**Verify / live-log (no rebuild):** poll `wart-arbiter sensor-state` (caches last
light `x=<lux>` in the Store) + `cat /sys/class/leds/lcd-backlight/brightness` in a
loop → live lux↔backlight. Device-measured (full-scale 600, snap): covered ~0.4
lux→node 15, ~100 lux→183, ~246 lux→219 — strong, immediate swing. Manual:
`wart-arbiter brightness 0.9`→230 / `0.1`→25 / inject-while-manual stays put / `auto`
re-tracks / `9`→ERR. Inject without a device: `wart-arbiter report-sensor light <lux>`
(interleaves with REAL ambient so injected values get pulled back). NOTE: the ALS is
on-change, so when lux is STEADY no new event flows — `sensor-state` shows the last
value + backlight holds (correct, not stuck). Arbiter `log::info!` goes to STDERR, NOT
the spawn_detached logfile (arbiter.log only shows child stdout like `wart-screen:
set_display_power`). 35 arbiter unit tests green. Subjective smooth/no-flicker = user's
visual call ([[feedback_visual_verification]]).

**Screen-off timeout + auto-lock (same task-86 follow-on, PowerManager role).** Under
`--no-art` there's no PowerManagerService, so the screen never auto-slept and keyguard
(which auto-locks on screen-off) never fired. Built the AOSP-faithful split: the input
dispatcher pokes activity, the arbiter decides the timeout — mirroring
`InputDispatcher::pokeUserActivity → PowerManagerService.userActivity`. NOT the host
(each host sees only its own window; the dispatcher sees all). Wiring:
`wart-inputflinger`'s `pokeUserActivity` policy hook (the exact AOSP spot; was an empty
override) → `arbiter_send("user-activity")` throttled ~1/s; `wart-arbiter-power` tracks
`last_activity`, a 5 s `Event::IdleTick` ticker (binary, `--no-art` only) checks idle vs
`screen_off_timeout` (default 60 s, live `screen-timeout <ms|off>`) → `set_panel_on(false)`
which already cascades to keyguard auto-lock + panel-off + backlight-0. Wake (POWER)
resets the clock. Device-verified: idle→backlight 0 + keyguard up, power-key→restored,
no immediate re-sleep. `wart-inputflinger` rebuilt via NINJA-DIRECT (`m` died in kati
dexpreopt) — see [[reference-a03-ninja-build]]. Also: run-hybrid-stack `--no-art` now
kills `bootanimation` (init restarts it framework-down → it covers the wart UI).

See [[project_artless_sensors]] (task 85 — the wart-sensors+arbiter feed pattern
mirrored here), [[project_proximity_screen_off]] (task 78 — the blank that
auto-brightness must not fight), [[feedback_no_hardcoding]],
[[feedback_no_art_layer_dependencies]].
