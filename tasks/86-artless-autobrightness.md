# Task 86 — ART-off auto-brightness (light sensor → backlight)

> Status: 🔲 scoped (2026-06-04). Follow-on to task 85 (ART-off sensors) + task 81
> (ART-off display power). With the Java framework stopped there is no DisplayManager
> /automatic-brightness controller, so the screen sits at a fixed backlight (task 85
> set a constant level so it's visible). This task makes brightness track the ambient
> light sensor — the third `wart-arbiter-sensors` consumer the task-77 design
> anticipated (`light→auto-brightness`), after proximity (task 78) and accel→orientation.

## The pieces (mostly already in place)

- **Sensor source:** the ambient light sensor is already there — the wart_sensors
  probe enumerated `handle=7 type=5 "TMx490x ALS"`. `wart-sensors` enables it (type 5)
  and feeds the arbiter exactly like proximity: push the HAL descriptor once
  (`report-sensor-descriptor light <max_range> <resolution>`) + each reading
  (`report-sensor light <lux>`). `SensorKind::Light` already exists in the arbiter.
- **Policy (new):** an auto-brightness consumer (in `wart-arbiter-sensors` or a small
  new module) maps lux → a backlight level via a curve, with **smoothing + hysteresis**
  (lux is noisy + auto-brightness must not flicker), and respects: the proximity blank
  (task 78 — don't fight it; brightness applies only while `panel_on` and not blanked),
  a manual/override level, and a min floor (never fully dark by ambient alone). Derive
  the curve from first principles where possible (panel `max_brightness`, sensor
  `max_range`) — no magic numbers (`[[feedback_no_hardcoding]]`).
- **Applier (the binder-vs-sysfs question):** see below. Reuse the task-85 backlight
  setter (`apply_backlight` in `wart-arbiter-bin`), generalized to take a level.

## Brightness mechanism: sysfs vs binder (investigated 2026-06-04)

Today task 85 sets brightness via **sysfs** (`/sys/class/leds/lcd-backlight/brightness`,
root write). Is there a binder way? Both exist, with caveats:

1. **`ISurfaceComposer::setDisplayBrightness(displayToken, DisplayBrightness{...})`**
   (AIDL) — the modern path; `DisplayBrightness` (0–1 float, `-1` = backlight off, nits)
   is already codegen'd in **`wart-hal-display`** (`sf_bindings.rs`), and SurfaceFlinger
   survives ART-off (we already call `setPowerMode` there, task 78). **TESTED on device
   2026-06-04** (added `wart-hal-display::set_display_brightness` + `wart-screen
   brightness <0-1>`): the call is fully **reachable** as uid system (no permission
   issue) but returns **`IllegalState`** — the taimen panel does NOT support
   composer-driven brightness (the HWC reports it unsupported); brightness routes
   through the Lights HAL / sysfs here. So on THIS device SF brightness is a dead end;
   on a newer device whose HWC supports it, `set_display_brightness` is the clean path
   (it's now wired + ready). sysfs unchanged by the call (confirmed 149→149).
2. **Lights HAL** — the device exposes `android.hardware.light@2.0::ILight/default`
   (vendor HAL, pid 1130, **survives ART-off**), and the framework set brightness via it
   (LIGHT_ID_BACKLIGHT) — which itself writes the **same sysfs node**. wart has an
   `ILights` path (task 17, `lights_impl.rs`) but it targets the AIDL lights interface;
   the device's is **HIDL @2.0**, which rsbinder can't speak → would need a small C++
   shim (like `libwart_sensors_hal`).
3. **sysfs** (current) — the lowest layer the Lights HAL ultimately writes on this
   device. Raw (0–`max_brightness`, no nits/curve), root, device-specific node, but
   simple and known-working.

**Decision:** keep **sysfs** as the applier (it's exactly what the Lights HAL writes on
taimen, so it's not a hack here — it's the same endpoint), but abstract it behind a
small `set_brightness(level)` so the backend is swappable. **First try** SF
`setDisplayBrightness` on-device (free — already wired in wart-hal-display); if it drives
the panel, prefer it (proper nits/curve, no hardcoded node) and keep sysfs as the
fallback. The Lights-HAL HIDL shim is a last resort (only if both fail on some device).
Node + curve params env-overridable, one named source.

## Steps
1. `wart-sensors`: enable light (type 5), push descriptor + readings to the arbiter
   (mirror the proximity wiring from commit 3946b72e). [device]
2. Arbiter: lux → brightness curve + smoothing/hysteresis consumer; emit a
   `SetBrightness`-style effect (or reuse `apply_backlight`), gated on `panel_on` +
   not-blanked + no manual override. Unit-test the curve + hysteresis. [desktop]
3. Applier: generalize `apply_backlight(level)`; probe SF `setDisplayBrightness` vs
   sysfs on-device, pick the working one (sysfs fallback). [device]
4. Verify under `--no-art`: cover the light sensor / shine a light → backlight tracks
   (smoothly, no flicker); manual level + proximity blank still win.

## Done when
Under `--no-art`, the screen brightness follows ambient light (smoothly, no flicker),
without fighting proximity screen-off (task 78) or a manual override.

## Related
Task 85 (`[[project_artless_sensors]]` — the sensor daemon + arbiter feed pattern to
mirror), task 81 (`[[project_art_shutdown]]`/display power — the panel `panel_on` +
backlight setter), task 78 (proximity blank — must coexist), task 77
(`[[project_arbiter_sensors]]` — `light→auto-brightness` was the anticipated 3rd
consumer), task 17 (lights HAL), `[[feedback_no_hardcoding]]`.
