---
name: reference_vaapi_zerocopy_real_players
description: "How mpv/GStreamer/Chromium/VLC/Firefox actually do VA-API zero-copy display — pool-vs-per-frame, NV12 as two textures not external OES, i965 Gen7 tiling trap, no fences. Read before touching the wandr zero-copy path."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 25b6eb4c-9122-4870-8734-7e515af11a68
  modified: 2026-07-22T14:09:57.025Z
---

Read from the actual sources (2026-07-22) before implementing wandr's zero-copy
video path, after burning a lot of cycles reasoning about pools and fences from
first principles when five shipping implementations already encode the answers.
That was a rule-1 miss: **read the reference implementation first**
([[feedback_read_source_first]]). Sources: FFmpeg `hwcontext_vaapi.c` /
`vaapi_decode.c`, mpv `hwdec_vaapi.c` / `dmabuf_interop_gl.c`, GStreamer `sys/va`
+ `gst-libs/gst/gl/egl`, VLC `interop_vaapi.c`, Chromium `vaapi_video_decoder.cc`
/ `vaapi_wrapper.cc`, Firefox `FFmpegVideoFramePool.cpp`, intel-vaapi-driver
`i965_drv_video.c`, Mesa `crocus_resource.c`.

## 1. Pool + cache, or per-frame? DEPENDS ON WHO OWNS THE POOL

* **Per frame** (export + `eglCreateImageKHR` every frame): mpv, VLC (1-deep
  cache), Firefox, and the Rust `rusty-codecs`. They do this ONLY because they
  sit downstream of FFmpeg's `AVBufferPool`, which on libva ≥ 1.1 is **dynamic,
  lazily grown and unbounded** (`vaapi_decode.c` sets `initial_pool_size = 0`;
  all the careful DPB accounting is dead code). Surface identity is not stable,
  so there is nothing to key a cache on.
* **Once per surface** (export + EGLImage + texture cached for the pool's
  lifetime): GStreamer and Chromium — the two that OWN their pools. GStreamer
  extracted `GstEGLImageCache` (`gsteglimagecache.c`) into public API in Nov 2024
  (MR !6792) specifically so a second element could reuse it.

**wandr owns its pool** (`VaSurfaceFrame::to_native_handle` is our code), so the
cached form is the right one. Per-frame export is not "simpler", it is a
workaround for a constraint we do not have.

Pool sizing, from GStreamer `gstvah264dec.c:744`: `min_buffers = dpb_size + 4`
("dpb size + scratch surfaces"), plus whatever the compositor holds in flight.

## 2. NV12 AS TWO TEXTURES (R8 + GR88), NOT `GL_TEXTURE_EXTERNAL_OES`

Unanimous: mpv, VLC, Firefox, rusty-codecs, and GStreamer's indirect path all
split NV12 into `DRM_FORMAT_R8` + `DRM_FORMAT_GR88`, one `GL_TEXTURE_2D` each,
and apply the YUV→RGB matrix in their OWN shader.

**Why, and this is the load-bearing bit:** the external/direct path can only pass
colour as a HINT — `EGL_YUV_COLOR_SPACE_HINT_EXT` /
`EGL_SAMPLE_RANGE_HINT_EXT`. The EGL spec lets drivers ignore them, and drivers
historically default to BT.601 limited regardless of content. For BT.709 or
full-range material that is a silent, visible error, and you also lose control of
chroma siting, primaries and transfer.

So the "will the GPU and CPU lanes differ in colour?" worry is not something to
MEASURE, it is something to DESIGN AWAY: own the matrix, take it from the
stream's VUI.

Counter-argument that is also real: GStreamer's *indirect* (per-plane) path
requires LINEAR and refuses tiled buffers, so the direct/external path is its
only tiled option.

## 3. ‼️ i965 / IVYBRIDGE GEN7: DECODE SURFACES ARE Y-TILED

From `i965_drv_video.c::i965_ExportSurfaceHandle` and
`i965_surface_native_memory`:

* `DRM_PRIME_2` is the **only** accepted export memory type; there is no legacy
  `DRM_PRIME` export.
* Always **one object** — planes are offsets into a single BO. So every EGL plane
  attrib references the same fd with different `OFFSET`/`PITCH`.
* NV12 decode surfaces are allocated **TILED** when `HAS_TILED_SURFACE` (true on
  Gen7) → expect **`I915_FORMAT_MOD_Y_TILED`, not linear**. Building the EGL
  attrib list without modifiers, or hardcoding LINEAR, renders GARBAGE SILENTLY.
  Mesa `crocus_resource.c::modifier_is_supported` accepts Y_TILED for ver ≥ 6 and
  explicitly REJECTS `DRM_FORMAT_MOD_INVALID`.
* Padding is aggressive (`ALIGN(w*bpp,128)`, `ALIGN(h,32)`) — use the returned
  pitches/offsets and crop to the real size.
* `i965_ExportSurfaceHandle` does **not** sync. Sync yourself.
* Format gaps are real: `drm_format_of_separate_plane()` covers NV12/I420/P010/
  I010 and returns 0 for the rest → export fails with `INVALID_SURFACE`
  (libva#626, no fix).
* Chromium disables `DRM_PRIME_2` **import** on i965 (`vaapi_wrapper.cc:2605`)
  but uses DRM_PRIME_2 **export** on all drivers. Export on i965 is well-trodden.

Modifier handling, universal pattern: query
`EGL_EXT_image_dma_buf_import_modifiers`; if present pass `MODIFIER_LO/HI`
(skipping `MOD_INVALID`); if ABSENT, **omit the attribs entirely** and let the
kernel's GEM tiling carry it — never substitute LINEAR.

## 4. NO FENCES ANYWHERE

Zero occurrences of `EGL_KHR_fence_sync` / `EGLSync` in mpv, GStreamer's `va`,
VLC's interop or ffmpeg's hwcontext. It is `vaSyncSurface` or implicit kernel
dmabuf fences, full stop. GStreamer's GPU path does not sync at all — it relies
on the i915 BO's `dma_resv`.

**What actually stops the decoder overwriting a live surface is REFCOUNTING ON
THE FRAME, not the sync.** The sync only proves decode finished. mpv holds an
`mp_image` ref, GStreamer a parent-buffer meta, Firefox an `av_buffer_ref`.

`va_sync_surface` should retry `VA_STATUS_ERROR_HW_BUSY` (GStreamer: 10x with
1 ms sleeps); that failure mode is real.

## 5. FIREFOX'S BACK-PRESSURE VALVE — the fixed-pool deadlock answer

`FFmpegVideoFramePool.cpp::ShouldCopySurface()`: when free pool ratio drops below
`SURFACE_COPY_THRESHOLD` (1/4), **GPU-copy** the frame into a private dmabuf so
the VA surface returns to the decoder immediately. Zero-copy degrades to
one-GPU-copy under compositor pressure instead of stalling the decoder. A fixed
pool WITHOUT this deadlocks the first time the compositor holds one frame too
long — the exact failure a dynamic pool hides.

## 6. `vaExportSurfaceHandle` IS the modern path

FFmpeg tries `esh` first and falls back to `vaDeriveImage`+`vaAcquireBufferHandle`
only pre-libva-1.1; that legacy path cannot report a format modifier at all
(`format_modifier = DRM_FORMAT_MOD_INVALID` — "no way to get it with this API").
mpv/GStreamer/Chromium/Firefox do not implement it. Treat it as dead.

## 7. Rust prior art

Almost none, and the negatives matter: cros-codecs' own `ccdec` is **CPU readback
only and never renders** (`gbm_video_frame.rs::import_from_vaapi` has ZERO
callers); gst-plugins-rs has no Rust VA element; `ffmpeg-next` **cannot do
VAAPI** (no `hw_frames_ctx` wrappers); wgpu's dmabuf import landed 2026-07 and is
single-plane only.

**The one good reference: `n0-computer/iroh-live`, crate `rusty-codecs`**
(MIT/Apache-2.0) — `src/codec/vaapi/decoder.rs` (sync-before-export, memoised in
a `OnceCell`, `catch_unwind` around a cros-codecs `clone` that panics on EMFILE)
and `src/render/gles_dmabuf.rs` (`import_nv12` / `create_plane_image`: R8 + GR88,
full EGL attrib construction). Take its attribs and workarounds, NOT its
per-frame caching strategy.

## 8. wandr specifics

* `cros-libva 0.0.12`'s `Surface::export_prime()` (`src/surface.rs:350`)
  **hardcodes `READ_ONLY | COMPOSED_LAYERS`** — you get ONE layer of NV12 with
  two planes, not two R8/GR88 layers. To get mpv's shape, call
  `vaExportSurfaceHandle` directly with `SEPARATE_LAYERS`, or work with the
  composed descriptor (on i965 it is one BO either way, so the offsets/pitches
  are all there).
* Our vendored `cros-codecs/src/backend/vaapi.rs:204` already implements
  `ExternalBufferDescriptor` with `MEMORY_TYPE = DrmPrime2`, i.e. the IMPORT
  direction (allocate dmabufs ourselves, hand them to VA) is available — that is
  Chromium's model and removes export entirely. But Chromium disables DRM_PRIME_2
  import on i965, so on Gen7 that needs the legacy
  `VASurfaceAttribExternalBuffers` shape.
* UNVERIFIED: how Skia wants to consume two planes. A manual matrix likely needs
  an `SkRuntimeEffect`, where `GL_TEXTURE_EXTERNAL_OES` would be a plain
  `GrBackendTexture`. Real ergonomics cost to weigh against colour correctness.
* WSL caveat: if VA there is d3d12 + `MESA_LOADER_DRIVER_OVERRIDE=vgem`
  ([[reference_wsl_vaapi_d3d12_hw_decode]]) none of the i965/crocus specifics
  apply. Check `vaQueryVendorString()` before trusting the tiling story.

See [[reference_libvpx_wandr_video]] for the crate this plugs into and
[[project_wandr_video_host.md]] for the host lane.
