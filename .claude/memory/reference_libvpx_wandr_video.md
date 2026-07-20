---
name: reference_libvpx_wandr_video
description: "libvpx/wandr-video gotchas (task 117) — 4 traps that produce plausible-looking video or platform-only build breaks, and why the crate is desktop-only"
metadata: 
  node_type: memory
  type: reference
  originSessionId: ed607dbd-a38a-49fd-b8ec-bfd92f150821
---

Task 117 replaced FFmpeg with statically-linked libvpx (BSD-3). Shipped as
`runtime/wandr-host/crates/wandr-video` (codec dispatch) + `crates/wandr-vpx-sys`
(own Apache-2.0 bindings; its build.rs compiles `vendor/libvpx` v1.16.0 into
`OUT_DIR`, or honors `VPX_LIB_DIR`). Verified on Linux, macOS x86_64/aarch64,
Windows (Signal video call works), Android unaffected.

**`wandr-video` is DESKTOP-ONLY.** Android does HW encode AND decode via MediaCodec
and must never link a codec library, so the crate sits in the
`cfg(not(target_os = "android"))` table where `ffmpeg-next` was, and `src/video.rs`
was left untouched. The crate owns only a *codec* vocabulary (`Codec`, `CodecError`,
`EncoderParams`, `DecoderParams`, `Packet`); the host keeps its WIT-shaped types and
`video_desktop.rs` maps at the boundary. Camera facing / preview rects / z-layer are
host concerns — putting them in a codec crate is what makes codec abstractions rot.

## Four traps

1. **`rc_target_bitrate` is KILOBITS/s** — ffmpeg's `set_bit_rate` took bits/s. A
   missing `/1000` is a 1000× bitrate. Sanity check: bytes/frame × fps × 8 ≈ target.
2. **Colorspace must be BT.601 + `YuvRange::Limited` on BOTH directions** (swscale's
   default for RGB24↔YUV420P, and VP8/VP9's default range). Defined ONCE in
   `convert.rs` — never inline it.
3. **`vpx_enc_frame_flags_t` / `vpx_codec_flags_t` are C `long`** → 64-bit on LP64
   (Linux/macOS), **32-bit on LLP64 (Windows/MSVC)**. A hand-written `i64` constant
   compiles on Linux and fails Windows CI with E0308. Never hand-write these widths:
   use bindgen's generated constants spelled via its typedefs. (`vpx_codec_frame_flags_t`
   is `u32`, so the keyframe test is safe.) Bare integer literals infer per-target fine.
4. **`mem::zeroed()` on `vpx_codec_enc_cfg_t` is UB** (niche field) and aborts under
   rustc's zero-init check. Use `MaybeUninit` + `vpx_codec_enc_config_default`.

Traps 1–3 **do not error** — they produce well-formed packets and plausible-looking
video. So `tests/roundtrip.rs` asserts on decoded PIXELS (MAE vs source), with an
**empirically measured** threshold: correct 1.68, BT.709 mixup 7.93, full-range mixup
9.36 → bar 4.0. A guessed threshold of 20 passed both bugs. If a codec/crate upgrade
moves the baseline, RE-MEASURE by injecting the bug; don't just raise the number.
See [[feedback_change_detection_test_primitive]], [[feedback_humility_proven_vs_guessed]].

## Build notes

* Needs `nasm` (or yasm) for x86 SIMD. Without it libvpx **silently** builds pure-C
  with badly degraded realtime encode — so the build script hard-fails instead.
* Windows: vcpkg `libvpx[core,realtime]:x64-windows-static-md`. The triplet is the
  whole game — `-static-md` = static lib + *dynamic* CRT (`/MD`) matching rustc's msvc
  target; `-static` (`/MT`) gives LNK4098; plain `x64-windows` gives a `vpx.dll` and
  reintroduces the runtime-DLL problem the task removed.
* Debug builds report `FREEZE/STALL` in `--video-selfview-test` (~42 fps). That is a
  **debug artifact** — release is ~58-59 fps. Don't chase it.
* macOS cameras negotiate **YUYV, not MJPEG** (the format-fallback loop matters), and
  encoder open takes ~400 ms there vs ~5 ms on Linux — AVFoundation/TCC, not libvpx.

## Behavior changes vs the ffmpeg path

* `set_bitrate` is **real** now (`vpx_codec_enc_config_set`) — desktop finally honors
  the guest's REMB/TWCC congestion control. It was a silent no-op before.
* A camera frame whose size differs from the encode size is **resized**, not dropped.
* `rc_end_usage` is CBR (ffmpeg defaulted to VBR) — deliberate for call traffic.

Related: [[project_desktop_video_nokhwa]], [[project_wandr_video_host]],
[[project_wandr_call_video_track]].
