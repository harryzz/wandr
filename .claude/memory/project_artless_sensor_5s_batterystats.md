---
name: project_artless_sensor_5s_batterystats
description: ✅FIXED+device-verified — ~5s --no-art sensor/proximity lag was absent batterystats service → BatteryService blocking getService; fix = batterystats stub in wart-framework-shim
metadata: 
  node_type: memory
  type: project
  originSessionId: 0469217c-e18c-466d-a654-cb7321915922
---

**The ~5s `--no-art` proximity (in-call) + sensor-enable lag is `batterystats`.**
Observed: `wart-hal-sensors TIMING enable(6) ... enableSensor()=5.017373937s` (the
*enable* call blocks ~5.0s, not delivery; 5.0s = libbinder blocking `getService`
doing `sleep(1)`×5). NOT the wake-up/ACK path (user correctly ruled that out: the
event registers at the HAL immediately, it's the host-side enable/note that stalls).

**Source path (vendored, read-only):**
`aosp-frameworks-native/services/sensorservice/SensorService.cpp` `enable()` :2113
→ `BatteryService::enableSensor` → `BatteryService::checkService()`
(`sensorservice/BatteryService.cpp:85-94`) which does the **blocking**
`defaultServiceManager()->getService(String16("batterystats"))`. Under `--no-art`
`batterystats` (a system_server service) is gone → `getService` polls ~5s, returns
null, and because `mBatteryStatService` stays null it **re-blocks on every call**:
- `enable()` :2113 → 5s on every sensor enable
- `disable` :1905/:2213 → 5s
- **every wake-up event**: `SensorEventConnection.cpp:394`
  `BatteryService::noteWakeupSensorEvent` → `checkService` → 5s (proximity is a
  wake-up sensor → per-transition delivery lag).
Accel/orientation pay the 5s once at bringup so it's invisible; proximity is
enabled on-demand at call-start + notes every event → the visible symptom.

**Not just proximity — whole-pipeline freeze.** `noteWakeupSensorEvent` (:394) runs
inside `SensorEventConnection::sendEvents`, which `threadLoop` calls **while holding
`mLock`** (taken at threadLoop:1154), BEFORE the actual `SensorEventQueue::write()`
(:405). So the 5s block freezes the entire distribution fan-out → when proximity (any
wake-up sensor) fires during a call, auto-brightness (light) + auto-rotation (accel)
ALSO freeze 5s ("auto-brightness stops working in a call" = same bug). `mLock` is also
held, so concurrent enable/disable/createSensorEventConnection from other clients stall
behind an in-flight wake event. One batterystats stub fixes ALL of it.
Under ART system_server registers `batterystats` so `getService` resolves
instantly (this is the entire ART vs --no-art asymmetry, same binary).

**FIX ✅device-verified 2026-06-07 (user: "proximity instant now"):** deployed shim-only
(push binary + kill/relaunch wart-framework-shim via wart-launch; sensorservice left
running unchanged — BatteryService re-getServices until success then caches, so no
sensorservice restart needed). Register a `batterystats` GenericStub in
`runtime/wart-framework-shim/cpp/wart_framework_shim.cpp` (descriptor
`com.android.internal.app.IBatteryStats`). Only the NAME needs to resolve —
`checkService` then caches the binder non-null and never blocks again; the
`noteStart/Stop/WakeupSensor` transactions are fire-and-forget (void return
ignored) so the benign reply is fine. Build on a-03, deploy, no device churn.

**Same-class latent (NOT yet hit by proximity):** `AppOpsManager::getService()`
(`aosp-frameworks-native/libs/permission/AppOpsManager.cpp:50-76`) uses
non-blocking `checkService` but its OWN `sleep(1)` loop up to **10s** when `appops`
absent — bites any sensor WITH a required appop + audio-record (`checkAudioOpNoThrow`).
Proximity has no required appop (`perm: n/a`) so `noteOpIfRequired`/`checkCanAccessSensor`
early-out → AppOps is NOT the proximity culprit. Add `appops` to the shim only if an
appop-gated sensor or audio-record shows the lag (and verify the GenericStub int reply
is parcel-correct for IAppOpsService's richer methods first). See [[project_artless_audio]].

Extends [[project_arbiter_sensors]], [[project_proximity_screen_off]],
[[project_art_shutdown]]. Discipline note: this was found by READING the source
path end-to-end (CLAUDE.md rule #1) after days of patch-and-cycle — see
[[feedback_read_source_first]].
