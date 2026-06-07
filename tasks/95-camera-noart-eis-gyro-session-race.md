# Task 95 — Camera under `--no-art`: the EIS-gyro session race (the *real* blocker)

> Status: ✅ RESOLVED 2026-06-08 — race reliably won after the `batterystats` shim fix
> (the gyro-enable was paying a ~5s `BatteryService::checkService` stall). See the
> RESOLUTION block below.
> Spun out of task 93 (camera capture under `--no-art`). Task 93's "28.8 fps" was
> **real but non-deterministic**; this task was about making it **reliable** — now it is.

## ✅ RESOLUTION (2026-06-08) — the `batterystats` shim stub

**The race is reliably won now.** With the task-96 `wart-framework-shim` carrying a
`batterystats` stub (see `[[project_artless_sensor_5s_batterystats]]`,
commit `27f94b4c`), all three `--probe-video` modes streamed **clean 3/3** in one
sitting, with **no gyro warming active** (`warm_camera_gyro()` was already removed):
- `--probe-video imagereader` (raw YUV) → **29.1 fps**, first-frame 140 ms
- `--probe-video c2.android.vp8.encoder` (SW VP8) → 17.3 fps
- `--probe-video` (default HW VP8) → 17.4 fps

**Why this is the fix (reconciles the "downstream of sensorservice / OIS kernel timer"
finding below).** This task had measured that the camera's gyro enable returns
`result=OK` on *every* probe and concluded the race was purely downstream in the kernel
OIS hrtimer — but it never measured the *latency* of that enable. Under `--no-art` the
sensor-enable path `SensorService::enable → BatteryService::enableSensor →
checkService` did a **blocking `getService("batterystats")`** that, with system_server
gone, spun `sleep(1)`×5 ≈ **5.0 s** (observed `enableSensor()=5.017s`). That 5 s stall in
establishing the EIS gyro session is what perturbed the timing of the downstream
`ois_open → msm_startGyroThread` sequence → the OIS hrtimer sometimes never armed
(`msm_stopGyroThread: invalid timer state = 0`) → 0 frames. Register `batterystats` so
the lookup resolves instantly → the gyro session establishes promptly → the OIS thread
arms → frames flow. The OIS-timer observation was a *symptom* of the upstream 5 s
sensor-enable stall, not an independent kernel bug.

**Confidence.** 3 consecutive wins at the documented ~1/8 baseline (no warming, same
stack) is ≈ (1/8)³ ≈ 0.2 % by chance — strong, though not a large-N controlled A/B. If
it ever regresses: run a larger-N win-rate count and confirm `batterystats` (+ `appops`)
resolve **before** the camera opens. The prior dead-ends below (persistent gyro client,
OIS-kernel-timer deep-dive) are now explained/superseded and need no further work.

---

### (historical, pre-2026-06-08 — kept for the record)

## TL;DR

The rear camera **does** deliver full frames under `--no-art` — **29 fps, ART-parity,
and stable once streaming** — but only when a **low-probability (~1 in 8) startup race
on the EIS gyro session is won**. Win → flawless. Lose → the gyro thread never arms
(`msm_stopGyroThread: invalid timer state = 0`) and the qcam pipeline delivers 0
buffers. Under ART the race is won **every** time; under `--no-art` it's ~1/8.

Two **separate** problems were entangled in task 93 and the long debug session:
1. **Magisk `am`-spin** (CPU/HAL starvation) — **SOLVED**, see below. This was the
   crash-mode (`mct_controller_proc_serv_msg: Timedout type=1` → SIGABRT → degraded
   provider). It is *not* the same thing as the gyro race.
2. **The EIS-gyro session race** — **this task**. Remains after #1 is removed.

## What works (proven, repeatedly)

- `adb root` (this is a `userdebug` build, `ro.debuggable=1`) → root commands never
  invoke Magisk `su` → **zero `am`-spinners**. Run the whole probe via root adbd.
- Sensor stack: `/system/bin/sensorservice` + `wart-sensormanager` (HIDL `ISensorManager`
  for the camera EIS gyro). The HAL bridge (`vendor.sensors-hal-1-0`) must be restarted
  to clear any stale single-client claim before sensorservice can own it.
- Then probe: `LD_LIBRARY_PATH=/data/local/tmp /data/local/tmp/wart-host --probe-video imagereader`.
- **Outcome: ~1/8 probes hit `captured frames: ~145 in 5.0s = 29 fps`; the rest 0.**
  When it streams it is rock-solid for the whole window.

## SOLVED sub-bug — Magisk `am`-spin (keep this fixed, do not re-litigate)

Under `--no-art`, **every Magisk `su -c` grant** makes `magiskd` fork
`com.android.commands.am.Am` to deliver a su-access log/notification to its Manager
app. The framework is dead, so `am` can't reach ActivityManager and **`magiskd`
respawns it in a tight loop** (the worker reparents to `adbd`/init). Even a **single**
spinner starves the qcam HAL's MCT command thread past its hard timeout →
`mct_controller_proc_serv_msg: Timedout type=1` → **SIGABRT** of
`android.hardware.camera.provider@2.4-service` → the provider **respawns degraded**
(`gyro_module_init: User selected to disable Gyro module`, `stats_module_init:
Sub-module NULL`) and stays degraded across probes (restarting `cameraserver` does NOT
reset the *provider*).

**Findings:**
- Per-uid `logging`/`notification=0` in the Magisk `policies` table does **not** stop
  it (this is a Magisk fork — `magisk -V` = 30700, hidden manager). Uninstalling the
  registered manager (`com.xyshj.machine`) does **not** stop it either — magiskd
  auto-falls back to the `com.topjohnwu.magisk` stub and keeps forking `am`.
- **Fix that works: `adb root`** (no `su` → no grant → no `am`). For the running stack
  (which *does* use `su`), the existing `magisk_worker_sweep` must become a
  **continuous background daemon** for the `--no-art` lifetime (today it's one-shot at
  bringup), or any timing-sensitive op (camera, and likely others) is at risk.

This belongs in `[[project_art_shutdown]]` / `[[reference_artoff_magisk_am_spin]]`.

## The real problem — EIS-gyro session race

The Pixel 2 XL rear pipeline hard-wires **`goog_eis`** (`pproc` topology:
`tmod → goog_eis → goog_llv → ppeiscore → c2d → cpp`). EIS needs a **high-rate gyro
feed** (the `GoogGyro` "tripod" module). The gyro session is a chain of single-client
QMI/binder sessions: **camera HAL → `ISensorManager` (wart-sensormanager) → libsensor →
sensorservice → SSC/SLPI**. Under `--no-art` this session **establishes only ~1/8 of
the time**; the failing 7/8 leave `GoogGyro` with no feed and the OIS gyro thread
unarmed (`msm_stopGyroThread: invalid timer state = 0`).

### Ruled out (by test/diff this session — do NOT redo)
| Hypothesis | Verdict | Evidence |
|---|---|---|
| Magisk `am`-spin (CPU/HAL starvation) | real but **separate**, fixed via `adb root` | crash-mode only; race persists at am=0 |
| Camera provider degraded state | contributing (post-crash), not the race | `ctl.restart vendor.camera-provider-2-4`; still ~1/8 |
| `wart-sensormanager` JVM: `nullptr` vs fakeVM | not it | both ~1/8; camera uses direct channel |
| `wart-sensormanager` **binder thread pool** missing | **not it** | task-94 build adds `ABinderProcess_startThreadPool()` → "Thread Pool max=0" warning gone, still 0/8 |
| Sensor stack cold vs warm | not it | same warm stack wins then loses |
| Consecutive probes (warm-up trend) | no trend | 1 win at probe #3 of 8, none after |
| SELinux | ruled out | `setenforce 0` (already permissive) still 0 |
| Tangled SLPI firmware / churn | ruled out | **cold power-cycle**, fresh SLPI, exact e7c2a058 binaries → still 0 |
| task-94 sensor unification | neither helps nor breaks | arbiter consumes via AIDL fine; camera still ~1/8 |

### ‼️ Persistent-gyro-client hypothesis — TESTED 2026-06-06 → FALSIFIED
Built the persistent gyro warm-keeper into the arbiter sensor-driver
(`warm_camera_gyro()`: enable raw gyro `SensorType::GYROSCOPE` = 4 at 200 Hz, held
forever) and measured the win rate on the live `--no-art` stack:

- Warm-keeper verified active: `dumpsys sensorservice` → `0x04 (LSM6DSM Gyroscope)
  active-count=1, selected=5.00 ms` (200 Hz) with `last 10 events` populated — the raw
  gyro hardware is genuinely streaming continuously.
- Provider healthy (53 min uptime, **no** `disable Gyro module` degraded markers).
- **Result: 0 / 12 probes streamed** (vs the ~1/8 baseline). Warming the raw gyro does
  **not** improve the camera win rate.

**Why it can't work (the layer mismatch):** the camera's gyro is `Camera Sync 0 - Rear`
(handle `0x09`, `com.google.sensor.sync`), a *different* virtual sensor from raw gyro
(`0x04`); and the actual failure — `msm_stopGyroThread: invalid timer state` — is in the
**kernel CAM_OIS driver's gyro hrtimer**, armed by the camera HAL's `ois_open`/`ois`
ioctl, NOT by sensorservice. The framework/SSC gyro session establishes fine (the camera
enables handle `0x09` at 200 Hz, `result=OK`, every probe); the race is **downstream of
sensorservice**, in the OIS kernel timer. Keeping the framework gyro warm therefore
cannot move it. → **Abandon the persistent-gyro-client direction; pivot to the OIS
kernel gyro path** (next steps below). The `warm_camera_gyro()` code is an experiment,
not a fix — revert unless re-purposed.

### Leading hypothesis (untested — start here)
ART wins the race **every** time; `--no-art` ~1/8. The most likely reason: under ART
**something keeps the gyro continuously active/registered**, so when the camera opens
it finds the gyro session **already established** — no cold-start. Under `--no-art`
nothing holds the gyro on, so the camera **cold-establishes** it on each open → the
race. **Candidate fix: a persistent gyro client** that enables the raw gyro (sensor
type 4 / uncalibrated gyro) via `ISensorManager` and keeps it live *before* the camera
opens. (The task-94 arbiter enumerates accel/proximity/light/device-orientation but
**does not** enable the raw gyro — so it does not currently warm it; that's why
task-94 doesn't move the win rate.)

## Next steps (source-first, per working rules)
> ~~Persistent-gyro-client (old step 2)~~ — **DONE + FALSIFIED** (see the tested box
> above). The race is **downstream of sensorservice**, in the kernel OIS gyro hrtimer.
> New focus = the OIS gyro thread itself.

1. **Find why `msm_startGyroThread` doesn't arm the OIS hrtimer under `--no-art`.** In a
   losing run the log shows `ois_open` then `ois_close` + `msm_stopGyroThread: invalid
   timer state` with **no `msm_startGyroThread`** in between — i.e. the camera HAL opens
   OIS but never (successfully) starts the gyro thread, so the kernel hrtimer is never
   armed → EIS gets no gyro → 0 frames. Read the qcom **OIS HAL** (vendor
   `libmmcamera2_sensor_modules` / `actuator`/`ois` module + the `cam_ois` kernel driver
   `msm_startGyroThread`/`msm_stopGyroThread`) to find **what gates the start** and what
   differs ART-up vs `--no-art`. Capture a **winning** vs **losing** trace filtered on
   `OISDBG|ois_|GyroThread|GoogGyro` — the difference names the gate.
2. **Interim, productionizable:** retry-on-open — reopen the camera until frames flow
   (~handful of tries), then stream stably. Acceptable for the Signal-call use case.
3. **Revert** the falsified `warm_camera_gyro()` warm-keeper in
   `wart-arbiter-bin/src/sensor_driver.rs` (battery cost, no benefit) unless re-purposed.

## Repro (current, deterministic harness)
- `adb root` (kills am-spam).
- HAL bridge restart → `setsid /system/bin/sensorservice` → `setsid
  /data/local/tmp/wart-sensormanager` (task-94, HIDL+AIDL, threadpool).
- Loop the probe ~8–16×, restarting `vendor.camera-provider-2-4` only if it died.
- Scripts left on device: `/data/local/tmp/{warmgyro,fullreset,freshprov}.sh`.

See `[[project_artless_camera]]` (task 93), `tasks/94-*` (sensor unification, validated),
`[[project_art_shutdown]]` (the `am`-spin), `runtime/wart-sensormanager/`.
