---
name: project_artless_camera
description: Task 93/95 ✅ camera capture works under --no-art (29fps raw / 17fps VP8 encode); EIS-gyro race now reliably won — likely the batterystats fix removed the 5s gyro-enable stall
metadata: 
  node_type: memory
  type: project
  originSessionId: 8a79d726-5989-436c-93ca-fceb8f26e051
---

**✅ 2026-06-07 UPDATE (task 95 — reliably works now):** all three `--probe-video`
modes ran **clean 3/3 today** under `--no-art` (no gyro-race loss):
- `--probe-video imagereader` (raw YUV delivery, no codec): **29.1 fps** 640×480,
  first-frame 140 ms — full ART-parity camera delivery.
- `--probe-video c2.android.vp8.encoder` (SW VP8): **17.3 fps**, ~1830 B/frame, 3 kf,
  180 ms.
- `--probe-video` (default HW VP8, `createEncoderByType`): **17.4 fps**, ~792 B/frame,
  1 kf, 128 ms. HW is the better path (smaller+faster); pick it for video calls.
- **Raw 29 vs encode 17 split** is the encoder/input-surface pipeline in the probe,
  NOT the camera and NOT codec choice (both SW+HW = identical 87 frames). Naming: it's
  `imagereader` (codec-free) — there is NO `imagereaderclear`; "software codec" = the
  separate `c2.android.vp8.encoder` name-arg.

**Likely WHY the gyro race is now won — the `batterystats` fix (battery saves gyro 🙂).**
The EIS gyro-arm is a *sensor enable*, which under --no-art was paying the same ~5s
`BatteryService::checkService` blocking-`getService("batterystats")` stall as proximity
(see [[project_artless_sensor_5s_batterystats]]). Registering the `batterystats` shim
stub removed that 5s gyro-enable stall → the EIS startup timing tightened → race won
3/3 (vs the old ~1/8). Strong correlation, not yet isolated to a controlled A/B (the
old "persistent gyro client" theory may now be moot). If it ever regresses, re-check
that `batterystats` (+ `appops`) resolve before the camera opens.

**⚠️ prior (2026-06-06) framing, kept for context:** 28.8 fps was **real but
non-deterministic** — full 29 fps once running, but only when a **~1-in-8 EIS-gyro
startup race was won**; lose → gyro unarmed (`msm_stopGyroThread: invalid timer
state = 0`) → 0 frames. Two entangled bugs: (1) **Magisk `am`-spin** starved the qcam
HAL → SIGABRT — **SOLVED via `adb root`**, see [[reference_artoff_magisk_am_spin]];
(2) the **gyro session race** (task 95) — now addressed by the batterystats fix above.
The 5-piece stub chain below is still correct/required.

Task 93 — **camera CAPTURE works under `--no-art`**: device-verified 28.8 fps /
640×480 YUV (`wart-host --probe-video imagereader`), matching ART's 29 fps. The
qcom camera HAL goes from "can't open" to streaming with the Java framework
stopped via FIVE pieces (all source-grounded in AOSP, A/B-confirmed):

1. `permission_checker` (IPermissionChecker) — camera-open hang. wart-activityms GenericStub.
2. `media.camera.proxy` (ICameraServiceProxy) — `isCameraDisabled` fail-closes
   (`proxyBinder==nullptr → return true`) → "Camera disabled by device policy"
   (-10012). GenericStub `isCameraDisabled→0=false`.
3. `processinfo` (IProcessInfoService) — camera eviction priority query;
   CUSTOM array stub (out int[] states/scores sized to input pid count; mirror
   `BpProcessInfoService` IProcessInfoService.cpp).
4. `package_native` (IPackageManagerNative) — codec `AMediaCodec_configure` hang
   (MediaCodec::connectFormatShaper waitForService). GenericStub. (Also un-hangs
   standalone `/system/bin/sensorservice`, the old task-85 blocker.)
5. **`wart-sensormanager`** (NEW C++ service, `runtime/wart-sensormanager/`) —
   registers `android.frameworks.sensorservice@1.0::ISensorManager` (system_server
   normally does: `new SensorManager(vm)`). The HAL's **EIS** (video stabilization)
   needs the gyro via ISensorManager; without it `startChannelLocked` SIGABRTs
   (`mct_controller_proc_serv_msg: Timedout ... type=1`). Impl = libsensorservicehidl
   `SensorManager(nullptr)` (null JavaVM ok — only createEventQueue's poll thread
   touches it; EIS uses direct channels). `registerAsService()`.

**Runtime recipe:** 4 stubs (wart-activityms) + `/system/bin/sensorservice` (owns
the single-client sensors HAL, registers `sensorservice`) + `wart-sensormanager`
(HIDL ISensorManager on top). HAL-OWNERSHIP GOTCHA: sensorservice now owns the
sensors HAL → the task-85 `wart-sensors` daemon (reads the HAL directly for
auto-rotation) CONFLICTS; reshape it to consume sensorservice's ISensorManager.

**Method that cracked it:** ART-up vs `--no-art` A/B (works @29fps ART, SIGABRT
`--no-art` → it's a framework dependency, not our config — [[project_artless_audio]]
pattern). `--probe-video imagereader` (camera→AImageReader, explicit dims) isolated
frame-delivery from the encoder 0×0 surface. Don't `pkill -9` a hung probe (wedges
cameraserver — restart it). a-03 new-module build: `m` dies in kati →
`m <mod> WITH_DEXPREOPT=false ALLOW_MISSING_DEPENDENCIES=true`.

REMAINING (no `--no-art` blockers): encode-feed (AImageReader→codec input buffers;
the zero-copy encoder input surface reports 0×0 to camera2 NDK — use ImageReader
instead), decode path, WIT + wart-call video track (rtc-rtp VP8 payloader exists).
See [[project_wart_call]], [[project_artless_sensors]] (task 85), `tasks/93-video-call.md`.
