# Task 93 — Video calls (AV gap to a full Signal app)

> Status: 🟡 ANALYSIS + SPIKE. Audio calls work (tasks 75/87/91); video is the
> remaining AV gap. This task scopes what's needed and de-risks it with a
> `wart-host --probe-video` camera→HW-VP8 spike. Analysis 2026-06-06.

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
  real browser ([[project_wart_call]]).
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
4. **wart-call video track** — wire the existing VP8 payloader/depacketizer into a
   video RTP stream over the existing SRTP transport; RTCP PLI/FIR keyframe requests.
5. **WIT** — `war:video` (or extend the call WIT): guest controls start/stop video +
   front/back camera + mute; host does capture+encode+decode+render. In-guest call
   engine → WIT carries **encoded VP8 frames** (KB each; cheap).
6. **Render** — decoded YUV → Skia (SkImage-from-YUV / YUV→RGB shader) → local PiP +
   remote in the call UI.
7. **Signal protocol** — in-call video enable/disable + resolution/bitrate adaptation.

## Two real risks (the spike targets these)

1. **Camera `open()` permission under `--no-art`** — cameraserver is up, but `open()`
   may hit its permission/AppOps check (the audio path needed the `permission`
   stub). May need another stub or a root bypass.
2. **SRTP at video bitrate in wasm** — audio SRTP-in-wasm works, but video is
   ~10–50× the packet rate; [[project_crypto_hw_offload]] already flags SRTP should
   move host-side. Video may force host-side SRTP/crypto — an architecture fork
   (keep RTP/SRTP in-guest vs. move the media+crypto pipeline host-side).

## Spike: `wart-host --probe-video`

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
`runtime/wart-host/src/video_probe.rs`, flag `--probe-video`, links `camera2ndk` +
`mediandk`. Next after green: decode-path probe, then the WIT + wart-call video track.

## SPIKE RESULTS (2026-06-06, device, `--no-art`)

`wart-host --probe-video [codec-name]` built + run. Findings:

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
`system_server`, and stub it (extend the `wart-activityms` stub set) — OR decide
SW-encode-in-guest/host as a fallback. Camera `open()` permission (risk #1) is
still unconfirmed (blocked behind the encoder in the probe order); swap the probe
to open-camera-first to isolate it next.

Spike artifacts committed: `video_probe.rs`, the `sf_start_binder_threadpool` shim
entry, `build.rs` (camera2ndk/mediandk links), `--probe-video` dispatch.

See `wit/task-manager.wit` sibling-package style for `war:video`, `audio_impl.rs`
(integration pattern), `external/rtc/rtc-rtp/src/codec/vp8` (packetizer).
