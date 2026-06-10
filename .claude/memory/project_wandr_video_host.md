---
name: project_wandr_video_host
description: "Task 93 Phase 1 ✅ wandr:video host impl (camera→HW VP8 encode + decode-to-buffer) device-verified 25fps through the WIT boundary; gotchas — lazy post-fork binder threadpool, ordered null-tolerant teardown, by-value bindgen configs"
metadata: 
  node_type: memory
  type: project
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**✅ Task 93 Phase 1 DONE + DEVICE-VERIFIED (2026-06-10).** `wandr:video` is a real
host interface, linked onto EVERY guest (both `app_loader.rs` linker sites, beside
`CryptoHost`). Layout mirrors [[project_wandr_crypto_srtp_offload]]:

- `runtime/wandr-host/src/video.rs` — NDK backend; its `pub mod ndk` is the ONE
  home of the camera2ndk/mediandk FFI (`video_probe.rs` now imports it — don't
  re-declare). Desktop builds get stubs (`open` → `codec-init-failed`).
- `runtime/wandr-host/src/video_host_impl.rs` — WIT resources (`EncoderState`/
  `DecoderState` in `HostState.table`, bindgen `with` mapping, world `video-host`).
- `apps/user/wandr.video.test` — `wasi:cli` loopback guest (`--run-once`):
  **25.0 fps camera→HW-VP8→guest→HW-decode**, 124/125 decoded, first-frame 90 ms,
  request-keyframe/set-bitrate observably work, 2× back-to-back = no wedge.

**Non-obvious decisions/gotchas:**
- **Binder threadpool is LAZY** (`ensure_binder_threadpool`, `Once`, called inside
  `open`) — NEVER at host init: threads don't survive the zygote fork; codec opens
  happen post-fork at call time, so the pool lands in the right process.
- **Teardown = ordered, null-tolerant `Drop`** (probe step-7 order: stopRepeating →
  close session → free req/target/container/output → close camera → stop/delete
  codec → release window). A failed `open` unwinds through the same Drop. This is
  what keeps cameraserver from wedging — don't "simplify" it.
- Encoder `next-frame` is a non-blocking poll (timeout 0) that FILTERS
  `BUFFER_FLAG_CODEC_CONFIG` and converts µs PTS → wrapping 90 kHz RTP ts.
- The encoder input-surface **0×0 fix** (`ANativeWindow_setBuffersGeometry` after
  `createInputSurface`) lives in the backend — required or the camera HAL DELetes
  the stream.
- Camera picked by WIT `camera-facing` via `ACameraMetadata_getConstEntry` tag
  `ACAMERA_LENS_FACING=524293` (FRONT=0/BACK=1), fallback = first id. Test guest
  uses BACK (probe baseline); the call self-view will want FRONT.
- **Phase 4 (2026-06-10): decode-to-SURFACE + PiP self-view, device-verified
  (screenshot: live video composited on the panel).** Video surfaces = CHILDREN
  of the app's SF surface (SurfaceView model; `sf_media_*` in the libgui shim —
  container w/ position+scale-matrix + buffer-state child + BBQ at the
  PRODUCER's size). Remote z=-2, PiP z=-1, below the UI buffer; host clears
  `eLayerOpaque` (`sf_set_opaque`) while video is up so the guest's transparent
  hole blends. NO arbiter `Role::Video` — z/visibility/rotation inherit from
  the app's role for free. Headless `--run-once` parents to the bufferless
  run-once fullscreen surface (screenshot-verifiable). Encoder `preview` rect =
  second `ACaptureSessionOutput` on the same session. ‼️ GOTCHAS: `Surface*`
  must be EXPLICITLY upcast to `ANativeWindow*` before crossing the C ABI as
  void* (missing base-pointer adjustment → camera2ndk SIGSEGV in
  `hook_lockBuffer_DEPRECATED`); a-03 shim builds need ninja re-run ~3× (an
  unrelated `libbinder_ndk` lsdump step fails AFTER our .so links — verify the
  artifact mtime before scp!); desk-down BACK camera encodes pure black (use
  `facing: front` for visual tests); sensor-orientation rotation NOT yet
  compensated (Phase 5: container matrix or encode-side rotation).
- Decode-to-buffer mode still exists (empty/None rect — diagnostics;
  `decoded-frames()`). `submit` → `queue-full` error when input buffers are
  exhausted (guest should resend after a keyframe).
- wit-bindgen guest side: record configs pass **by value** (`open(EncoderConfig{…})`),
  only `submit` takes a reference — the first compile error every consumer will hit.
- Qcom VP8 encoder seems to ignore `i-frame-interval` (1 keyframe in 5 s);
  `request-sync` via `AMediaCodec_setParameters` DOES work — Phase 3's PLI handler
  must rely on request-keyframe, not the configured interval.

Runtime prereqs unchanged from [[project_artless_camera]] (4 stubs + sensorservice
+ wandr-sensormanager; `wandr-sensors` must NOT hold the HAL — task 94 unified this).
Next: Phase 3 wandr-call video track ([[project_wandr_call]]).
