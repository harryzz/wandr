# Task 117 — Permissive HW video backends (drop the FFmpeg dependency on desktop)

> Status: 🔲 NOT STARTED — research done 2026-07-19, recorded here so it isn't lost.
> Prereq for shipping redistributable desktop binaries (see task 118).

## Why

The desktop `wandr:video` backend (`runtime/wandr-host/src/video_desktop.rs`, 561 lines)
uses FFmpeg for VP8/VP9 encode+decode and YUV↔RGB scaling. Two problems:

1. **Licensing.** FFmpeg is LGPL-2.1-or-later, but nearly every distro builds it
   `--enable-gpl` (verified: this machine's is), which makes *that build* GPL. Shipping
   binaries against it is legally murky. wandr is Apache-2.0.
2. **Distribution.** Linking system FFmpeg binds the binary to one soname — the
   `libavutil.so.58` failure seen when running a CI artifact locally; on macOS Homebrew
   bottles pin a minimum OS; on Windows a *release* ffmpeg is required (BtbN
   `master-latest` fails to compile: `AVCodec::pix_fmts` removed post-8.0).

**Important:** FFmpeg is NOT the source of hardware acceleration — the OS/GPU driver is.
FFmpeg is a portable wrapper. Going direct to the OS APIs removes the third-party media
licence entirely AND keeps HW paths. That is already the model on Android (MediaCodec,
task 93).

Note also: desktop is **software-only today** — it selects `"libvpx"` / `"libvpx-vp9"`,
with zero HW plumbing (no `hwaccel`, `hw_device_ctx`, VAAPI, VideoToolbox, D3D11VA).
HW encode/decode on desktop is unimplemented, not merely un-accelerated.

## Research (2026-07-19) — is there a "cpal for video"?

**No.** Nothing has cpal's maturity *and* cross-platform coverage. But good permissive
per-platform crates exist, so this is wiring, not writing codecs:

| Platform | Crate | Licence | Downloads | Notes |
|---|---|---|---|---|
| Linux | **cros-codecs** | BSD-3 | 1.47M | ChromiumOS. HW encode AND decode. **Linux-only** (VA-API/V4L2; deps drm/gbm/v4l2r) |
| Linux | cros-libva | BSD-3 | 714K | raw VA-API bindings (crosvm) |
| macOS | **videotoolbox** | MIT/Apache | 1.2K | maintained, macOS only |
| Windows | **windows** crate | MIT/Apache | huge | Media Foundation / D3D11VA |

Rejected: **oxideav** (MIT) claims all backends but has **27 downloads** — an experiment,
not a dependency. **avio** (582 dl) *wraps FFmpeg*, so it changes nothing about licensing.

**video-rs** (MIT/Apache, **301K downloads**, maintained since 2022) deserves a specific
note because it looks like the answer and is not:
- It is a high-level API *over* `ffmpeg-next` (pins `=8.0.0`) — so it inherits every
  licensing and distribution problem above unchanged. Ergonomics, not independence.
- HW **decode** only: `Cuda, D3D11Va, Drm, Dxva2, MediaCodec, OpenCL, Qsv, Vdpau,
  VideoToolbox` (`src/hwaccel.rs`). `encode.rs` has **zero** hwaccel references, and
  **VAAPI is absent** — the mainstream Linux path for VP8/VP9 on Intel/AMD.
- Signal calls need HW *encode* (camera → VP8 → peer), which is exactly the gap.
Worth revisiting if HW-accelerated *playback* is ever wanted: it beats hand-rolling
`ffmpeg-next` decode paths, and its binding licence is friendlier than ffmpeg-next's WTFPL.

Also permissive if a software fallback is wanted: libvpx (BSD-3, VP8/VP9), libyuv (BSD-3,
replaces swscale), dav1d (BSD-2) / SVT-AV1 / rav1e (AV1). None give HW accel.

## Plan

1. Keep the `wandr:video` WIT unchanged — it is already the abstraction; only the backend
   swaps. `video_desktop.rs` is the single file to fork per platform.
2. Linux first (`cros-codecs`): VP8/VP9 HW encode+decode, matching what Signal calls need.
3. macOS (`videotoolbox`), Windows (`windows`/MediaFoundation).
4. Replace `ffmpeg::software::scaling` with libyuv or a Rust YUV crate.
5. Feature-gate the whole video backend so a plain desktop `cargo build` needs NO external
   media library at all (`image` handles `decode-image` independently — verified).
6. Keep FFmpeg as an optional software fallback for formats/paths the HW backends miss.

## Trade-off (be honest about it)

FFmpeg's real value is the long tail — containers, odd camera pixel formats, software
paths everywhere, robustness. Three native backends have different capability matrices and
no universal fallback. Do NOT remove FFmpeg wholesale until the native paths are proven on
real hardware; make it optional instead.
