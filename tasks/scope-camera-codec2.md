# Scope: Camera + Codec2 on wart

> Preparatory analysis, not yet a numbered task. Written 2026-05-17 with
> device-confirmed service shapes from a live Pixel 2 XL (LineageOS,
> Android 15 / API 35 base). Captures whether wart's existing rsbinder
> pipeline + the task-21 primitives extend to camera and video codec
> work, and at what cost.
>
> The "primitives" referenced throughout are A2
> (`wart-host/src/binder_shared_memory.rs`) and A3
> (`wart-host/src/eventfd_signal.rs`) from task 21.

## Device probe (Pixel 2 XL, 2026-05-17)

`adb shell service list` (AIDL services on `servicemanager`):

```
11  android.frameworks.cameraservice.service.ICameraService/default
    : [android.frameworks.cameraservice.service.ICameraService]
19  android.hardware.media.c2.IComponentStore/software
    : [android.hardware.media.c2.IComponentStore]
141 media.camera         : [android.hardware.ICameraService]
142 media.camera.proxy   : [android.hardware.ICameraServiceProxy]
```

`adb shell lshal --types=binderized` (HIDL services on `hwservicemanager`):

```
android.frameworks.cameraservice.service@2.0/2.1/2.2::ICameraService/default
android.hardware.camera.provider@2.4::ICameraProvider/legacy/0
android.hardware.media.c2@1.0/1.1/1.2::IComponentStore/software
android.hardware.media.omx@1.0::IOmx/default
android.hardware.media.omx@1.0::IOmxStore/default
android.hardware.cas@1.0/1.1/1.2::IMediaCasService/default
```

VINTF manifest confirms `android.hardware.camera.provider @ 2.4` is the
only camera HAL declared on this device.

## Camera

| Layer | Type | Service / interface |
|---|---|---|
| App-facing system service | **Stable AIDL** ✅ | `media.camera` → `android.hardware.ICameraService` |
| New NDK Camera2 surface (Android 12+) | **Stable AIDL** ✅ | `android.frameworks.cameraservice.service.ICameraService/default` |
| Legacy app-side AIDL bridge | HIDL | `android.frameworks.cameraservice.service@2.0/2.1/2.2` |
| **Vendor camera HAL** | **HIDL only on this device** | `android.hardware.camera.provider@2.4::ICameraProvider/legacy/0` |

**The vendor HAL being HIDL-only is not our problem.** CameraService
(`media.camera`) is stable AIDL on this device and bridges to the
HIDL HAL internally. wart-host talks to `media.camera` via rsbinder
exactly the same way task 21 talked to `media.aaudio` — no HIDL
plumbing in wart.

Permissions: `android.permission.CAMERA` is runtime-prompted, NOT a
SELinux-only check. We'd declare it in `Cargo.toml` and either survive
the prompt dialog from NativeActivity or pre-grant for testing with
`adb shell pm grant com.example.wasmruntime android.permission.CAMERA`.
Permission state survives APK reinstall as long as the package name
doesn't change.

Data plane is `IGraphicBufferProducer` — Android's BufferQueue IPC.
Each frame carries:
- a gralloc fd (HAL-allocated buffer, possibly GPU-resident),
- a **sync fence fd** (`<sync/sync.h>`, NOT eventfd) for buffer-ready signaling,
- metadata (timestamp, transform, crop).

### Primitive reuse — task 21

| Primitive | Fits camera? |
|---|---|
| `BinderMappedMemory` (A2) | ✅ Close fit. gralloc-buffer mmap matches the SharedFileRegion+offset+size shape. May want `MAP_LOCKED` and pixel-format awareness — additive, not breaking. |
| `EventfdSignal` (A3) | ❌ Camera uses `sync_file` fences. Same fd-based shape but different syscalls (`SYNC_IOC_WAIT` vs `read`). Would need a sibling `FenceSignal` primitive. |

### Effort estimate

~**1.5–2 weeks** for a basic preview-frame-dump PoC. Roughly 4× task
21 because:
- multi-binder graph (ICameraService → ICameraDevice → ICameraDeviceUser
  + ICameraDeviceCallbacks + IGraphicBufferProducer per Surface),
- BufferQueue protocol w/ producer-side fence wait,
- runtime-permission flow integration,
- callback Bn server for `ICameraDeviceCallbacks` (errors, state changes).

### Risk register

- **Bn callback recursion** — `ICameraDeviceCallbacks.onCaptureResult` may
  arrive while we're still inside an `IGraphicBufferProducer.dequeueBuffer`.
  rsbinder's single-thread tokio runtime pattern may deadlock; might
  need a multi-thread runtime.
- **Pixel-format negotiation** — vendor HAL may only expose specific
  formats (YUV_420_888, RAW16). Conversion to RGB for our skia path
  is non-trivial.
- **`AttributionSourceState` again** — `ICameraService.connectDevice`
  takes one. Task 21 B5 found the empty-stub works because the
  service auto-fills pid/uid; same trick should apply here.

## Codec2

| Layer | Type | Service |
|---|---|---|
| Software codecs (CPU) | **Stable AIDL** ✅ | `android.hardware.media.c2.IComponentStore/software` |
| Legacy SW codec2 HIDL | HIDL | `android.hardware.media.c2@1.0/1.1/1.2::IComponentStore/software` |
| **Hardware codecs** | **HIDL OMX only on this device** | `android.hardware.media.omx@1.0::IOmx/default` |

Notable: **no hardware codec2 service registered.** Only `/software` is
exposed via stable AIDL. Hardware codecs still go through legacy OMX
HIDL on the Pixel 2 XL. Newer devices (Pixel 6+) register
`/default` (HW) too. For a SW-only PoC (h264 decode @ SD or lower, ~640x360
at 30 fps doable on a 2017 SoC) we can stay 100% AIDL. HW acceleration =
HIDL OMX = significant extra work.

Permissions: no app-level perm needed for decode (audio/video data is
your own). SELinux gates `untrusted_app → hal_codec2_*_default` but
the same `setenforce 0` workflow we use for haptics works.

Data plane: input/output buffers are `C2Block`s = `(fd, offset, size)`
tuples passed via `IConfigurable`. This is **exactly the
SharedFileRegion shape A2 was built for** — direct fit. Signaling is
batched binder callbacks (`onWorkDone`) rather than per-frame; the
task 20 BnEventQueueCallback pattern covers it.

### Primitive reuse — task 21

| Primitive | Fits codec2? |
|---|---|
| `BinderMappedMemory` (A2) | ✅ **Direct 1:1 fit.** C2Block ≡ SharedFileRegion. |
| `EventfdSignal` (A3) | ❌ Binder callbacks, no eventfd. |

### Effort estimate

~**1 week** for hello-world decode (`encoded h264 bytes → decoded YUV
frames`). Lighter than camera because:
- one IComponent per decoder (flat binder graph, no sub-binders),
- input is bytes from a file, not a live BufferQueue,
- no permission flow.

### Risk register

- **Codec2 HAL version drift** — v1.0 → v1.3 have wire-incompatible
  changes. Need to pick a target version matching what
  `IComponentStore/software` actually implements on this device
  (probably v1.2 per `lshal`).
- **WorkBundle recursive parcelables** — at a quick glance some of
  the codec2 parcelables look recursive in the same way
  `AttributionSourceState.next` is. rsbinder-aidl 0.7.0 chokes on
  `Vec<Box<Self>>`. Workarounds from the
  `feedback_rsbinder_aidl_recursive` memory apply.
- **YUV → RGB conversion** — same problem as camera. Skia has
  `MakeFromYUVATextures` but our wasi guest doesn't.

## Cross-cutting findings

1. **A3 (`eventfd_signal`) does not get a second customer.** Both
   camera (sync_file fences) and codec2 (binder callbacks) use
   different signaling. The "reusable for camera + codec2" rationale
   from task 21 B1 was wrong. A3 is still small (~160 lines incl.
   tests) and might serve a future service, but worth flagging that
   its first follow-on consumer didn't materialize.

2. **A2 (`binder_shared_memory`) gets confirmed twice.** Both camera
   (gralloc buffers) and codec2 (C2Blocks) use SharedFileRegion-style
   buffer transport. The primitive's shape is right.

3. **Both services reach us via stable AIDL.** No HIDL plumbing in
   wart for either, despite the vendor camera HAL being HIDL-only
   on this device. The HIDL boundary is internal to CameraService
   and Codec2 broker.

4. **HW codec acceleration would require HIDL.** Pure software codec2
   is fine for a PoC; HW (which is what real video apps want) means
   talking to OMX HIDL, which is a different rsbinder pipeline build.

## Recommendation (for whoever picks this up)

Codec2 is the lighter target if either becomes a priority. ~1 week
SW-only decode PoC stresses the task-21 binder + shared-memory
pipeline on a genuinely different service without the camera-side
complications (BufferQueue producer/consumer dance, runtime
permission flow, multi-binder graph). It's also a closer reuse of
A2 (the C2Block ≡ SharedFileRegion correspondence is near-exact).

Camera is more product-visible but a much bigger lift (~2 weeks)
and pulls in fence-fd signaling that A3 doesn't cover.

**Neither is on the critical path for the boot-model migration**
(`post-art-roadmap.md` §6.1). They're product-capability additions
queued behind runtime-model decisions in §9 and the boot-model
spike, not architecture unblockers.

## Open questions

- Do we want to extend our WIT surface with `camera` and `codec2`
  interfaces at all? They're orthogonal to "replace ART", and a
  Compose app on wart can ship perfectly well with no camera or
  video-decode access for v1.
- If yes, which first? See recommendation above; codec2 is
  technically easier, camera is more user-visible.
- HW codec acceleration question: accept SW-only for v1 (simple,
  AIDL-only) or invest in HIDL OMX integration for performance?
  This is a separate scope question worth its own analysis.
