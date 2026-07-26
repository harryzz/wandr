---
name: reference_gstreamer_desktop_backend_spike
description: "GStreamer is the SOLE desktop video-DECODE backend (2026-07-26) — the per-OS handwritten decoders (d3d11/vaapi/vt + libde265/dav1d/openh264/oxideav) are RETIRED+deleted; libvpx kept for VP8/VP9 encode only. Zero-copy on all 3 OSes (Linux dma-buf / Windows D3D11-ANGLE / macOS IOSurface). Working recipe + traps + the retirement/decouple. Read before touching desktop video decode or revisiting \"one library for OS video\"."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 215f1733-fbc2-4004-aac8-cacd9719553d
  modified: 2026-07-26T06:08:39.442Z
---

Motivated by the libde265-Windows pain ([[reference_libde265_windows_win32cond_crash]]).
Question: is there ONE library that handles OS-specific HW/SW video decode + render-to-
surface + subtitles + transport + HW/SW-force, desktop + Android, so wandr stops hand-
rolling per-OS backends? Evaluated LiveKit (WebRTC transport over C++ libwebrtc — wrong
layer), OxiMedia (pure-Rust FFmpeg/OpenCV reimpl — NO H.264/H.265 by patent policy,
decoders intra-only, no OS HW — unusable), and **GStreamer** (the real answer).

## Verdict: GStreamer `playbin3`/`decodebin3` is the component — DESKTOP only.
- It auto-plugs native HW decoders per OS (`d3d11h26xdec`, `vah26xdec`, `vtdec`) + libav
  SW, renders to a GL surface, does subtitles + play/stop/seek/rate, and HW/SW is
  forced by decoder **rank** (or playbin autoplug-select). One ~350-line program, zero
  per-OS code, covers Win+mac+Linux.
- **Android is OUT**: GStreamer HW decode = the `amc` plugin = `android.media.MediaCodec`
  via **JNI/Java** → cannot run on wandr's `--no-art` stack. KEEP the NDK `AMediaCodec`
  backend on Android (already shipped, --no-art-safe). GStreamer covers the 3 desktops;
  Android stays as-is. Both behind the existing WIT video contract.
- Cost: bundles the GStreamer **C runtime** on desktop (the thing task 117 left). It's a
  strategic swap to "exit codec-build maintenance wholesale", not a quick win — but it
  DOES delete the libde265/dav1d/openh264/libvpx + d3d11/vaapi/vt maintenance.

## Spike (repros/gstreamer-spike/, WSL + Pop!_OS `popos` w/ Intel VAAPI) — ALL PROVEN
Crates: `gstreamer` + `gstreamer-app` (decode) + `gstreamer-gl`/`-gl-egl` (zero-copy).
No "playbin3 crate" — it's `ElementFactory::make`; `gstreamer-play` is the player-object crate.
1. **Decode + HW/SW-force + correct pixels**: `filesrc ! decodebin3 ! videoconvert !
   appsink`. `hw` mode demotes SW decoder ranks to NONE / `sw` demotes HW. On popos:
   H.264 HW `vah264dec` + H.264/H.265 SW `avdec_*`, all pixel-perfect (the bbb tree frame).
   The bbb-h265 that libde265 corrupted on Windows decodes flawlessly via `avdec_h265`.
2. **HW is fast & effectively HW-age-independent**: pure `vah264dec ! fakesink` = ~1500 fps
   on a 2012 Ivybridge iGPU. A naive `! videoconvert ! RGBA ! appsink` collapses to ~240 fps
   because it forces a GPU→CPU readback + CPU convert — that tax is the ONLY reason HW ever
   looks slow. Stay GPU-resident.
3. **Zero-copy GL + context-sharing (the crux)**: `... ! glupload ! glcolorconvert !
   appsink(caps=GLMemory)` keeps frames as GL textures (300/300 GLMemory). We create our
   OWN surfaceless EGL/GLES2 context (Skia/ANGLE stand-in), share it into GStreamer, and
   SAMPLE GStreamer's HW-decoded texture in OUR context → correct frame, no CPU copy.

## ‼️ Context-sharing recipe (the official gstreamer-rs glupload-example pattern)
Three traps, each cost an iteration; all needed:
1. **Separate shared context, NOT the wrapped one.** `let shared = GLContext::new(&display);
   shared.create(Some(&wrapped))?;` and hand GStreamer `shared` for `gst.gl.app_context`.
   Giving it your wrapped context makes GStreamer make YOUR context current on its gl
   thread → steals it from your main thread → all your GL calls no-op → black frame.
2. **Cross-thread GL fence.** Buffer often has NO GLSyncMeta → add one: get the buffer's
   producing ctx from `buf.peek_memory(0).downcast_memory_ref::<GLBaseMemory>().context()`,
   `GLSyncMeta::add(bufm, &pctx)` if absent, `set_sync_point(&pctx)`, then in YOUR context
   `meta.wait(&wrapped)` before sampling. Skip it → texture reads empty.
3. **Force `texture-target=2D` in the appsink caps.** VAAPI/glupload gives a
   `TEXTURE_EXTERNAL_OES` (EGLImage); a plain `sampler2D` reads it BLACK. `glcolorconvert`
   to `...,format=RGBA,texture-target=2D` yields a normal 2D texture. (samplerExternalOES
   is the alternative.) Diagnostic that pinned it: clear-RED then readback = red (our ctx
   renders fine) but the textured draw = black → it's the sample, not the FBO/sync.
Also: `glIsTexture(id)` in your context == true is the share-group proof; headless GL on
popos over SSH needs `GST_GL_PLATFORM=egl` + `EGL_PLATFORM_SURFACELESS_MESA`.

These are the SAME class of problems (device/context sharing, sync, texture target) the
Windows d3d11→ANGLE backend already solved ([[reference_media_codec_strategy]] Phase 2b),
so Win/mac reuse the recipe with `d3d11h265dec`/`vtdec` swapped for `vah264dec`.

## Backend WIRED (CPU-first) into wandr-video, behind `gstreamer` feature.
`crates/wandr-video/src/backends/gstreamer.rs` — a real `CodecBackend`+`Decoder`.
- Registered as TWO lanes sharing one impl: `GStreamerBackend{hardware:true}` (kind
  Hardware, prio 15) + `{hardware:false}` (Software, prio 95). HW vs SW chosen by
  INSTANTIATING the specific element (`vah264dec` vs `avdec_h264` — `pick_decoder`
  probes availability), NOT global rank mutation, so both coexist. `#[cfg(all(
  feature="gstreamer", not(target_os="android")))]` (desktop only).
- Pipeline: `appsrc(codec caps) ! h26xparse ! <decoder> ! videoconvert !
  video/x-raw,format=I420 ! appsink`. `decode(chunk)`=push buffer w/ pts;
  `next_frame`=try_pull → tight I420 → `Frame::cpu`; `flush`=EOS + BLOCK-drain
  `pull_sample` loop into a queue (non-blocking poll misses the tail).
- ‼️ GOTCHA (cost an hour): libav `avdec_*` default FRAME threading holds ~Ncores
  frames and does NOT flush that tail on appsrc-EOS → silently drops the last ~10
  frames. Fix: append `thread-type=slice` to avdec (no cross-frame delay, still
  parallel, correctness > peak fps). HW decoders untouched (no libav threading).
- Test `tests/gstreamer_decode.rs` (feature-gated): feeds the committed test-25fps.h264
  AUs through `open_decoder_with`. `gstreamer-sw` (avdec) → 250/250 @ 320x240 on WSL.
  `gstreamer-hw` (`require_hardware:true`) → **250/250 via VAAPI `vah264dec` on popos**
  (skips gracefully where no HW decoder). BOTH lanes verified end-to-end through the trait.
  To build wandr-video off-tree (e.g. popos): it's standalone (own Cargo.lock) but needs
  the path/patch dirs present — `crates/wandr-vpx-sys` (optional dep, manifest parsed even
  when off) + `vendor/cros-codecs` + `vendor/libde265-sys` (the two [patch.crates-io]).
  WSL glibc (2.41, Debian 13) > popos (2.39) so you can't copy the binary — build on target.
- Feature OFF = plain `cargo build` unaffected (opt-in, links system GStreamer only when on).
- GPU zero-copy output IMPLEMENTED (dma-buf lane), compiles, integrated: `WANDR_VIDEO_GST_GPU=1`
  + HW lane on Linux → pipeline `appsrc ! parse ! vah26xdec ! appsink(caps="video/x-raw(memory:DMABuf),format=DMA_DRM")`;
  `build_gpu_frame` extracts fd (gstreamer-allocators `DmaBufMemory::fd`, dup via
  `BorrowedFd::try_clone_to_owned`) + planes (`VideoMeta` offset/stride) + fourcc/modifier
  (parse the caps `drm-format="NV12:0x…"` string manually — `dma_drm_fourcc_from_str` needs
  the `v1_24` feature) → `GpuFrame` with a `GstDmabufOwner`. This is the SAME dma-buf currency
  vaapi.rs emits, so the host's existing dma-buf→EGLImage compositor consumes it — NO host changes.
  ✅ RUNTIME-VERIFIED ON i965 (2026-07-25): the whole clip plays zero-copy at **4% CPU** (vs ~30%
  for I420 readback, below handwritten vaapi's 8%), presented 300/300. The earlier "i965 too old to
  export dma-buf / hardware limit" claim WAS FALSE — a ONE-LINE NEGOTIATION BUG: `vah264dec` exports
  dma-buf on i965 fine (gst-launch to fakesink negotiates DMA_DRM drm-format=NV12:0x0100000000000002,
  the SAME Y-tiled modifier vaapi.rs's export_prime gives). The failure was `gst_va_base_dec_decide_
  allocation: DMABuf caps negotiated without the mandatory support of VideoMeta` — the va decoder
  won't export unless the SINK advertises GstVideoMeta in the ALLOCATION query. FIX: an appsink
  sink-pad QUERY_DOWNSTREAM probe doing `alloc.add_allocation_meta::<gst_video::VideoMeta>(None)` +
  return `PadProbeReturn::Handled` (GPU lane only). LESSON: `not-negotiated` is almost never a HW
  limit — read the ERROR line above it before inventing a hardware story.
  GPU lane is now DEFAULT-ON for Linux HW (was opt-in), gated on data-free `decoder_exports_dmabuf`
  = does the decoder SRC pad template list `video/x-raw(memory:DMABuf)`. GPU that can't export stays
  on I420 (no untested fallback). `WANDR_VIDEO_GST_GPU=0` forces readback. Mirrors vaapi default-ZC.
  (Test `gstreamer_hw_gpu_dmabuf` still SKIPS on 0 frames as an honest guard elsewhere; on i965 it
  now produces frames. The GL-texture path in share.rs also works on i965 but is NOT needed.)
- Also `reset()` (seek) is best-effort flush-events; real seek may need pipeline restart.

## ✅ PLAYS IN THE REAL PLAYER ON LINUX (popos, on-screen, smooth) — both SW + HW.
Ran the actual `wandr.video.player` on Pop!_OS through the GStreamer backend, VISUALLY
verified by the user: `gstreamer-sw` (avdec_h265) H.265 = smooth 289/300 presented;
`gstreamer-hw` (VAAPI vah264dec) H.264 = smooth 281/286; clock tracks the clip, ~no drops.
Force with `WANDR_VIDEO_BACKEND=gstreamer-hw|gstreamer-sw`. (popos i965 has H.264 HW but
NOT H.265 HW, so H.265 must use SW there.)

‼️ THREE fixes were needed — the backend decoding correctly was NOT enough:
1. **appsink `new-sample` CALLBACK** (not `try_pull_sample` polling). GStreamer decodes
   ASYNC on its own thread; a poll right after `decode()` returns None and the playback
   lane deadlocks. The callback (`gst_app::AppSinkCallbacks::builder().new_sample(|s|{
   let sample=s.pull_sample()?; queue.push_back(sample); Ok(FlowSuccess::Ok)})`) pushes
   frames into an `Arc<Mutex<VecDeque<gst::Sample>>>` as they're produced; `next_frame`
   just pops. This is THE documented pattern (gstreamer-rs examples/src/bin/appsink.rs).
   `flush` waits on an EOS-callback `AtomicBool` so the reorder tail is fully queued.
2. **Host playback lane must DRAIN on consume, not just submit** (`src/video_desktop.rs`).
   `submit_for_playback` called `queue_decoded()` (→ Decoder::next_frame) only on submit —
   a SYNCHRONOUS-decoder assumption. Added `self.queue_decoded()` at the top of BOTH
   `present_due` and `take_next_decoded` so the host pulls the ASYNC decoder's frames when
   the guest presents/takes, not only when it feeds. No-op for sync decoders (vaapi/libde265).
3. **The player `ui.wasm` must have the first-frame CLOCK ANCHOR** (apps/user/
   wandr.video.player, ~line 664 "anchor the playback clock to the FIRST frame's actual
   emergence"). GStreamer's startup latency (VA init + async spin-up) lets the playback
   clock run ahead; without anchoring at first-frame every frame reads "too late" → 0
   presented. popos had a STALE ui.wasm predating this fix — pushing the current build was
   the missing piece. LESSON: when integrating a new backend, verify the guest wasm is current.
Symptom that fingerprints all three: `fed N, decoded-out N, presented 0/6, held 0` +
an erratic clock jumping to tens of seconds. Native vaapi (sync + fast) masked all three.

## Backend selection: FAMILY matching + PROBE gate (2026-07-25)
- `WANDR_VIDEO_BACKEND=gstreamer` (NO `-hw`/`-sw` suffix) now means the gstreamer
  FAMILY: the registry `candidates()` filter (`crates/wandr-video/src/lib.rs`) matches
  a backend name either EXACTLY or by family — `name == want || name.strip_prefix(want)
  .is_some_and(|r| r.starts_with('-'))`. So a family pin + the accel preference resolves:
  in-app `h` (PreferHardware) → `gstreamer-hw`, `s` (PreferSoftware, no_hardware) →
  `gstreamer-sw`, `n`/no-pref → gstreamer-hw by priority. BONUS: H.265+`h` on a
  no-HEVC-HW GPU (i965) now gracefully falls to gstreamer-sw instead of `no-hw-codec`
  (PreferHardware allows the sw fallback; a hard `-hw` exact pin could not). Verified on
  popos: `gstreamer_family_{sw,hw}_resolves` tests both decode 250/250 (hw did NOT skip).
- Enablement is a PROBE, mirroring vaapi: host feature `gstreamer = ["wandr-video/
  gstreamer"]`; `scripts/build-host-linux.sh` enables `--features gstreamer` iff
  `pkg-config --exists gstreamer-1.0 gstreamer-app-1.0` (GST=0/1 override). A box without
  GStreamer still builds a software host. NOT unconditional in Cargo.toml (that regressed
  build portability). popos probe: "ENABLED (gstreamer 1.24.2 found)".
- GATING SEAM (user intends to eventually DROP the hand-written backends for GStreamer):
  it's pure Cargo features. GStreamer-only = build with `--features gstreamer`, clear the
  `["libvpx","openh264","libde265","dav1d"]` list on the wandr-video dep (host Cargo.toml
  line ~157), and `VAAPI=0`. No code change. User said "i don't need now" — left enabled.

## Windows: GStreamer decode WORKING (2026-07-25) — same backend, SW+HW
- Build box: WSL can escape to Windows via `powershell.exe`/`cmd.exe`; VS2022 vcvars64 at
  `C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat`;
  source tree `C:\Users\harry\wandr-host-build` (SEPARATE working copy, diverged from Linux —
  its own uncommitted d3d11/hevc/libde265 work; apply gstreamer changes SURGICALLY, never clobber).
- GStreamer is NOT preinstalled — install it (the gstreamer WAY = we provide the media layer).
  Installed **1.24.13 MSVC** (matches gstreamer-rs 0.23 baseline; newer 1.28+ ship .exe not .msi).
  NO-ADMIN install: `msiexec /a <msi> /qn TARGETDIR=C:\Users\harry\gstreamer` (administrative
  EXTRACT, no elevation) for BOTH runtime + devel MSIs → `C:\Users\harry\gstreamer\gstreamer\1.0\msvc_x86_64`.
  ‼️ The freedesktop server truncates large downloads (~200-300MB) — the 724MB devel MSI needs
  CHUNKED download: `curl.exe -r <start>-<end>` in 20MB pieces + concat (script in gst-dl/fetch-chunked.ps1).
  Build env: `GSTREAMER_1_0_ROOT_MSVC_X86_64=<root>\`, `PKG_CONFIG_PATH=<root>\lib\pkgconfig`,
  `PKG_CONFIG=...UniGetUI\Chocolatey\bin\pkg-config.exe`, `PATH+=<root>\bin`. See build-gstreamer.bat.
- Build: `cargo build --release --features p3-async,d3d11,gstreamer`. TWO fixes were needed:
  (1) host Cargo.toml Windows wandr-video line listed `dav1d/libvpx/openh264` (contradicting its own
  comment) → they need meson/nasm/vcpkg → DROP them (GStreamer covers H.264/VP8/VP9/AV1 now); keep
  `libde265`+`oxideav-h265` (cc/pure-Rust SW H.265). (2) `gstreamer-allocators` uses `std::os::unix`
  → won't compile on Windows → move it to `[target.'cfg(...linux...)'.dependencies]`, keep
  `dep:gstreamer-allocators` in the `gstreamer` feature (target-gated dep edge = Linux-only, winit pattern).
- ✅ `gstreamer_decode` test 5/5 on Windows: SW (avdec_h264) 250/250, HW (**d3d11h264dec, DXVA on
  Intel UHD 620**) 250/250, family hw/sw both 250/250.
- ✅ **D3D11 ZERO-COPY GPU lane DONE + VERIFIED** (2026-07-25, player on UHD 620): H.265 via
  d3d11h265dec, presented **300/300**, CPU **10.5% zero-copy vs 43.8% readback** (`GST_GPU=0`) = ~4×.
  HOW (all in backends/gstreamer.rs `mod d3d11_gpu`, `cfg(windows, feature=d3d11)`): (1) `win_gpu_available`
  = host set ANGLE's device via `set_angle_d3d11_device` (video_gl.rs pulls it from `eglQueryDeviceAttrib
  EXT(EGL_D3D11_DEVICE_ANGLE)`). (2) `inject_angle_device`: `gst_d3d11_device_new_wrapped(angle_dev)` →
  `gst_d3d11_context_new` → `pipeline.set_context` BEFORE PLAYING, so d3d11h26xdec decodes on ANGLE's
  device. (3) tail = `appsink caps="video/x-raw(memory:D3D11Memory),format=NV12"`. (4) `build_gpu_frame_d3d11`:
  `gst_d3d11_memory_get_resource_handle`+`_subresource_index` → `CopySubresourceRegion` (device-LOCKED,
  GStreamer decodes on its streaming thread) into a FRESH single `SHADER_RESOURCE|RENDER_TARGET` NV12
  texture (array_slice 0) → `D3d11View`. This MIRRORS d3d11.rs::export_texture EXACTLY, so the host's
  `import_d3d11` (EGL_D3D11_TEXTURE_ANGLE) consumes it with ZERO host changes — array-slice never leaves
  the backend. Raw FFI vs `gstd3d11-1.0` (`#[link]`, search path from gstreamer-sys); D3D11 = `windows` crate.
  ‼️ Build with `--features d3d11,gstreamer` (gstreamer d3d11 lane reuses d3d11.rs's `angle_d3d11_device`
  + `readback_nv12_texture`, both pub(crate)). ANGLE (libEGL/libGLESv2) ships next to the host exe.

## macOS: IOSurface zero-copy DONE (2026-07-26) — H.265 via `vtdec`, 300/300, 10.7% ZC vs 26.5% readback.
Same backend, `mod iosurface_gpu`: appsink `caps="video/x-raw,format=NV12"` (VideoToolbox keeps
CVPixelBuffers), read the `GstCoreVideoMeta` (`g_type_from_name("GstCoreVideoMetaAPI")` +
`gst_buffer_get_meta`, repr(C) `{GstMetaHdr, cvbuf, pixbuf}`) → `CVPixelBuffer` → the host's
EXISTING `import_iosurface` (`CGLTexImageIOSurface2D` → `GL_TEXTURE_RECTANGLE`), NO host change.
IOSurface is a shareable GPU resource → race-free. Reuses the videotoolbox.rs import path exactly.

## ✅ CONSOLIDATION COMPLETE + hand-written decoders RETIRED (2026-07-26, wandr-host `a63e3ae`).
The "user intends to eventually DROP the hand-written backends" gating seam was EXERCISED:
`vaapi`/`d3d11`/`hevc`/`hevc_dxva`/`videotoolbox`/`libde265`/`dav1d`/`openh264`/`oxideav_h265`
backends + their tests DELETED; GStreamer is the SOLE desktop decode path. `libvpx` KEPT (VP8/VP9
ENCODE for Signal — GStreamer doesn't encode). The per-OS GPU zero-copy glue those decoders carried
(ANGLE `ID3D11Device` handoff + D3D11 NV12 readback; CVPixelBuffer readback) was DECOUPLED into
`backends/gpu_interop.rs`, gated on the `gstreamer` feature (not the retired `d3d11`/`videotoolbox`
features); the `IOSurfaceView`/`D3d11View` host handles + `import_iosurface`/`import_d3d11` re-gated
the same way (macOS handle needs no crate → `target_os="macos"`; Windows `D3d11View` needs the
`windows` crate, now pulled by the `gstreamer` feature on Windows). ONE feature set
`--features p3-async,gstreamer` builds the full decode stack on all 3 desktop OSes — verified
building Linux + Windows (libvpx via vcpkg `VPX_LIB_DIR`) + macOS; all CI legs green. Cargo.toml
dropped the retired features/deps (cros-codecs, libva, openh264, oxideav, libde265, dav1d,
core-foundation, anyhow) + their `[patch]`. RETIRING the decoders also fixed the Windows "Could not
allocate vertices" flood: it was the PRIORITY MISHMASH (handwritten d3d11 + libde265 registered
ABOVE gstreamer), NOT a device race — with no handwritten decoder registered, `gstreamer-hw` always
wins. Build guide: `runtime/wandr-host/docs/building-desktop.md`. Supersedes the "keep hand-rolled
backends" decision in [[reference_media_codec_strategy]]. Task-117 tail: Jellyfin/YouTube
real-client proof + upstream `wandr:video` proposal (see task doc).
