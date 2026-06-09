# Task 94 — Unify sensors on the native AIDL ISensorManager; delete wandr-sensors

> Status: 🛠️ CODE COMPLETE (pending a-03 build + device verify). **Unblocks task 93**
> coexistence under `--no-art`.

## Implemented (revised — full unification, not a wandr-sensors refactor)

The original plan (below) was to swap `wandr-sensors`' data source from the direct
HAL to a `sensorservice` client. While reading the source we found the arbiter
**already** has a Rust/AIDL sensor path — `wandr-hal-sensors` (task 77) wraps
`android.frameworks.sensorservice.ISensorManager/default` over rsbinder, and
`sensor_driver::spawn()` runs unconditionally — inert under `--no-art` only because
nothing registers that AIDL endpoint. So instead of refactoring `wandr-sensors`, we
**deleted it** and lit up the existing path:

1. **`wandr-sensormanager`** now publishes BOTH ISensorManager transports system_server
   does (`com_android_server_SystemServer.cpp` pattern): HIDL @1.0 (camera EIS, task 93,
   kept) **+ AIDL** `…ISensorManager/default` (`SensorManagerAidl`, same fakeVM, NDK
   binder pool). `runtime/wandr-sensormanager/cpp/{wandr_sensormanager.cpp,Android.bp}`.
2. **Arbiter sensor-driver** consumes the AIDL endpoint under `--no-art`: proximity
   (task 78) + light (task 86) work unchanged; added **DEVICE_ORIENTATION (type 27)**
   always-on → the WM turns each reading into auto-rotation.
   `wandr-arbiter-bin/src/sensor_driver.rs`, `wandr-arbiter-wm/src/lib.rs`,
   `wandr-arbiter-core` (`SensorKind::DeviceOrientation`).
3. **`wandr-hal-sensors`** caches only *successful* service/queue lookups (no `None`
   latch) so a consumer racing the endpoint reconnects.
4. **Deleted `runtime/wandr-sensors/`** (daemon + `libwart_sensors_hal.so` direct-HAL
   shim + accel orientation math). Bringup (`run-hybrid-stack.sh`) now starts
   `/system/bin/sensorservice` + `wandr-sensormanager` BEFORE the zygote/arbiter.

Result: one Rust/AIDL sensor path for ART-up and `--no-art`; guests get sensors under
`--no-art` for free. Plan: `~/.claude/plans/cat-task-state-melodic-rabin.md`.
See `[[project-artless-sensors]]`, `[[project_artless_camera]]`.

---

## Original plan (superseded by the above)

> The camera needs `sensorservice` to own the sensors HAL; wandr-sensors currently
> owns it directly → single-client conflict. Refactor wandr-sensors' data source
> from the HAL to a `sensorservice` client; keep all the policy/arbiter logic.

## Why (the conflict)

The HIDL sensors HAL `android.hardware.sensors@1.0::ISensors` is **single-client**.

- **Task 85 `wandr-sensors`** opens it DIRECTLY: the C++ shim
  `runtime/wandr-sensors/cpp/wandr_sensors_hal.cpp` does `ISensors::getService()` +
  `activate(handle, …)` + `poll(…)` (libwart_sensors_hal.so), and `src/main.rs`
  enables DEVICE_ORIENTATION(27)/accel, proximity(8), light(5), computes
  orientation, and pushes `report-orientation` / `report-sensor` to the arbiter
  (auto-rotation, proximity-screen-off task 78, auto-brightness task 86).
- **Task 93 `sensorservice`** must ALSO own `ISensors` — because the qcom camera
  HAL's EIS needs the gyro via `android.frameworks.sensorservice@1.0::ISensorManager`,
  which only the real `SensorService` provides, and `SensorService` opens the HAL
  itself. (See `[[project_artless_camera]]`.)

Both running = two direct HAL clients = `DEAD_OBJECT` (device-confirmed: starting
`sensorservice` while `wandr-sensors` held the HAL aborted `sensorservice` with
`Abort due to ISensors hidl service failure ... DEAD_OBJECT`). **They cannot
coexist as-is.**

## Decision: sensorservice owns the HAL; wandr-sensors becomes its client

The camera forces `sensorservice` to own the HAL, and `sensorservice` is the
*more* complete sensor solution anyway (fused sensors, the standard client API).
So **wandr-sensors stops touching the HAL** and reads sensors *through* sensorservice.

## Scope (refactor, ~the C++ shim only — Rust policy logic is preserved)

1. **Rewrite the data source** `runtime/wandr-sensors/cpp/wandr_sensors_hal.cpp`:
   replace the `ISensors@1.0` direct path (`getService`/`activate`/`poll`) with a
   **`libsensor` native client** — `android::SensorManager` (`getSensorManager` for
   an opPackageName) → `createEventQueue()` → `enableSensor(accel/proximity/light/
   orientation)` → `SensorEventQueue::read()`. Same `extern "C"`
   open/enable/poll/max_range/resolution FFI surface, so `src/main.rs`,
   `hal.rs`, `orientation.rs` are largely unchanged (they already consume events +
   descriptors over the thin FFI). Alternative: the HIDL `ISensorManager`
   (`createEventQueue`/`createDirectChannel`) — but `libsensor`'s `SensorManager`
   (talks to the `sensorservice` binder) is the standard, simpler path.
   - Prefer sensorservice's **fused DEVICE_ORIENTATION** / rotation-vector sensor
     so the accel-gravity orientation calc can stay a fallback (as today).
2. **Bringup ordering** (`tools/scripts/run-hybrid-stack.sh`, `--no-art`): start
   `/system/bin/sensorservice` + `wandr-sensormanager` (task 93) BEFORE wandr-sensors;
   wandr-sensors now waits for `sensorservice` instead of the HAL. Add both to the
   teardown/respawn blocks alongside wandr-sensors.
3. **Delete** the now-unused direct-HAL shim bits (or keep behind a fallback flag
   for ART-off-without-sensorservice, if ever wanted — probably not).
4. **Verify** all three consumers still work `--no-art` WITH the camera path live:
   auto-rotation (rotate phone → `report-orientation`), proximity-screen-off during
   a call (task 78), auto-brightness (task 86) — AND a `--probe-video imagereader`
   run streaming concurrently (sensorservice serving both wandr-sensors and the
   camera's ISensorManager — multi-client through SensorService is exactly what it's
   for).

## Why this is the right shape
- SensorService is *designed* for many concurrent clients (camera, wandr-sensors,
  apps) multiplexing one HAL — that's its whole job. Going through it removes the
  single-client contention entirely.
- Keeps wandr's sensor *policy* (orientation/proximity/brightness → arbiter) intact;
  only the transport under it changes.

## Build / test
- C++ shim builds on a-03 (cc_library, like today's libwart_sensors_hal.so). The
  `libsensor` headers/lib are on-device + in the AOSP tree.
- Device A/B as in task 93: rotate / cover-proximity / lux-change while
  `--probe-video imagereader` streams — both must work together.

## ‼️ KNOWN BUG (2026-06-06) — AIDL `SensorManagerAidl` event-queue spins 100% CPU

When the **arbiter actively consumes** sensors via the AIDL
`android.frameworks.sensorservice.ISensorManager` (this build's `wandr-sensormanager`
registers HIDL **and** AIDL), a **wandr-sensormanager binder thread busy-loops at ~100%
CPU** (device-confirmed: `binder:<pid>_1`, huge utime+stime). Root cause: the AIDL
`SensorManagerAidl` **`createEventQueue` poll thread under the fakeVM (null `JNIEnv`)** —
its looper `pollOnce` returns immediately instead of blocking on the sensor BitTube fd,
so it spins. The **camera never hit this** (camera EIS uses the HIDL **direct channel**,
no poll thread); only the **event-queue** path the arbiter sensor-driver uses trips it.

Consequence: the current task-94 stack is **not deployable as-is** — bringing up
arbiter + AIDL wandr-sensormanager pins a core, and bad proximity readings make the power
module flap `set_display_power`. (Also: restarting the arbiter alone orphans the
wandr-inputflinger window registration → touch dies; needs a full re-bringup or the
`refeed_last_block` path — see `[[project_pathA_inputflinger]]`.)

**Fix before landing:** give the fakeVM poll thread a real blocking looper (proper
`ALooper`/fd wait), or pump the AIDL event queue directly from the BitTube fd without
the libsensorservice poll-thread JNI path, or gate the arbiter to HIDL
`createDirectChannel`. Until fixed, use the **HIDL-only nullptr wandr-sensormanager** for
camera work (no AIDL consumer → no spin). Tracked alongside `tasks/95-*` (gyro race).

**FIRST STEP for the fix (source-first — `[[feedback_read_source_first]]`):** the
diagnosis above is from *symptoms* (a busy thread + null-`JNIEnv` reasoning), NOT from
reading the code — confirm it before patching. The spinning tid was `binder:<pid>_1`
(a **regular binder** thread, not `HwBinder:` and not an obviously-named poll thread),
so **first verify which thread actually spins**: the libsensor `SensorEventQueue`
poll/looper thread (from `createEventQueue → getLooper`) vs. an `ABinderProcess` binder
pool worker — the fix differs (blocking-looper fix vs. a stuck binder-call handler).
Read end-to-end: `frameworks/native/services/sensorservice/aidl/SensorManagerAidl.cpp`
(`createEventQueue`/`EventQueue`) + libsensor `SensorEventQueue` + the `ALooper`/BitTube
fd path, and compare to how system_server's real (ART) `SensorManagerAidl` avoids the
spin. Repro: arbiter consuming AIDL → `ls /proc/<sm-pid>/task` + per-tid stat utime/stime.

See `[[project_artless_camera]]` (task 93), `[[project_artless_sensors]]` (task 85),
`[[project_proximity_screen_off]]` (task 78), `[[project_artless_autobrightness]]`
(task 86), `runtime/wandr-sensors/`, `runtime/wandr-sensormanager/`.
