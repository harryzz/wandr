---
name: project_artless_camera
description: Task 93 — camera capture works under --no-art (28.8 fps); the 5-piece stub/service chain + the ISensorManager/EIS root cause
metadata: 
  node_type: memory
  type: project
  originSessionId: 8a79d726-5989-436c-93ca-fceb8f26e051
---

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
