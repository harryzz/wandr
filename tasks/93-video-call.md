# Task 93 — Video calls (AV gap to a full Signal app)

> Status: ✅ CAMERA CAPTURE RELIABLE under `--no-art` (2026-06-08: 29.1fps raw +
> HW VP8 encode 17.4fps, 3/3 — the task-95 EIS-gyro race is now reliably won, see
> `tasks/95-*`/[[project_artless_sensor_5s_batterystats]]). The WIT contracts are
> ✍️ **DRAFTED + validated**: `wit/video.wit` (`wandr:video`) + `wit/crypto.wit`
> (`wandr:crypto`), commit `697da5de` (steps 5 + risk #2 below). Remaining = pure
> integration (no `--no-art` blockers): implement those WITs host-side (encode-feed,
> HW decode→surface, AEAD offload) + wire the wandr-call video track + render.

## ✅ SOLVED: full `--no-art` camera capture chain (2026-06-06)

Five new pieces, all device-verified, take the camera from "can't open" to
**28.8 fps capture** with the Java framework stopped (matches ART's 29 fps):

1. **`permission_checker`** stub (wandr-activityms) — camera-open permission gate.
2. **`media.camera.proxy`** stub — `isCameraDisabled` fail-closed → device-policy.
3. **`processinfo`** custom stub — camera eviction priority query.
4. **`package_native`** stub — codec `configure` (`connectFormatShaper`).
5. **`wandr-sensormanager`** (NEW C++ service, `runtime/wandr-sensormanager/`) —
   registers `android.frameworks.sensorservice@1.0::ISensorManager`, which
   system_server normally publishes (`new SensorManager(vm)`). The qcom camera
   HAL's **EIS** (video stabilization) needs the gyro via `ISensorManager`;
   without it `startChannelLocked` SIGABRTs (`mct_controller_proc_serv_msg:
   Timedout in processing HAL command type=1`). Runs alongside the standalone
   `/system/bin/sensorservice` (which our `package_native` stub un-hung — the
   task-85 blocker). `SensorManager(nullptr)` JavaVM is fine (only `createEventQueue`
   touches it; EIS uses direct channels).

**Runtime recipe (camera under `--no-art`):** the 4 stubs (wandr-activityms) +
`sensorservice` (owns the sensors HAL, registers `sensorservice`) +
`wandr-sensormanager` (registers the HIDL `ISensorManager` on top).

> ‼️ **PREREQUISITE — `tasks/94-wandr-sensors-refactor-for-sensorservice.md`.** The
> sensors HAL (`ISensors@1.0`) is single-client. `sensorservice` MUST own it (only
> it provides the `ISensorManager` the camera needs), but the task-85 `wandr-sensors`
> daemon currently opens that HAL DIRECTLY → they can't coexist (device-confirmed
> `DEAD_OBJECT` abort). **Task 94 refactors `wandr-sensors` to read sensors *through*
> `sensorservice` (libsensor client) instead of the HAL** — required before the
> camera path can run alongside wandr's auto-rotation / proximity / auto-brightness.
> Until then, the camera helpers + wandr-sensors are mutually exclusive (the probe
> ran with wandr-sensors stopped).

> (Original analysis + spike narrative follows.)

> Earlier status: 🟡 ANALYSIS + SPIKE. Audio calls work (tasks 75/87/91); video is the
> remaining AV gap. This task scopes what's needed and de-risks it with a
> `wandr-host --probe-video` camera→HW-VP8 spike. Analysis 2026-06-06.

## Headline (verified live on device, `--no-art`)

**Nothing is missing at the Android-native service layer** — every subsystem a
video call needs is already running with the Java framework stopped, and the SoC
has the right HW codecs. The whole gap is *our* integration code.

Verified present under `--no-art`:

| Need | Service / HAL (pid) | Notes |
|---|---|---|
| Camera | `cameraserver` (20684) + `media.camera`/`ICameraService` + `frameworks.cameraservice` + `camera.provider@2.4` HAL (1122) | unblocked by the task-87 stubs (activity/permission/sensor_privacy/scheduling_policy) — same `waitForService` path as audioserver |
| Codecs | `media.codec` (HW Codec2, 1418) + `media.swcodec` (1420) + `media.c2 IComponentStore` | |
| Buffers/display | gralloc `allocator@2.0` (764) + `composer@2.1` (763) | HIDL allocator present (the AIDL `IAllocator` "not in VINTF" log is benign) |

HW codecs on this SoC (`/vendor/etc/media_codecs*.xml`): `OMX.qcom.video.encoder.vp8`
+ `decoder.vp8` (**HW VP8 both ways**), `decoder.vp9` (HW VP9 decode), AVC/HEVC
enc+dec. → A **VP8 call is fully hardware-accelerated** — and VP8 is what
Signal/WebRTC negotiate. No SW-codec compromise.

## What we already have

- **RTP video done**: `external/rtc/rtc-rtp/src/codec/{vp8,vp9,h264}` payloaders +
  depacketizers.
- **Transport proven**: SRTP/DTLS/ICE/TURN — audio call connects + interops with a
  real browser ([[project_wandr_call]]).
- **Signaling advertises VP8/VP9** already (ringrtc requires it; `signal/mod.rs`).
- **Native-AV integration pattern**: `audio_impl.rs` = rsbinder to `media.aaudio` +
  `binder_shared_memory.rs` + `eventfd_signal.rs` — whose comments already pre-plan
  "CameraService BufferQueue and Codec2 ports."
- **EGL/Skia** render path for the remote frame.

## The gap = integration to build

Native-facing clients (mirror `audio_impl` — rsbinder/NDK + shared-mem):
1. **Camera capture** — NDK Camera2 (`libcamera2ndk`) or direct `ICameraService`;
   `YUV_420_888` stream ~640×480/720p @ 24–30 fps.
2. **Video codec** — `MediaCodec` via NDK `AMediaCodec` (`libmediandk`) or Codec2;
   `OMX.qcom.video.encoder.vp8`/`decoder.vp8` (HW). YUV↔VP8.
3. **gralloc buffer plumbing** — `AHardwareBuffer`/BufferQueue camera→encoder and
   decoder→display; zero-copy ideal (camera output surface == encoder input surface).

Glue (our code):
4. **wandr-call video track** — wire the existing VP8 payloader/depacketizer into a
   video RTP stream over the existing SRTP transport; RTCP PLI/FIR keyframe requests.
5. **WIT** — ✍️ **DRAFTED 2026-06-08: `wit/video.wit` (`wandr:video`) + `wit/crypto.wit`
   (`wandr:crypto`)**, both wasm-tools-validated (commit `697da5de`); NOT yet implemented.
   - `wandr:video` — bidirectional HW codec via host MediaCodec/Codec2: `encoder` (host
     captures camera + HW-encodes, guest **pulls** `next-frame()`) + `decoder` (guest
     **pushes** `submit(frame)`, host HW-decodes). Carries **encoded** frames (KB each).
     Decisions baked in: **decode-to-surface** (host composites; pixels never re-enter
     the guest — zero-copy, right for 30fps), and **prefer VP8 for OUT** (VP9 HW *encode*
     is SW-only on this SoC; VP9/VP8 HW *decode* both present — confirmed
     `/vendor/etc/media_codecs*.xml` 2026-06-08).
   - `wandr:crypto` — the SRTP AEAD offload (risk #2 below): `aead.gcm` seal/open per
     packet, key schedule expanded once. Guest keeps the SRTP framing, offloads only the
     AES-256-GCM primitive to host RustCrypto on ARMv8 HW AES.
   - 🔲 STILL OPEN: wire the `wandr:video` decoder surface to an arbiter **`Role::Video`**
     surface (z-order vs the guest skia UI, rotation, occlusion) instead of the sketch's
     raw `video-rect`; same for the self-view preview.
6. **Render** — decoded YUV → Skia (SkImage-from-YUV / YUV→RGB shader) → local PiP +
   remote in the call UI. (Mostly subsumed by decode-to-surface host compositing per
   the `wandr:video` decision above; Skia path applies if decode-to-buffer is chosen.)
7. **Signal protocol** — in-call video enable/disable + resolution/bitrate adaptation.

## Two real risks (the spike targets these)

1. **Camera `open()` permission under `--no-art`** — cameraserver is up, but `open()`
   may hit its permission/AppOps check (the audio path needed the `permission`
   stub). May need another stub or a root bypass.
2. **SRTP at video bitrate in wasm** — audio SRTP-in-wasm works, but video is
   ~10–50× the packet rate; [[project_crypto_hw_offload]] already flags SRTP should
   move host-side. **Direction chosen (2026-06-08, see `wit/crypto.wit`):** keep the
   SRTP *framing* (ROC/replay/HKDF in `rtc_srtp::Context`) IN the guest, offload only
   the per-packet **AEAD primitive** (`wandr:crypto` `aead.gcm` seal/open) to host
   RustCrypto on ARMv8 HW AES — a small WIT, not a full pipeline-to-host fork. Signal
   V4 SRTP = `AEAD_AES_256_GCM` (`wandr-call transport.rs:468`), software AES in wasm
   today, so this offload applies to **audio calls too** (task 91), not just video.

## Spike: `wandr-host --probe-video`

Prove camera → HW VP8 encode end-to-end under `--no-art`, minimal code, via the
**Surface path** (camera writes frames straight into the encoder's input surface —
no manual YUV buffer copies):
1. `ACameraManager` → open a camera (back/first id).
2. `AMediaCodec` VP8 encoder, `COLOR_FormatSurface`, `createInputSurface` →
   `ANativeWindow`.
3. Capture session targeting that window → `setRepeatingRequest(PREVIEW)`.
4. ~5 s: drain `AMediaCodec` output → count frames + sizes, first-frame latency.
5. Report: camera-opened? encoded-frames/fps/avg-size/first-frame-ms + any error.

Answers risk #1 (open under ART-off) + encoder throughput in one shot. Module
`runtime/wandr-host/src/video_probe.rs`, flag `--probe-video`, links `camera2ndk` +
`mediandk`. Next after green: decode-path probe, then the WIT + wandr-call video track.

## SPIKE RESULTS (2026-06-06, device, `--no-art`)

`wandr-host --probe-video [codec-name]` built + run. Findings:

1. **Binder threadpool is required (infra).** The NDK camera/codec libs use C++
   `libbinder` (not `libbinder_ndk`); our process must run the C++ libbinder
   threadpool or every camera/codec call hangs ("Thread Pool max thread count is
   0"). The NDK stub doesn't export `ABinderProcess_*`, and rsbinder's threadpool
   is a separate context. **Fix = `sf_start_binder_threadpool()` added to the
   task-33 C++ shim** (`ProcessState::self()->startThreadPool()`), dlopen'd by the
   probe. **Reusable by the real camera/codec integration.**
2. ✅ **Camera enumerates under `--no-art`** — `getCameraIdList` returns 2 cameras
   (front+back) with a fresh cameraserver. The camera *path* is reachable ART-off.
3. ⚠️ **`AMediaCodec_configure` HANGS under `--no-art`** — for BOTH the HW
   `OMX.qcom.video.encoder.vp8` AND the SW `c2.android.vp8.encoder`. The probe's
   main thread blocks in a binder transaction; `media.codec`'s `omx@1.0-service`
   thread is itself stuck in binder during component configure. **Same class as
   task-87 audio** (a media service blocked on a framework dependency ART-off) —
   NOT a HW-vendor-specific issue (SW hangs too), so it's a Codec2/MediaCodec
   framework dependency missing/blocked without `system_server`.
4. **GOTCHA:** `pkill -9` of a hung probe leaves a half-open client in cameraserver;
   accumulated kills progressively WEDGE cameraserver (then even enumeration
   blocks). Restart cameraserver (`pkill cameraserver`, respawns via init) between
   runs; the probe needs clean async teardown (don't SIGKILL mid-transaction).

**Verdict:** the big unknowns are de-risked — every native service exists ART-off,
HW VP8 is present, the camera enumerates, and the binder-threadpool path works. The
remaining blocker is the **codec `configure` binder block**, a focused task-87-style
investigation: read the CCodec/Codec2 `configure` path (frameworks/av
`media/codec2` + `MediaCodec.cpp`), find which service it waits on without
`system_server`, and stub it (extend the `wandr-activityms` stub set) — OR decide
SW-encode-in-guest/host as a fallback. Camera `open()` permission (risk #1) is
still unconfirmed (blocked behind the encoder in the probe order); swap the probe
to open-camera-first to isolate it next.

Spike artifacts committed: `video_probe.rs`, the `sf_start_binder_threadpool` shim
entry, `build.rs` (camera2ndk/mediandk links), `--probe-video` dispatch.

## IDENTIFIED BLOCKER + fixes to try (2026-06-06) — camera privacy/policy services

Tracing the hang surfaced the privacy/permission angle: during the probe,
**cameraserver logs `PermissionChecker: Waiting for permission checker service`**.
`service check` confirms which `system_server`-hosted policy services are GONE
under `--no-art` (and NOT in our task-87 stub set):

| service | present? | who needs it |
|---|---|---|
| `permission_checker` (IPermissionChecker) | **missing** | cameraserver (observed wait); the modern AppOps-integrated permission check |
| `appops` (IAppOpsService) | **missing** | camera/mic op gating (noteOp/checkOp CAMERA) |
| `platform_compat` (IPlatformCompat) | **missing** | MediaCodec/Codec2 compat-change checks |
| `device_policy` (IDevicePolicyManager) | missing | admin camera-disable (likely not critical) |
| `permission`,`sensor_privacy`,`activity`,`scheduling_policy` | present | already stubbed (task 87, `wandr-activityms`) |

**Fixes to try, in priority order** (each = a new binder stub in
`runtime/wandr-activityms/cpp/wandr_activityms.cpp`, the proven task-87 pattern —
`addService` a `BnX` returning the allow/granted answer; built on a-03):

1. **`permission_checker` / `IPermissionChecker`** — the directly-observed blocker.
   Stub `checkPermission(...)`-family to return `PERMISSION_GRANTED`
   (`PermissionChecker::PERMISSION_GRANTED = 0`). Highest-value first try.
2. **`appops` / `IAppOpsService`** — stub `checkOperation`/`noteOperation`/
   `startOperation` to return `MODE_ALLOWED (0)`, `checkPackage` OK. Camera+mic
   attribution rides AppOps; very likely needed alongside #1.
3. **`platform_compat` / `IPlatformCompat`** — if the codec `configure` still hangs
   after #1/#2, stub `isChangeEnabled*` → false / no-op. Codec2/MediaCodec query
   compat changes during configure.

**Still to disentangle:** the probe currently creates+configures the encoder
BEFORE opening the camera, so it's unclear whether `permission_checker`/`appops`
is blocking the *codec configure* or only the *camera open*. Reorder the probe to
**open-camera-first** (then encoder) to isolate which service each step needs —
do this together with adding stub #1 so the next run shows real progress. The
`permission_checker` wait is a certainty for camera `open()` regardless (risk #1).

Method note: don't `pkill -9` a hung probe (wedges cameraserver — restart it
between runs). Trace blockers via `cat /sys/kernel/debug/binder/proc/<pid>` +
`logcat | grep -iE 'Waiting for|waitForService|PermissionChecker'`.

## CAMERA-OPEN STUB CHAIN (2026-06-06, in progress) — peeling the privacy/policy layers

Probe reordered **open-camera-first** (`video_probe.rs`) to isolate the camera-open
gate from the codec. Each `system_server` service cameraserver needs is being
stubbed in `wandr-activityms` (the proven task-87 pattern). Progress, layer by layer
(each verified on device by the error CHANGING):

1. ✅ **`permission_checker`** (IPermissionChecker) — was: `openCamera` HANGS
   ("Waiting for permission checker service"). Stub (GenericStub, descriptor
   `android.permission.IPermissionChecker`) → hang gone; `openCamera` now *returns*.
2. ✅ **`media.camera.proxy`** (ICameraServiceProxy) — was: `-10012
   PERMISSION_DENIED`, "Camera disabled by device policy".
   `CameraServiceProxyWrapper::isCameraDisabled` FAIL-CLOSES (`proxyBinder==nullptr
   → return true`). Stub (GenericStub, `android.hardware.ICameraServiceProxy`;
   `boolean isCameraDisabled(int)` reads `writeInt32(0)`=false → enabled) → policy
   check passes; error moved on.
3. ✅ **`processinfo`** (IProcessInfoService) — was: `-10000`, cameraserver
   `Could not retrieve process states and scores from ProcessInfoService after 5
   retries` → `Priority score query failed: -110` (timeout). The camera eviction
   logic (`CameraService.cpp:2007` `getProcessStatesScoresFromPids`) queries
   `processinfo` and FAILS the open if `err != OK`. **Custom stub** (`ProcessInfoStub`
   — the blanket GenericStub can't serve `out int[]`): read the input pid count N,
   reply `writeNoException` + `writeInt32(N)` + N×`PROCESS_STATE_TOP` + [scores:
   `writeInt32(N)` + N×`0`] + trailing `writeInt32(NO_ERROR)`. Marshalling mirrors
   `BpProcessInfoService` (frameworks/native `IProcessInfoService.cpp`); `N` MUST
   equal the input count or the client returns `NOT_ENOUGH_DATA`. Codes 1/2,
   descriptor `android.os.IProcessInfoService`.

### ✅ CAMERA OPEN WORKS under `--no-art` (2026-06-06)

With all three stubs (`permission_checker` + `media.camera.proxy` + `processinfo`)
in `wandr-activityms`, the reordered probe prints
**`camera OPENED id=0 (status=0) ✓`** — risk #1 RESOLVED. The probe then proceeds
to the encoder and hangs at **`AMediaCodec_configure`** (the separate, already-known
codec blocker — `media.codec`'s `omx@1.0-service` stuck in binder), now cleanly
isolated *after* a good camera open.

## ✅ CODEC CONFIGURE UNBLOCKED — all `--no-art` service blockers solved (2026-06-06)

4th stub: **`package_native`** (IPackageManagerNative). `AMediaCodec_configure`
hung in `MediaCodec::connectFormatShaper` → `waitForService("package_native")`
(device-confirmed: probe pid retrying it 14×). It only calls `hasSystemFeature()`
to guess "handheld" for format-shaping (not load-bearing), so a GenericStub
unblocks it. `MediaCodec.cpp:2681`.

**Full `--no-art` camera→HW-VP8 service chain is now resolved** — 4 stubs in
`wandr-activityms` (`permission_checker`, `media.camera.proxy`, `processinfo`,
`package_native`). Device-verified end of the binder-blocker hunt: the reordered
`--probe-video` runs the WHOLE setup clean — camera OPEN ✓, encoder configure ✓,
start ✓, input surface ✓, repeating capture ✓ — and the camera HAL streams
(`mm-camera: Session stream linked successfully`).

## NON-BLOCKER remaining: camera↔encoder frame plumbing (0×0 surface)

0 frames encoded — but NOT a service/`--no-art` problem. The camera configures a
**0×0** stream (`mm-camera: c2d_module_notify_add_stream: width 0 height 0` →
DEL_STREAM) because the NDK `ACaptureSessionOutput`/`ACameraOutputTarget` derive
the stream size from the encoder input surface's CONSUMER side (the
`GraphicBufferSource` behind `AMediaCodec_createInputSurface`), which reports 0×0.
Producer-side `ANativeWindow_setBuffersGeometry(w,h,fmt)` does NOT fix it (the
camera sets its own geometry as producer). This is a known NDK camera↔MediaCodec
zero-copy plumbing wrinkle, independent of `--no-art`.

**For the real integration, sidestep it:** feed the encoder via an `AImageReader`
(`YUV_420_888`) intermediate (camera → ImageReader → copy/queue into the codec
input buffers) instead of the zero-copy input Surface — gives explicit dimensions
and is also what the WIT path wants (the host owns the YUV → VP8 step). The
zero-copy Surface path can be revisited later as an optimization.

## FRAME DELIVERY: vendor camera HAL crashes on stream-start under `--no-art`

Decisive test (`--probe-video imagereader`, 2026-06-06): camera → **AImageReader**
(`640×480 YUV_420_888`, explicit dims — sidesteps the encoder 0×0). Still **0
frames**, but for a NEW reason: with correct dims the stream configures
(`mm-camera: VIDEO hw_stream width 640, height 480`) and then the **Qualcomm camera
HAL crashes** — `provider@2.4-service` (pid) takes `SIGABRT` inside
`QCamera3HardwareInterface::startChannelLocked()` ← `process_capture_request`
(`/vendor/lib/hw/camera.msm8998.so`, via `camera.device@3.4/3.5-impl.so`). cameraserver
then sees `DEAD_OBJECT` / `Broken pipe (-32)` / `Shutting down in an error state` /
`Stream 0 leaked`, and the BufferQueue is abandoned. The HAL respawns and
re-enumerates fine, so it's a **deterministic crash on stream-start**, not a teardown
race. (Also seen: an UNRELATED `storaged` SIGSEGV — a separate `--no-art` casualty.)

So the encoder-surface 0×0 was real but secondary; the deeper wall is the **closed
vendor camera HAL aborting when it starts the sensor channel ART-off** — beyond the
binder stubs (can't patch `camera.msm8998.so`).

**A/B DONE (2026-06-06) — it's a `--no-art` dependency, confirmed.** Same probe
(`--probe-video imagereader`), framework UP:

| | camera → ImageReader (640×480 YUV) |
|---|---|
| ART up    | ✅ **145 frames in 5.0s = 29 fps**, 640×480, first-frame 181 ms |
| `--no-art` | ❌ qcom HAL SIGABRT in `startChannelLocked` |

The probe is correct (flawless under ART); the camera HAL aborts at sensor
channel-start ONLY with the framework stopped — the task-87 pattern. So the remaining
work is to identify the specific framework-provided dependency the qcom HAL needs at
`startChannelLocked`. Leads to chase next (get the `--no-art` abort message +
diff property/service access vs the working ART run):
- **System properties** the HAL reads (seen in the ART run, harmless there but worth
  checking ART-off values): `persist.vendor.camera.privapp.list`,
  `ro.vendor.camera.res.fmq.size`, `service.bootanim.exit`, vendor camera props.
- A **vendor daemon** normally (re)started in the framework boot path (perfd/cnd/
  sensor-cal/`vendor.qti.*`) that the channel-start RPCs into.
- Framework-set **camera state** (e.g. `cameraserver`↔framework `ICameraServiceProxy`
  notifications, or a gralloc/usage flag only set with SF fully up).
Method: under `--no-art`, capture the provider@2.4 tombstone **Abort message** +
`strace`/property-access just before `startChannelLocked`, and diff vs the ART run.
(Camera open + codec configure remain solved; this is purely sensor-streaming.)

## Net result of task 93 so far
- Analysis: every native AV service for video calling exists under `--no-art`; HW VP8 present.
- Spike: camera OPEN + codec CONFIGURE both work `--no-art` after 4 `wandr-activityms`
  stubs (all source-grounded in AOSP); camera streams.
- Remaining for a working pipeline: (a) camera→encoder frame delivery via ImageReader
  (above), (b) the decode path, (c) the WIT + wandr-call video track (the RTP VP8
  payloader already exists). None are `--no-art` blockers.

See `wit/task-manager.wit` sibling-package style for `wandr:video`, `audio_impl.rs`
(integration pattern), `external/rtc/rtc-rtp/src/codec/vp8` (packetizer).
