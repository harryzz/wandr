---
name: reference_media_codec_strategy
description: "Media codec strategy: OS-native (MF/VideoToolbox) vs bundled C codecs vs GStreamer, and why Servo's media layer is NOT reusable. Read before adding a desktop codec backend or reacting to Win/mac codec CI failures."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 25b6eb4c-9122-4870-8734-7e515af11a68
  modified: 2026-07-23T07:29:05.062Z
---

Researched 2026-07-23 when Windows/macOS CI broke on the C-source codec builds
(libde265, dav1d) and we asked whether OS-native media APIs could replace bundled
codecs. Two source-grounded agent studies; conclusions:

## The codec matrix wandr has
- Android: **MediaCodec** (NDK) — HW decode-to-Surface, zero-copy. PRODUCTION.
- Linux: **VA-API** (cros-codecs/cros-libva) — HW decode, dma-buf→EGLImage→GL
  zero-copy. Just shipped (task 117). Needs libgbm-dev at build+runtime (cros
  links gbm-sys even though we use VA surfaces).
- Desktop SW fallback: libvpx (VP8/9), openh264 (H.264), libde265 (H.265, LGPL,
  "third tier"), dav1d (AV1) — all built from C source. libde265 + dav1d FAIL on
  Windows (cl.exe / dav1d pkg-config) and macOS arm64 (libde265-sys globs x86 SSE
  .cc with no target_arch gate). oxideav-h265 (pure Rust, MIT) also present and
  builds everywhere.

## OS-native APIs exist but are NOT "auto" in Rust
- Windows **Media Foundation** (MFTs on D3D11) and macOS **VideoToolbox**
  (VTDecompressionSession → CVPixelBuffer) are the MediaCodec/VA-API peers: OS
  decodes incl. HW, no bundled codec. BUT the Rust bindings are RAW: `windows`
  crate = raw COM (~400-700 lines unsafe per codec path; H.264 in-box, HEVC/VP9/
  AV1 gated behind Store extension packs — must PROBE); `objc2-video-toolbox`
  0.3.2 = raw objc (~200-400 lines, no COM, cleaner). NO higher-level wrapper
  crate for either. Nobody in Rust ships a native MF/VT decode-zero-copy backend
  to copy — the only native-HW reference code is FFmpeg/GStreamer (C).
- Zero-copy: Windows MF→GL is realistic IF Skia runs on ANGLE-D3D11 same-device
  (EGL_ANGLE_d3d_texture_client_buffer, NV12). macOS VT→GL is clean ONLY on
  Skia-Metal (CVMetalTextureCache→MTLTexture); any GL flavor is deprecated/ANGLE-
  IOSurface. So macOS zero-copy really means "decide to run Skia-Metal".

## The only cross-platform "auto" is GStreamer — and it's HEAVIER, not lighter
- `gstreamer-rs` = safe Rust over the GStreamer **C runtime**; auto-plugs native
  HW decoder elements per OS (d3d11*/vt*/va*/amc*) + zero-copy-to-GL. Real,
  maintained, used by Servo + Slint's HW example. But it BUNDLES the GStreamer C
  runtime (LGPL core + per-plugin licenses to audit, heavy Android/UWP packaging)
  — a bigger dep than the C codecs we're escaping. It's a STRATEGIC swap ("exit
  codec-build maintenance wholesale"), not a quick win.

## ‼️ Servo's media layer is NOT reusable (checked the actual code)
components/media (crates.io servo-media* 0.4.0, MPL-2.0) is a browser
MEDIA-ELEMENT/WebRTC/WebAudio PLAYER, not a codec backend. Its `Backend` trait
mints players: `create_player(...) -> dyn Player` where Player is HTMLMediaElement
(`push_data(Vec<u8>)` with NO PTS, `end_of_stream`, `seek`, `buffered`,
`set_playback_rate`). The GStreamer backend is playbin3+decodebin3 — OWNS demux,
A/V sync, buffering, audio out. `can_play_type(mime)` is HTMLMediaElement.
canPlayType, not a codec registry. There is NO open_decoder / decode(chunk,pts) /
codec enumeration anywhere.
- ‼️ CLINCHING PROOF it's the wrong layer: OpenHarmony ships BOTH OH_AVCodec
  (low-level codec) AND OH_AVPlayer (high-level player). Servo's `ohos` backend
  wraps **OH_AVPlayer** — the trait forced player-level even when codec-level was
  available. It's also not even zero-copy (CPU NV12→BGRA blit), and implements
  only create_player + can_play_type; everything else is `todo!()`.
- Coupling: servo-media-gstreamer → servo-base → webrender_api + ipc-channel +
  MallocSizeOf. Pulls a slice of the browser into the host.
- Reusable bits: only the generic `auto` build-time selector + `dummy`
  conformance skeleton (we already have a priority registry). And
  `backends/gstreamer/render-unix/lib.rs` as a CODE REFERENCE for the zero-copy
  appsink→GL-texture tail — but it's ~100 lines of stock gstreamer-gl, and our
  VA-API dma-buf→GL path already does the equivalent.

## DECISION (2026-07-23)
Our codec-level, guest-demuxes-first design is RIGHT; no higher-level abstraction
fits it. For the CI failure: GATE libde265 + dav1d to Linux (where the C builds
are proven and desktop perf matters); Windows/macOS keep libvpx + openh264 +
oxideav-h265 (pure-Rust H.265), losing only AV1 SW decode on those DEV/build-check
platforms. HW decode is already met where it matters (Android + Linux). Native
MF/VideoToolbox backends = LATER, optional, VideoToolbox-first and only alongside
a Skia-Metal macOS backend. GStreamer only if the goal becomes exiting codec-build
maintenance wholesale. See [[reference_vaapi_zerocopy_real_players]],
[[reference_libvpx_wandr_video]].
