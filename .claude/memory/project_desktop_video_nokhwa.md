---
name: project_desktop_video_nokhwa
description: Desktop wandr:video backend = nokhwa camera + ffmpeg VP8/VP9; WSLg camera truncation + ffmpeg-next feature gotchas
metadata: 
  node_type: memory
  type: project
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

Desktop (Linux/WSLg/Win/mac) backend for `wandr:video` (encoder + decoder) =
**nokhwa** (camera) + **ffmpeg-next** (software VP8/VP9 via libvpx), in
`runtime/wandr-host/src/video_desktop.rs` — the cross-platform peer of the
Android NDK-camera + AMediaCodec backend (`video.rs::android`). Fills the
`#[cfg(not(target_os="android"))]` `VideoEncoder`/`VideoDecoder` (were
`CodecInitFailed` stubs). **device-verified: `wandr.video.test` ALL PASS on
desktop** — Part 1 (camera→VP8→decode loopback) + Part 2 (camera→encoder→
SRTP/real-UDP→decoder through the wandr-call engine, ICE+DTLS-SRTP, 0 broken).
Mirrors [[project_desktop_audio_cpal]] (the cpal analog). Roadmap: nokhwa/cpal/
ffmpeg = the cross-platform A/V trio; H.264/H.265 (ffmpeg has libx264/5) for a
future movie player.

**Load-bearing gotchas (each cost real time):**

1. **WSLg RDP-forwarded camera TRUNCATES large buffers → use 640x480 MJPEG.**
   1280x720 MJPEG tears (bottom ~60% gray = truncated JPEG); 1280x720 **YUYV is
   WORSE** (mostly green/empty — 1.8 MB/frame truncates harder). 640x480 MJPEG =
   complete frames @ ~15 fps (10 fps over the pipe). 640x480 is the call
   resolution anyway. Real cameras (device/native Linux/Win/mac) handle 720p+ —
   the truncation is specific to WSLg's virtual-camera forwarding. Verify a frame
   by eye (Read the PNG) — fps/counts alone hide tearing.

2. **nokhwa 0.10.11, feature `input-native`** (v4l2/MediaFoundation/AVFoundation)
   — pure-Rust `v4l` on Linux, **no sudo, no system libs**. `RequestedFormat::
   new::<RgbFormat>(Closest(CameraFormat::new(Resolution, FrameFormat::MJPEG,
   fps)))`; `camera.frame()?.decode_image::<RgbFormat>()` → RGB.

3. **ffmpeg-next 7.1, `default-features=false, features=["codec","format",
   "software-scaling"]`.** Match the crate major to system ffmpeg (7.1 =
   libavcodec 61). Default features pull libavfilter/libavdevice (extra `-dev`
   headers you likely lack) — disable them; but `format` (libavformat) is NOT
   optional (ffmpeg-next uses AVIOInterruptCB unconditionally). Needs `sudo apt
   install libavformat-dev libswscale-dev` if partial. VP8 encode =
   `encoder::find_by_name("libvpx")` (VP9="libvpx-vp9"); decode =
   `decoder::find(codec::Id::VP8)`; RGB24→YUV420P via `software::scaling::Context`;
   force keyframe = `frame.set_kind(picture::Type::I)`; `ffmpeg::ffi` re-exports
   the sys crate.

4. **`unsafe impl Send` for the encoder/decoder** — nokhwa Camera + ffmpeg
   contexts are `!Send`, but `ResourceTable::push` needs `Send`; the store is
   single-threaded on desktop. Android does the same (raw camera/codec pointers).

5. **Desktop `--run-once <app-id>` now works** (was android-only). Un-gated
   `run_once` (cfg the android-only android_logger/wasi_stderr/sf_surface/binder
   bits) + added `SkiaRenderer::new_headless(w,h)` (CPU raster surface, all of
   gl/sb_surface/window = None) so a `wasi:cli/command` guest that never draws
   satisfies HostState's non-Option renderer. `WANDR_APPS_ROOT=... wasm-android-
   host --run-once wandr.video.test`.

Scope shipped = OUTGOING encoder (camera→VP8) + decode-to-BUFFER (frame counts)
+ **PiP self-view compositing** (the encoder's local camera drawn on-screen).
Compositing model (no gstreamer/wgpu — reuse the host's Skia): the encoder pushes
its latest camera frame as RGBA into a thread_local `PREVIEWS` registry
(keyed per encoder, rect+visible from `set_preview_*`); `composite_previews(canvas)`
draws each with `skia_safe::images::raster_from_data` + `draw_image_rect`,
mirrored (self-view), and is called from the wasi:canvas host `present()` AFTER
the guest UI (= above-ui) before `flush_and_swap`. Android instead uses a
SurfaceView child surface. Verify: `wasm-android-host --camera-shot <out.png>`
(opens the camera encoder w/ a PiP rect, composites over a fake UI onto a
headless raster surface via `SkiaRenderer::new_headless`+`snapshot_png`, writes
PNG). STILL follow-up: remote **decode-to-surface** (incoming video composite) —
`set_rect`/`set_visible` on the decoder are still no-ops (decode-to-buffer only).
De-risk repro: `repros/nokhwa-camera-probe` (camera→VP8→decode, standalone).
