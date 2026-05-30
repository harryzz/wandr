---
name: feedback_no_art_layer_dependencies
description: "Don't design wart features to depend on ART-layer infrastructure (system_server, WindowManager, ActivityManager, the launcher) — it's being removed. Use HAL/binder signals that survive post-ART."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 5c0eb8cc-cdbc-4cfe-b6cf-7d5eb0c39607
---

The project's north star is to **drop ART and everything that depends on
it** — `system_server`, `ActivityManager`, `WindowManager`, and very
likely the launcher/SystemUI too (anything that doesn't comply with the
wart plan). So when wiring a runtime feature, do NOT build it on top of
an ART-layer signal that won't exist post-ART.

**Concrete trap (task 43, 2026-05-29):** for standalone screen
orientation I first reached for `dumpsys window mCurrentRotation` /
`settings get system user_rotation`. Both are ART/activity-driven —
`mCurrentRotation` is computed by WindowManager from the *foreground
activity's* orientation policy, and with the launcher
(`SCREEN_ORIENTATION_NOSENSOR`) as the source it stays `ROTATION_0` no
matter how the device is physically held. When wart owns the screen
(SystemUI + launcher force-stopped) there is no Activity driving
rotation at all. The correct, ART-independent source is the **raw
accelerometer read directly via our rsbinder sensors HAL** (task 20,
`sensors_impl.rs` — `list_sensors` / `enable` / `poll_latest`), which
talks to the sensor service at the HAL layer and survives post-ART.

**Why:** features built on `system_server` break the moment ART is
removed; the whole point of the runtime is to not need it.

**How to apply:** prefer binder-to-HAL / kernel signals (sensors HAL,
SurfaceFlinger, InputFlinger, sysfs) over `cmd`/`dumpsys`/`settings`
shell-outs to `system_server`. If the only available signal is
ART-layer, treat that as a design smell and flag it. We already have
sensors, SurfaceFlinger, and InputFlinger reachable via rsbinder — reuse
them. See [[project_app_lifecycle_and_packaging]] (SF + InputFlinger
kept; APK/ART ecosystem out of scope) and
[[project_boot_model_libgui_build]].

**Clarification (user, 2026-05-29):** "native" = C++ / HAL / binder, NOT
Java/ART. Native Android libraries and services are fine to KEEP and
REUSE — the rule is "no ART/Java deps," not "avoid all Android
services." And before computing a derived value ourselves from raw data,
**check whether a native service/sensor already reports the computed
value**. For orientation specifically: the sensor HAL often exposes
`SENSOR_TYPE_DEVICE_ORIENTATION` (type 27) — a low-power on-change sensor
that reports screen rotation directly as 0/1/2/3 (0°/90°/180°/270°), so
the platform's auto-rotate doesn't have to run the fusion math on the
AP. If present (`dumpsys sensorservice` / our `list_sensors`), enable +
poll it via the rsbinder sensorservice path and use its value directly —
no accel→rotation math, no ART. Only fall back to deriving rotation from
raw accelerometer if the device lacks the orientation sensor.
