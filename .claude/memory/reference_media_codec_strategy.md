---
name: reference_media_codec_strategy
description: "Media codec strategy: OS-native (MF/VideoToolbox) vs bundled C codecs vs GStreamer, and why Servo's media layer is NOT reusable. Read before adding a desktop codec backend or reacting to Win/mac codec CI failures."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 25b6eb4c-9122-4870-8734-7e515af11a68
  modified: 2026-07-23T14:49:29.187Z
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

## VA-API on NATIVE Windows — real, but NOT a reuse path for our stack (2026-07-23)
User asked to check VA-on-Windows ("Microsoft ships VA for Windows", "ffmpeg
already works this way"). Both TRUE — and it's native, not WSL:
- **libva-win32** (libva 2.17, Dec 2022): display via `vaGetDisplayWin32(const
  LUID*)` — a DXGI adapter LUID, NOT a DRM fd; driver name from Windows registry.
- **`vaon12_drv_video.dll`** = Mesa's d3d12 VA frontend on D3D12 Video. Shipped by
  Microsoft as NuGet `Microsoft.Direct3D.VideoAccelerationCompatibilityPack`
  (`LIBVA_DRIVER_NAME=vaon12`). Native ffmpeg `-hwaccel vaapi` decode works via it.
BUT three walls block reusing wandr's Linux VA backend, so DON'T pursue it:
1. **cros-libva is DRM-only, full stop** — `Display::open()` hard-wires
   `/dev/dri/renderD*`; no `vaGetDisplayWin32` path, and NO Rust libva binding
   wraps the Win32 entry points. Windows = fork the binding.
2. **Zero-copy doesn't carry** — Windows `vaExportSurfaceHandle` yields an
   `ID3D12Resource`/NT shared handle, not a dma-buf. Our VA→dma-buf→EGLImage→GL
   chain is Linux-only; Windows needs D3D-texture→ANGLE-D3D11→GL interop = the
   SAME interop Media Foundation needs. vaon12's D3D interop is upstream-flagged
   incomplete (copy fallback "disappointing").
3. **Flaky decode driver** — Mesa 25.0.0 known issues: d3d12-vaapi "thread safety"
   + "Failure to correctly decode H.264"; not marked fixed in 26.0.0. Matches the
   WSL hang I hit (same Gallium binary). See [[reference_wsl_vaapi_d3d12_hw_decode]].
CONCLUSION: VA-on-Windows is a Mesa compat shim for Linux/WSL apps, not a native
media stack. Once you must fork the binding AND build D3D→ANGLE interop anyway,
**Media Foundation** (first-party, stable) is the strictly better Windows target
for the same effort. Windows HW decode = MF, not VA. Nothing to build here.

## Windows HW-decode re-scope: D3D12/DXVA video decode as a cros-codecs backend (2026-07-23)
Two source-grounded confirmations changed the Windows plan from "MF MFT" toward
"reuse our cros-codecs stateless decoder":
1. **cros-codecs is backend-pluggable and NOT VA-coupled** (verified in vendored
   `runtime/wandr-host/vendor/cros-codecs`): per-codec traits
   `StatelessH264DecoderBackend` etc. in `src/decoder/stateless/{h264,h265,vp9,vp8,av1}.rs`
   with `new_sequence(sps)→new_picture()→decode_slice()→submit_picture()`; parsers/DPB
   in `src/codec/*` are backend-agnostic; THREE backends already exist
   (`backend/vaapi`, `backend/v4l2`, `backend/dummy`). A Windows GPU decode backend
   is a 4th impl reusing ALL the (hard, correctness-critical) parser/DPB code — the
   same way our Linux VA path does.
2. **The `windows` crate exposes the full D3D12 Video DECODE API** — CONFIRMED by a
   green `cargo check` on x86_64-pc-windows-msvc (windows v0.62.2, latest). ‼️ GOTCHA:
   the D3D12 Video family lives under `windows::Win32::Media::MediaFoundation`, NOT
   `Graphics::Direct3D12` (win32metadata groups it with MF). Features needed:
   `Win32_Media_MediaFoundation` + `Win32_Graphics_Direct3D12` + `Win32_Graphics_Dxgi_Common`.
   Confirmed types: ID3D12VideoDevice/1, ID3D12VideoDecoder, ID3D12VideoDecoderHeap,
   ID3D12VideoDecodeCommandList, D3D12_VIDEO_DECODER_DESC, _HEAP_DESC, _DECODE_CONFIGURATION,
   _FRAME_ARGUMENT, _INPUT/_OUTPUT_STREAM_ARGUMENTS, _REFERENCE_FRAMES, _COMPRESSED_BITSTREAM,
   D3D12_FEATURE_DATA_VIDEO_DECODE_SUPPORT/_PROFILES; methods DecodeFrame, CreateVideoDecoder,
   CreateVideoDecoderHeap, CheckFeatureSupport all present (generic-method E0283 = exists).
   So NO FFI authoring for a D3D12/DXVA backend — bindings are complete & maintained by MS.

**‼️ DECISION (2026-07-23): target DXVA2 / `ID3D11VideoDecoder`, NOT D3D12 video.**
Both stateless (both fit cros-codecs); the decider is zero-copy. D3D12 video decode runs
on a D3D12 device → its NV12 output needs D3D12→D3D11 shared-NT-handle interop (+keyed
mutex) to reach ANGLE. `ID3D11VideoDecoder` (DXVA2) runs on **D3D11 — the SAME device our
Windows host's ANGLE renderer uses** (`canvas_impl.rs` `DisplayApiPreference::Egl` →
ANGLE-D3D11), so its output reaches GL via `EGL_ANGLE_d3d_texture_client_buffer` with NO
cross-API bridge. DXVA is also the ancestor of VA-API's picture-param structs, so the
`backend/vaapi` template maps even more directly, and it's the mature workhorse (ffmpeg
`d3d11va`, Chromium D3D11VideoDecoder) with abundant reference code. D3D11 video decode
bindings ALSO confirmed green on windows 0.62.2 (feature `Win32_Graphics_Direct3D11`:
ID3D11VideoDevice/Context/Decoder, ID3D11VideoDecoderOutputView, D3D11_VIDEO_DECODER_DESC/
_CONFIG/_BUFFER_DESC/_OUTPUT_VIEW_DESC + picture-params/bitstream/slice-control buffers).
D3D12 video stays a fallback (confirmed available; only worth it if a GPU exposes a codec
via D3D12 video but not DXVA2 — rare). MF MFT is the stateful last resort (simplest
same-device D3D11 zero-copy, but reuses none of cros-codecs and needs Store extension
packs for HEVC/AV1/VP9).

**Effort (revised): ~5.5–11 focused days.** Phase 0 spike (0.5–1d: decode one H.264 frame
via ID3D12VideoDevice, readback, verify pixels — burns down the DXVA picture-param fill +
video-command-list/barrier dance); Phase 1 (2–4d: implement StatelessH264DecoderBackend
against D3D12/DXVA, `backend/vaapi` is the line-by-line template — VA and DXVA picture-param
structs are both DXVA-derived; CPU-readback output first = working HW decoder, no compositor
change); Phase 2 (3–6d: generalize GpuFrame to a platform GPU handle + D3D→ANGLE zero-copy,
resolve the D3D12-vs-D3D11 fork here). ‼️ Can't test on the Linux dev box — needs the user's
Windows GPU for pixels.

## ✅ Phase 0 spike DONE — bit-exact HW decode on Windows (2026-07-23)
`repros/d3d11-video-decode-spike/` (uncommitted). Decodes cros-codecs' `64x64-I.h264`
via DXVA2/`ID3D11VideoDecoder` on harry's Windows GPU and the output NV12 is
**BIT-EXACT** vs the ffmpeg reference (CRC32 `7dd66ef1` == `64x64-I.h264.crc`, the
`ffmpeg -pix_fmt nv12 -f framehash -hash crc32` oracle). Decoded frame = the expected
videotestsrc colour bars. This validates, on real hardware, EVERYTHING the spike
targeted:
- cros-codecs' pure-Rust H.264 parser builds+runs on native Windows (`default=[]`,
  path `cros_codecs::codec::h264::parser::{Parser,Nalu,NaluType,Sps,Pps,SliceHeader}`;
  `Sps` comes as `&Rc<Sps>`, `Pps`/`Sps` are NOT `Clone` → copy needed fields out).
- DXVA structs are NOT in the windows crate (win32metadata excludes dxva.h) → hand-define
  `DXVA_PicParams_H264`/`_Slice_H264_Short`/`_Qmatrix_H264` (repr(C), zeroed()-init is
  valid — all ints). Filled by translating cros-codecs' vaapi `build_pic_param`
  (`src/decoder/stateless/h264/vaapi.rs`) — VA↔DXVA fields map 1:1. Traps that mattered:
  `Reserved16Bits=3`, `StatusReportFeedbackNumber` nonzero, RefFrameList all `0xFF`,
  bitstream buffer zero-padded to a multiple of 128, `ContinuationFlag=1`.
- The full `ID3D11VideoContext` dance works first try: `DecoderBeginFrame` →
  `GetDecoderBuffer`/copy/`ReleaseDecoderBuffer` ×4 (PicParams, IQMatrix, SliceControl-
  SHORT, Bitstream) → `SubmitDecoderBuffers` → `DecoderEndFrame`. Config short-slice =
  `ConfigBitstreamRaw==2`. Output NV12 texture (`BIND_DECODER`, ArraySlice 0) →
  `CopyResource` to STAGING → Map; NV12 UV plane at `pData + RowPitch*Height` (confirmed
  by the bit-exact CRC). H264_VLD_NoFGT GUID `1b81be68-...`; GPU exposed 35 profiles.

**Phase 1 confidence is now HIGH.** The hard, uncertain parts (DXVA param fill + the
D3D11 video-context sequence) are proven bit-exact.

## Phase 1 progress (2026-07-23) — REFERENCE decode proven, multi-slice pending
Grew the spike into a thin Windows H.264 driver (parser + POC + sliding-window DPB +
surface pool). ‼️ ARCHITECTURE FINDING: cros-codecs' DECODER DRIVER is unusable on
Windows — `pub mod decoder` is `#[cfg(feature="backend")]`, and `backend` pulls
gbm/drm/nix (Linux); worse, its `VideoFrame` handle trait bakes in `fill_v4l2_plane`/
`process_dqbuf`/`to_native_handle(Display)` (V4L2/DRM). So "one identical driver across
Linux+Windows" would need refactoring cros-codecs' core handle trait (heavy upstream
change), NOT a feature flag. BUT the correctness-critical bits — the PARSER and the full
DPB reference algorithms (`codec::h264::dpb`: sliding_window_marking, mmco_op_*,
store_picture, bump_as_needed, update_pic_nums) — are in the ungated `codec` module
(Windows-OK). So the real architecture = shared codec parser+DPB algorithms + a THIN
per-platform driver (~150 lines: POC calc + orchestration) over the D3D11 backend.
- POC computed by hand (type 0 = 8.2.1.1, type 2 = 8.2.1.3; test clips are type 2 for
  no-B, type 0 for B). Type 1 unsupported (rare).
- ✅ BIT-EXACT with references: `64x64-I-P` (I+P) and `64x64-I-P-B-P` (3 pics) — the
  DXVA short-slice RefFrameList mechanism (driver builds ref lists from RefFrameList +
  FieldOrderCntList/POC + FrameNumList + UsedForReferenceFlags) WORKS. Surface pool =
  one NV12 Texture2D array (BIND_DECODER), one ID3D11VideoDecoderOutputView per
  ArraySlice; CurrPic/RefFrameList index = ArraySlice; readback via CopySubresourceRegion.
- ✅ BIT-EXACT: `test-25fps.h264` (320x240 Main/profile-77, **2 slices/picture**, B-frames,
  **4 IDR-delimited GOPs**, 250 frames) — ALL 250 frames match the ffmpeg reference
  (`ffmpeg -pix_fmt nv12 -f framehash -hash crc32`) in display order. So the thin Windows
  H.264 driver is COMPLETE for Main profile: multi-slice pictures, P+B references, POC
  (types 0 & 2), multi-GOP, sliding-window DPB, and display-order output all correct.

‼️ ROOT CAUSE of the multi-slice bug (cost hours; REMEMBER): **dxva.h uses
`#pragma pack(1)` — every DXVA struct is byte-packed.** `DXVA_Slice_H264_Short` is
**10 bytes, not 12**. A Rust `#[repr(C)]` version is 12 bytes (u32-aligned, 2 tail pad),
so a 2-entry SliceControl array has the WRONG stride — the driver reads garbage for
entry 1 and silently decodes only slice 0 (top half correct, bottom undefined). Fix:
`#[repr(C, packed)]` on DXVA_Slice_H264_Short (and any DXVA struct with trailing padding).
`DXVA_PicParams_H264` happened to work with repr(C) ONLY because its fields are laid out
with no internal padding either way — which is why SINGLE-slice was bit-exact and masked
the bug. This is why matching ffmpeg's *bytes* wasn't enough: ffmpeg's sizeof is 10
(packed header), mine was 12. Diagnosis path that worked: dump my NV12 + ffmpeg's ref
NV12, diff Y/UV per-plane (top bit-exact, bottom desync) → reverse slice order (each slice
correct when FIRST → data good, submission wrong) → hexdump the exact SliceControl bytes →
realize 12-vs-10 stride. Other findings along the way: driver reads only entry-0's
byte-span with the 12-byte stride; 1-entry whole-span makes it scan+CABAC-desync;
per-slice submits don't accumulate — ALL symptoms of the stride bug, not separate issues.
Display order: POC resets at each IDR, so sort by (GOP, POC) not global POC (global gave
3/250; per-GOP gave 250/250). Real backend: reuse `codec::h264::dpb::bump` for proper
output ordering + MMCO (hand-rolled POC type-0/2 + sliding-window DPB suffices for these
test clips but not MMCO/long-term refs).
Spike: `repros/d3d11-video-decode-spike/` (uncommitted, `EXE i|ip|ipbp|25` all PASS).
ffmpeg-on-WSL available for reference NV12 diffing.

## ✅ REAL backend/d3d11 SHIPPED + tested (2026-07-23, uncommitted)
`crates/wandr-video/src/backends/d3d11.rs` — a real `CodecBackend`+`Decoder` impl (Windows
peer of vaapi.rs), wired into `default_registry()` (HW-first, probe-and-decline, SW
fallback) and `backends/mod.rs`, gated `#[cfg(all(feature="d3d11", target_os="windows"))]`.
Cargo.toml: new `d3d11` feature + a `cfg(target_os="windows")` dep table (windows 0.62,
cros-codecs default-features=off = parser only, anyhow). Integration test
`tests/d3d11_hw.rs` feeds test-25fps as access units through the public trait and asserts
ALL 250 FRAMES BIT-EXACT in display order — PASSES on the Intel UHD 620. Linux build
unaffected (feature Windows-gated). Key impl notes: `Decoder` requires `Send` but
cros-codecs' `Parser` holds `Rc<Sps>/Rc<Pps>` (not Send) → build a FRESH parser per
`decode()`, primed from stored raw SPS/PPS NAL bytes (Send); one chunk = one access unit
(guest demuxes); streaming display-order via per-GOP POC bumping (drain on IDR + reorder
window = `sps.max_num_order_frames()`); output = CPU-readback NV12→I420 (`Frame::cpu`), so
`frames_in_flight_limit()=None` like SW backends. Build/test from WSL: cargo over the
UNC manifest path + native CARGO_TARGET_DIR (wandr-video is standalone, own Cargo.lock).

## ✅ Phase 2a DONE — GpuFrame carries a D3D11 texture + backend Frame::gpu path (2026-07-23)
Self-contained foundational half of Phase 2 (no host changes needed, verified standalone):
- `GpuFrame` generalized WITHOUT touching the Linux dma-buf lane: added a cfg-gated
  (`all(feature="d3d11", target_os="windows")`) `GpuFrameOwner::d3d11() -> Option<D3d11View>`
  (default None) + `D3d11View { texture: ID3D11Texture2D, device: ID3D11Device, array_slice }`
  + `GpuFrame::d3d11()` accessor in lib.rs. So lib.rs/vaapi/host video_gl.rs are UNCHANGED on
  Linux (kept `planes/fourcc/modifier`); the D3D11 texture rides via the owner. (`ID3D11Texture2D`
  is Send+Sync in the windows crate — confirmed — so the owner stays Send.)
- Backend GPU-output path: `WANDR_VIDEO_D3D11_GPU=1` makes the decoder emit `Frame::gpu`
  instead of `Frame::cpu`. Each frame is a GPU-side copy (`CopySubresourceRegion`,
  BIND_SHADER_RESOURCE) OUT of the decode pool into its OWN NV12 texture — decouples frame
  lifetime from the DPB pool (no pool-refcount needed), stays on the GPU (no CPU roundtrip).
  `D3d11Owner` hands the host a `D3d11View` for import, or reads back to I420 on demand.
- VERIFIED: `tests/d3d11_hw.rs` passes 250/250 bit-exact on BOTH paths (CPU default, and
  `WANDR_VIDEO_D3D11_GPU=1` where the test's `read_i420` reads back the per-frame texture).
  Linux build unaffected. Clean (no warnings).
Note: per-frame texture = a GPU copy, not literal zero-copy (sampling the pool slice direct
would need pool-refcounting + the DPB); the win is "GPU-resident, no CPU readback", the
standard video-output pattern. True zero-copy is a Phase-2b refinement if measured worth it.

## Phase 2b = host ANGLE import of the D3D11 texture (NEEDS THE HOST ON WINDOWS + VISUAL CHECK)
Fundamentally different from everything above (which was codec-only + standalone-testable).
Requires: (1) generalize `GpuFrame` in wandr-video/lib.rs — today dma-buf-specific
(`planes: Vec<DmabufPlane>`, DRM fourcc/modifier) — to also carry a D3D11 texture / DXGI
shared handle (touches the vaapi producer + host video_gl.rs consumer); (2) ‼️ DEVICE
SHARING crux: the backend's decode D3D11 device must be the SAME device ANGLE uses
(`canvas_impl.rs` `DisplayApiPreference::Egl` → ANGLE-D3D11) — extract it via
`eglQueryDisplayAttribEXT(EGL_D3D11_DEVICE_ANGLE)` and hand it to the backend, else
cross-device shared-handle+keyed-mutex; (3) import NV12 texture via
`EGL_ANGLE_d3d_texture_client_buffer` → 2 GL textures (Y R8 + UV RG88) → Skia YUV, the
ANGLE analog of the Linux dma-buf→EGLImage path in `src/video_gl.rs`; (4) backend then
outputs `Frame::gpu` and `frames_in_flight_limit()` = pool budget. ‼️ VERIFICATION needs
the FULL host on Windows (window + ANGLE + Skia + video compositing) + a USER VISUAL CHECK
— not a standalone codec test. So Phase 2 is a real rendering-pipeline effort, not a codec
one; the CPU-readback backend already WORKS on Windows today.

Phase 1 = wrap this as a real `backend/d3d11` (thin driver + full codec::dpb reuse for
MMCO/bumping). H265/VP9/AV1 = same pattern with each codec's DXVA pic-param struct.
Effort estimate holds at the low end (Phase 1 ~2–3d, Phase 2 zero-copy 3–6d).

**Windows dev-from-WSL workflow (works):** `powershell.exe` interop from WSL bash reaches the
Windows toolchain (cargo/rustc at C:\Users\harry\.cargo\bin, msvc target, vcvars64.bat present).
Run cargo inside `cmd /c 'cd /d %TEMP%\proj && cargo ...'` (a NATIVE Windows cwd — UNC cwd from
the WSL path fails "UNC paths are not supported"). Feed multi-line PowerShell via
`cat script.ps1 | powershell.exe -NoProfile -Command -`. Avoid `$ErrorActionPreference='Stop'`
(cargo's normal stderr status lines become fatal). See [[reference_wsl_vaapi_d3d12_hw_decode]].
