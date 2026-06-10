# Task 93 — Video calls (AV gap to a full Signal app)

> Status: ✅ HW CODECS BOTH WAYS PROVEN under `--no-art`. Camera capture reliable
> (2026-06-08: 29.1fps raw + HW VP8 encode 17.4fps, 3/3; task-95 EIS-gyro race won).
> **HW VP8 DECODE now also confirmed** (2026-06-10, `--probe-video decode` loopback):
> camera → HW VP8 encode (20.1fps) → HW VP8 decode (19.9fps, 100/101 frames; the
> decoder `configure()` did NOT block — the one open unknown) → full round-trip works
> ART-off. WIT contracts ✍️ **DRAFTED + validated**: `wit/video.wit` (`wandr:video`)
> + `wit/crypto.wit` (`wandr:crypto`), commit `697da5de`. **Audio calls connect both
> ways (incoming fixed).** Remaining = pure integration, no `--no-art` blockers — see
> **IMPLEMENTATION PLAN** below. **ALL 5 PHASES ✅ DONE — TASK COMPLETE
> (2026-06-10): real Signal VIDEO CALLS work both ways on the Pixel 2 XL under
> `--no-art` (camera→HW VP8→SRTP→peer and back), with TWCC/REMB-adaptive
> bitrate, CVO rotation incl. live device rotation, aspect-fit display, and a
> call-screen UI. Verified live against a real Signal client; user-confirmed.
> Follow-ups (cosmetic): mirrored self-view, behind-ui hole once dioxus-canvas
> grows a clear blend, full 4-pose landscape matrix validation.**

## IMPLEMENTATION PLAN — ready to build (next session)

All the `--no-art` unknowns are now de-risked: camera opens + delivers frames, HW
VP8 **encode AND decode** both run (≈20 fps round-trip, `video_probe.rs` decode
loopback 2026-06-10), the codec `configure()` blocks are gone (task-96 shim), and
1:1 audio calls connect both directions. What's left is our integration code, in
five buildable phases (each independently testable):

**Phase 1 — host `wandr:video` impl (encoder + decoder). ✅ DONE + DEVICE-VERIFIED
(2026-06-10).** Promoted `video_probe.rs` to the real host component:
`runtime/wandr-host/src/video.rs` (NDK backend — shared `ndk` FFI the probe now
also rides; camera picked by new `camera-facing` WIT field via ACAMERA_LENS_FACING,
fallback first id) + `video_host_impl.rs` (WIT resources in `HostState.table`) +
`video-host` bindgen world + `add_to_linker` at both app_loader sites. Encoder:
camera → input-surface (0×0 geometry fix) → HW VP8, guest pulls `next-frame()`
(non-blocking dequeue, CSD filtered, µs→90kHz ts); `request-keyframe`/`set-bitrate`
via `AMediaCodec_setParameters`. Decoder: decode-to-buffer (Phase 4 = to-surface);
`submit` returns new `queue-full` error when input bufs exhausted; new
`decoded-frames()` diagnostic. Threadpool (`sf_start_binder_threadpool`) starts
LAZILY at first open behind a `Once` (post-fork safe). ‼️ Clean teardown = ordered
null-tolerant `Drop` (resource drop + store drop both reach it). **Verified by
`apps/user/wandr.video.test`** (`wasi:cli` guest, `--run-once`): **25.0 fps**
camera→encode→guest→decode loopback, 125 pulled / 125 submitted / 124 decoded,
ready=true, first-frame 90 ms, mid-run request-keyframe produced a 2nd keyframe +
set-bitrate(500k) dropped avg frame 4.8K→3.4K; **ran 2× back-to-back clean**
(teardown does NOT wedge cameraserver); probe regression (`--probe-video decode`)
still passes (24.9 fps); chrome stack unaffected by the new linker import.

**Phase 2 — host `wandr:crypto` impl (SRTP AEAD offload). ✅ DONE + LIVE-CALL-VERIFIED
(2026-06-10).** Built bigger than sketched: universal `wandr:crypto` WIT (hash/mac/
aead/cipher/kdf + caps; `--probe-crypto` 13/13 vectors, HW AES via `--cfg aes_armv8`
rustflags). SRTP integration exactly as planned: only the AEAD primitive crosses
(`aead-key` resource — key schedule expanded once host-side); ROC/replay/KDF stay in
the guest (`rtc-srtp` `external-aead` feature in `wandr-rtc.patch` + wandr-call
`host-aead`, all trait-injected, WIT stays out of the libs). Device-measured
(`wandr.srtp.bench`): audio 160B 3.0×, video-size 1100B **8.2–8.7×** (24→2.8 ms
CPU/s) — the video critical path is cleared. Real Signal call verified, audio both
ways. Commits 11c5efaa/266e6834/3c316895. Gotcha for the next phases: after host
linker changes RESTART THE ZYGOTE (stale-image trap, `repros/wac-resource-import/`).

**Phase 3 — wandr-call video track. ✅ DONE + DEVICE-VERIFIED (2026-06-10).**
New `crates/wandr-call/src/video.rs` (`VideoStream`): VP8 RTP track on the SAME
SRTP contexts as audio (own SSRC) — RFC 7741 payloader (PictureID on, 1172 B
payload budget = 1200 − RTP 12 − GCM tag 16) out; depacketize + in-order frame
reassembly (S-bit/ts keyed, torn frames dropped, never reach the decoder) in.
RTCP over the same contexts (SRTCP, RFC 5761 byte-1 demux 192–223; works through
the Phase-2 host-AEAD cipher too): **PLI/FIR in** → `take_keyframe_request()` →
guest calls encoder `request-keyframe`; **PLI out** = `request_keyframe()` +
AUTO-PLI on any inbound loss (rate-limited 300 ms — VP8 has no recovery but a
keyframe; RTX PT 118 ignored); **~1 Hz video SR out** + peer SR surfaced
(`peer_sender_report` = the A/V-sync NTP⇄RTP anchor); **REMB parsed** →
`peer_remb_bps()` (v1 congestion control = fixed bitrate + feed REMB to
`set-bitrate`; `external/rtc` has the RTCP types but no BWE engine — full
TWCC/BWE deferred). **ringrtc wire constants grounded in source**
(signalapp/webrtc rffi `peer_connection.cc`): VP8 PT **108**, RTX 118, VP9 109,
SSRCs = BASE(offerer 1000/answerer 2000)+2 audio/+3 video/+13 RTX → our video
SSRC 1003/2003 (and the old captured peer audio ssrc 0x7d2=2002 now explained).
`PeerSession`/`SignalCall` API: `send_video(frame, ts90)` / `recv_video()` /
`take_keyframe_request` / `request_keyframe` / `peer_remb_bps` / `video_diag`.
3 new engine tests (round-trip w/ fragmentation, PLI+SR cross the wire,
lost-fragment→drop+auto-PLI) — 21/21 pass. **Device capstone** (`wandr.video.test`
Part 2): camera → HW VP8 → PeerSession A → SRTP over real wasi:sockets UDP →
PeerSession B → reassembled **88/88 frames, 0 broken** → HW decode **17.4 fps**;
PLI round trip answered on-wire; SR anchor received; 2× runs clean.

**Phase 4 — render: decode-to-surface + PiP self-view. ✅ DONE + DEVICE-VERIFIED
(2026-06-10, screenshot-proven: live camera video composited on the panel).**
DESIGN DECISION (replaces the sketched arbiter `Role::Video`): video surfaces are
**children of the app's own SF surface — Android's SurfaceView model** (`sf_media_*`
in the libgui shim: container+buffer-state-child+BBQ, geometry on the container
with a scale matrix). Z/visibility/rotation/occlusion are inherited from the app's
existing role for FREE — backgrounding the app hides its video; zero new arbiter
state/IPC. Remote video child z=-2, PiP z=-1, both BELOW the app's UI buffer; the
host clears `eLayerOpaque` while a decoder surface is up (`sf_set_opaque`) so the
guest's transparent hole blends (controls can overlap video). Headless
(`--run-once`) processes parent to the bufferless run-once fullscreen surface —
the same tree, screenshot-verifiable. Decoder = decode-to-surface
(`releaseOutputBuffer(render=true)`; empty rect = Phase-1 buffer mode); encoder
gained the **PiP self-view** (camera streams to a second media surface — a second
`ACaptureSessionOutput`/target on the same session; frames never cross the WIT).
WIT: `preview: option<video-rect>` + `set-preview-rect/visible`, decoder
`set-visible`; rects are GUEST-SURFACE pixels. Verified end-to-end on device:
camera → HW VP8 → wandr-call SRTP/UDP → HW decode → **panel** (~17–30 fps, runs
back-to-back clean), live `set-rect`/`set-preview-rect` moves on a hot codec,
buffer→rect scaling (640×480 → 1280×960). GOTCHAS: (1) `Surface*` must be
explicitly upcast to `ANativeWindow*` before crossing the C ABI as `void*` —
missing base-pointer adjustment SIGSEGVs camera2ndk; (2) shim rebuilds on a-03:
re-run ninja ~3× (unrelated `libbinder_ndk` lsdump step fails the build AFTER our
.so links — check the artifact mtime); (3) a desk-down BACK camera encodes pure
black — use `facing: front` for visual tests; (4) sensor-orientation rotation is
NOT yet compensated (image arrives sensor-rotated; fix via container matrix or
encode-side rotation in Phase 5). REMAINING (moved to Phase 5, needs the in-call
video-enable signaling to drive it): wire the Signal call UI to remote-video +
local-PiP rects + transparent hole.

**Phase 5 — Signal-protocol video. ✅ DONE + LIVE-CALL-VERIFIED (2026-06-10:
real Signal peer, video both ways, adaptive quality, correct rotation —
user-confirmed 'now is ok').** Grounded in ringrtc
source: video on/off rides the RTP-data channel (`rtp_data.Message.senderStatus.
video_enabled`, accumulated + resent ~1 Hz — NOT signaling); the peer's
`receiverStatus.max_bitrate_bps` = its requested send bitrate (wins over REMB);
rotation rides the CVO header extension (`urn:3gpp:video-orientation`, ringrtc
ext id **4**) — outgoing = the camera's facing-adjusted sensor orientation
(`display-rotation()`), incoming = applied at decoder open (MediaCodec
`rotation-degrees`). **Receive advert is VP8-ONLY now** (the peer encodes from
OUR set; VP9-first would get us VP9 we don't depacketize). WIT gained `z-layer`
(`behind-ui` hole model / `above-ui` — the Signal screen uses above-ui; no
clear-blend in dioxus-canvas yet). Signal engine: `pump_video` beside
`pump_audio` (lazy encoder on toggle+layout; decoder on first keyframe with its
CVO rotation; keyframe-gated mid-stream join; in-band hangup honored). UI:
camera button in the in-call header → full-screen `VideoCallScreen` (rects
derived from surface size, reported via `set-video-layout`; camera toggle +
hang-up). Deployed state-preserved. **TO VERIFY LIVE:** 1:1 call ↔ real Signal
client, camera on both ways; watch `/state/calldbg.log` `video:` lines.
LIVE-CALL HARDENING (the second day's worth, all live-verified): RED demux
(peer wraps VP8 in RFC-2198 RED, PT 120); TWCC receiver feedback (~10 Hz —
a transport-cc peer's BWE ramps ONLY on it; without it it parks at ~36 kbps);
REMB death-spiral floor (never follow the peer's estimate below 250 kbps —
sending at the floor is the probe); receiverStatus/REMB receive-budget
adverts; aspect-fit from the peer's VP8-keyframe coded dims; and THE ROTATION
MECHANISM: layer setTransform is overwritten per-buffer by BLASTBufferQueue,
producer-window buffers-transform is overwritten by MediaCodec itself — the
ONLY producer-proof place is the CONTAINER's transform matrix
(sf_media_set_geometry: pre-rotation crop box + matrix+position into the
final rect). CVO wire formula = libwebrtc getFrameOrientation (front:
(sensor+dev)%360), folded with live device rotation from the arbiter's
geometry push. Hit the STALE-ZYGOTE trap mid-iteration (pushed host without
stack restart → app forked the old image → HAL enums hit the degrees shim →
identity matrix). KNOWN LIMITS: self-view not mirrored; landscape poses
validated mechanically (matrix+map) but not face-confirmed in all 4 combos.

**Next step (Phases 1–4 ✅ done): Phase 5** — Signal-protocol video: in-call
video enable/disable over the `opaque` ConnectionParameters (wire constants
grounded: VP8 PT 108, SSRC 1003/2003), the Signal call screen (remote-video +
local-PiP rects + transparent hole + controls), and camera sensor-rotation
compensation.

**Residual risks (post-decode-confirmation):** SRTP CPU at video bitrate (→ Phase 2
mandatory); PLI/keyframe + congestion control (→ Phase 3, the WebRTC bits audio
skipped); decode-to-surface + `Role::Video` compositing (new but lower-risk now that
HW decode is proven); ~20 fps is the codec ceiling on this 2017 SoC and combined
live-call load (camera + 2× codec + crypto + Skia + net) may thermal-throttle.

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
