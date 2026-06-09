---
name: project-artless-sensors
description: "ART-off sensors (task 85): why standalone sensorservice fails + the lean C++ HIDL-shim path to revive sensors/auto-rotation under --no-art"
metadata: 
  node_type: memory
  type: project
  originSessionId: 4b71db65-e2f9-402f-93dd-767a35eff7ba
---

**✅ SOLVED + device-verified (auto-rotation, 2026-06-04).** FIX = thin C++ HIDL shim
`libwandr_sensors_hal.so` (`runtime/wandr-sensors/cpp/`, reads
`android.hardware.sensors@1.0::ISensors` directly — the HAL survives ART-off; HIDL=hwbinder
so this hop MUST be C++, rsbinder can't) + Rust daemon `wandr-sensors` (dlopens the shim
like wandr-host loads libsf_surface; prefers HAL-fused `DEVICE_ORIENTATION` type 27 →
report-orientation to arbiter; accel+OrientationTracker fallback, 5 unit tests). **No
Fusion.cpp port needed**: the probe found the HAL exposes ALL fused sensors (27 Device
Orientation, 11 Rotation Vector, 9 Gravity, …) — this Qualcomm device computes them on the
SSC/SLPI sensor core, not the framework. Verified: physically rotating the phone
auto-rotates UI+chrome+touch with the Java framework stopped (daemon logged
report-orientation 0↔3). Wired into run-hybrid-stack --no-art (launched via wandr-launch =
uid system, after the arbiter). a-03 NEW-MODULE build gotcha: `m` dies in kati (LineageOS
dexpreopt `$(error)`) but soong shards regen first → direct-ninja the soong intermediate
(`out/soong/.intermediates/external/wandr-sensors/<tgt>/android_arm64_armv8-a*/...`) via the
combined ninja. PROXIMITY also DONE+device-verified (2026-06-04): wandr-sensors enables proximity HAL
sensor (type 8), pushes descriptor (max_range from HAL SensorInfo) via NEW arbiter verb
`report-sensor-descriptor <kind> <max_range> <resolution>` (the in-process sensor_driver
seeds this at enumerate normally, dead under ART-off), feeds readings via `report-sensor
proximity <x>`. Arbiter classifies near/far → wandr-arbiter-power (gated CommsActive) blanks
panel on near + restores on far + touch-suppress (task 79). Verified: cover→blank+suppress,
uncover→panel ON+resume (raw HAL 0 near / 5.0 far). C++ shim exposes wandr_sensors_max_range/
_resolution. Daemon launch gotcha: `setsid` wrapper made it die right after startup — use
plain `(wandr-launch wandr-sensors &)`. Follow-on: on-demand ref-counted enable (arbiter
SetSensor→wandr-sensors) vs always-on; light/other sensors.

(Diagnosis record below — why the simpler paths failed.)

Under `--no-art` **no sensors work** — auto-rotation, proximity-screen-off (task 78),
light, etc. all route through `android.frameworks.sensorservice.ISensorManager`, served
by the C++ `SensorService` which is **instantiated inside `system_server`** and dies
when ART stops (`service list | grep sensor` empty under `--no-art`). The sensor **HAL
survives**: `android.hardware.sensors@1.0::ISensors` (HIDL), processes `sensors.qcom` +
`android.hardware.sensors@1.0-service`. So the host's orientation poll
(`SENSOR_TYPE_DEVICE_ORIENTATION` 27, sensors_impl.rs / standalone.rs:947) finds nothing
→ no auto-rotation. Task-84 input rects ALREADY rotate correctly once an orientation is
set (verified via manual `wandr-arbiter report-orientation`); the gap is the sensor TRIGGER.

**Path A (run standalone sensorservice) FAILS — tried 2026-06-04.** SensorService IS
C++ with a standalone `cc_binary` (`/system/bin/sensorservice` on device, main =
`SensorService::publishAndJoinThreadPool()`), so the input-style path-A looked clean.
But it HANGS in init + never registers: `SensorService::isAutomotive()` does an infinite
`waitForService("package_native")` (SensorService.cpp:144) — native PackageManagerService,
in system_server, gone. Beyond that SensorService is heavily coupled to the system_server
permission layer (UidPolicy→ActivityManager uid observer, SensorPrivacyPolicy, AppOps,
package/permission checks) — all multi-app-sandbox machinery wandr doesn't have. NOT a
clean drop-in like InputManager was.

**Pure-Rust port also blocked:** sensors arrive over **HIDL** (`@1.0::ISensors`,
hwbinder transport); `rsbinder` speaks AIDL/binder only → Rust can't reach the HAL on
this device. (Device-specific: a newer device with the AIDL sensors HAL could be
pure-Rust.) Sizes: SensorService.cpp 2856 lines, whole subsystem 8550 — but most is
per-app connection mgmt / AppOps / privacy / direct channels, irrelevant to wandr.

**DECISION = lean C++ HIDL shim → Rust** (NOT full sensorservice, NOT a Rust port). A
small shim (HidlSensorHalWrapper pattern, few hundred lines, like libsf_surface is for
libgui) opens `ISensors@1.0`, enables wanted sensors, feeds raw events to Rust over a
thin FFI. Then base sensors (accel/proximity/light/gyro/mag) flow to the existing Rust
consumers (`wandr-hal-sensors`/`wandr-arbiter-sensors`, reshaped to consume the shim not
`ISensorManager`); device-orientation = ~15 lines of Rust (gravity vector → 0/1/2/3 +
debounce) → `report-orientation` → auto-rotation; fused sensors (rotation vector/gravity)
port the specific Fusion.cpp math (~565 / 80–160 per sensor) only if a consumer needs it.
Skips the permission-layer mess; HIDL contained in one shim. Build on a-03.

Scoped in `tasks/85-artless-sensors.md`. See [[feedback_no_art_layer_dependencies]],
[[project_arbiter_sensors]] (task 77 — the Rust consumer), [[project_proximity_screen_off]]
(task 78 — also dead under --no-art), [[project_standalone_input]]/task 80 (path A for
input — the template that did NOT transfer), [[project_pathA_inputflinger]], task 84.
