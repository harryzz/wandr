# Task 85 — ART-less sensors (path A: standalone sensorservice)

> Status: ✅ DONE + device-verified (auto-rotation, Pixel 2 XL, 2026-06-04). Under
> `--no-art` no sensors worked — `ISensorManager` is served by the C++ `SensorService`
> instantiated inside `system_server`, which dies with ART. Fix: a thin C++ HIDL shim
> (`libwandr_sensors_hal.so`) reads `android.hardware.sensors@1.0::ISensors` directly
> (the HAL survives ART-off) + a Rust `wandr-sensors` daemon that prefers the HAL-fused
> `DEVICE_ORIENTATION` (no Fusion port needed — the HAL fuses on the SSC core) and
> pushes `report-orientation` to the arbiter. **Verified: physically rotating the phone
> auto-rotates the UI with the Java framework fully stopped** (daemon logged
> `report-orientation 0↔3`, UI + chrome + touch followed via task 84). Wired into
> `run-hybrid-stack.sh --no-art`.
>
> **PROXIMITY also done + device-verified (2026-06-04):** wandr-sensors enables the
> proximity HAL sensor (type 8), pushes its descriptor (`max_range` from the HAL) via
> a new arbiter verb `report-sensor-descriptor`, and feeds each reading via
> `report-sensor proximity <x>`. The arbiter classifies near/far and — during a call
> (`wandr-arbiter-power` gated on `CommsActive`) — blanks the panel on near + restores
> on far, with touch suppression (task 79). Verified: cover → screen blanks + touch
> suppressed, uncover → panel ON + touch resumed (raw HAL values 0 near / 5.0 far).
> Follow-on: on-demand ref-counted enable (arbiter `SetSensor` → wandr-sensors) instead
> of always-on; light / other sensors as needed.

## Experiment 1 (2026-06-04): standalone `sensorservice` — BLOCKED by system_server deps

Ran the on-device `/system/bin/sensorservice` via `wandr-launch` (uid system) +
`setenforce 0` under `--no-art`. Result: process is **alive but HANGS in init and
never registers** (`service check sensorservice` → not found). Cause:
`SensorService` (during init) calls `isAutomotive()` →
`serviceManager->waitForService("package_native")` (`SensorService.cpp:144`) — the
native **PackageManagerService**, which lives in `system_server` and is gone under
`--no-art`. `waitForService` retries forever ("Waited one second for package_native"
in logcat). Beyond that, `SensorService` is **heavily coupled to the system_server
permission layer**: `UidPolicy` (ActivityManager uid observer), `SensorPrivacyPolicy`
(SensorPrivacyManager), `AppOpsManager`, package/permission checks — all
multi-app-sandbox machinery. Conclusion: the full SensorService is NOT a clean
standalone drop-in like `InputManager` was; its policy layer assumes system_server.

## Decision — the lean path: a thin C++ HIDL sensor shim → Rust (NOT full sensorservice, NOT a Rust port)

Two non-starters: **(a)** running the full `sensorservice` drowns in the
permission-layer deps above; **(b)** porting it to Rust is blocked because the sensors
arrive over **HIDL** — this device exposes only `android.hardware.sensors@1.0::ISensors`
(hwbinder transport), and `rsbinder` speaks AIDL/binder only, so Rust can't reach the
HAL here at all. (Device-specific: a newer device with the AIDL sensors HAL could be
pure-Rust.) Sizes for reference: `SensorService.cpp` 2856 lines, whole subsystem 8550
— but most is per-app connection mgmt / AppOps / privacy / direct channels that wandr's
single-context model doesn't need.

So: a small **C++ HIDL shim** (the `HidlSensorHalWrapper` pattern, a few hundred lines
— exactly like `libsf_surface` is a C++ shim for libgui) that opens `ISensors@1.0`,
enables the wanted sensors, and feeds raw events across a thin FFI to Rust. Then:
- Base sensors (accel, **proximity**, light, gyro, mag) flow straight to the existing
  Rust consumers (`wandr-hal-sensors` / `wandr-arbiter-sensors` reshaped to consume the
  shim instead of `ISensorManager`).
- **Device-orientation = ~15 lines of Rust** (gravity vector → 0/1/2/3 + debounce) →
  `report-orientation` to the arbiter → auto-rotation (task-84 input rects already
  rotate). No fusion needed for rotation.
- Any genuinely-fused sensor (rotation vector, gravity) → port that specific math from
  `Fusion.cpp` (~565) / the per-sensor files (~80–160 each) to Rust later, only if a
  consumer needs it.

This skips the permission-layer mess entirely (we never run SensorService's policy)
and contains the HIDL in one small shim — the same C++-shim-thin / Rust-logic split as
`libsf_surface`. Build the shim on a-03; verify it reads accel + computes orientation
under `--no-art`.

## Experiment 2 (2026-06-04): direct HIDL HAL access WORKS — and the HAL provides the fused sensors

Built `wandr_sensors_probe` (cc_binary, `android.hardware.sensors@1.0`) and ran it under
`--no-art` via `wandr-launch` + `setenforce 0`. **It works**: `ISensors@1.0` acquired, **29
sensors enumerated**, accelerometer streaming. Crucially the HAL exposes the **fused/
virtual sensors itself** — this Qualcomm device computes them on the SSC/SLPI sensor core,
NOT in the framework SensorService: `Device Orientation` (type 27, handle 23), `Rotation
Vector` (11), `Gravity` (9), `Linear Acceleration` (10), `Orientation` (3), `Game/Geomag
Rotation Vector` — all present.

**So `Fusion.cpp` does NOT need porting** (it would only matter on a device whose HAL
lacks the fused sensors). Auto-rotation = read **DEVICE_ORIENTATION (27)** directly (it
reports 0/1/2/3) → `report-orientation`. The accel→orientation Rust calc
(`orientation.rs`, gravity-vector + debounce, 5 unit tests) stays as the **fallback** for
HALs without type 27. This is strictly better than a port — the vendor fusion is tuned.

## Build artifacts (this is the shipped design)
- `runtime/wandr-sensors/cpp/wandr_sensors_hal.cpp` → **`libwandr_sensors_hal.so`** — thin
  C-ABI HIDL shim (open / enable-by-type / poll), `dlopen`'d by the daemon (HIDL = hwbinder,
  unreachable from rsbinder). Built on a-03.
- `runtime/wandr-sensors/` Rust **`wandr-sensors`** daemon — `dlopen`s the shim, prefers
  HAL `DEVICE_ORIENTATION` (accel fallback), pushes `report-orientation <0..3>` to the
  arbiter on change. Owns the single HAL connection (poll is single-consumer); future
  consumers (proximity → task 78) read from here.

## How to run / test

- Build the C++ HIDL shim on a-03 (against `android.hardware.sensors@1.0`, libhidlbase)
  → `libwandr_sensors.so` (or a small `wandr-sensors` reader binary), deploy to device.
- Wire the Rust side (`wandr-hal-sensors`/`wandr-arbiter-sensors`) to the shim; add the
  accel→orientation calc + `report-orientation` emit. Launch under `--no-art` (start
  before/with the hosts in `run-hybrid-stack.sh`).
- Verify: physically rotate the phone → UI rotates (no manual `report-orientation`);
  proximity cover/uncover drives screen-off during a call (task 78).

## Done when
Under `--no-art`, physically rotating the phone rotates the UI (auto-rotation), and
the wandr sensor consumers (orientation, proximity) work — no manual `report-orientation`.

## Related
Task 84 (input routing — rotates correctly once orientation is set), task 77
(`wandr-arbiter-sensors`, consumes `wandr-hal-sensors`), task 78 (proximity screen-off),
task 80 (path A for input — the template), `[[feedback_no_art_layer_dependencies]]`,
`[[project_art_shutdown]]`.
