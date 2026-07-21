# vaapi-decode-probe — VA-API HW decode (task 117 M2, Phase A)

Proves VA-API **hardware** H.264 decode before wiring a `VaapiBackend` into
`wandr-video`. Uses **cros-libva-direct surfaces** (`vaCreateSurfaces`, no GBM),
keeping cros-codecs only for its H.264 parser + DPB/reference management.

## Run

```bash
sudo modprobe vgem                 # creates /dev/dri/card0 (WSL)
LIBVA_DRIVER_NAME=d3d12 MESA_LOADER_DRIVER_OVERRIDE=vgem GALLIUM_DRIVER=d3d12 \
WANDR_DRM_DEVICE=/dev/dri/card0 \
  ./target/release/vaapi-decode-probe bbb.h264 100
```
Input is a raw H.264 **Annex-B** elementary stream:
`ffmpeg -i in.mp4 -c:v copy -bsf:v h264_mp4toannexb out.h264`.
See `[[reference_wsl_vaapi_d3d12_hw_decode]]` for the WSL env-var story.

## ✅ What is PROVEN

**VA-API HW decode is available on the WSL box** (Intel UHD 620 via Mesa d3d12 —
*below* Microsoft's documented 11th-gen floor, but it works):
`H264 Constrained/Main/High`, `HEVC Main/Main10`, `VP9 Profile0/2` — all VLD.

Capability probe output (this machine):
```
driver resolution limits: min=[64]x[64] max=[8192]x[4352]
driver memory types: ["VA", "DRM_PRIME", "DRM_PRIME_2"] (raw 0x68000001)
  tier 1 (zero-copy export_prime -> DMA-buf): unavailable (VaError(6) INVALID_SURFACE)
  tier 2 (derive_from / vaDeriveImage):       AVAILABLE
  tier 3 (create_from / vaGetImage copy):      USED
```
Two lessons already banked:
* **min resolution is 64x64** — upstream cros-codecs hardcodes a **16x16**
  placeholder context, which D3D12 rejects with
  `VA_STATUS_ERROR_RESOLUTION_NOT_SUPPORTED` (19). Our vendored patch queries the
  driver instead (`VASurfaceAttribMinWidth/Height`). Confirmed working:
  `wandr: VA placeholder context 64x64 (driver-reported minimum)`.
* The driver **advertises `DRM_PRIME_2` but `export_prime()` fails** — proof that
  probing beats trusting advertised capability bits.

The H.264 bitstream also parses correctly end to end:
`format: NV12 coded 1280x720 display 1280x720`, `Decode picture POC 0`.

## ❌ Where it stops (open)

After `Finishing picture POC 0` the process **hangs** (`vaEndPicture` /
submission). Isolated: it still hangs with `sync()` **and** readback skipped
(`WANDR_SKIP_SYNC=1` in the isolation build), so it is **not** the readback path
— it is the decode submission against the D3D12 driver.

Leading hypothesis: we allocate a **fresh VA surface per frame** with no pooling
(`VaSurfaceFrame::to_native_handle` calls `vaCreateSurfaces` every time). D3D12
video decoders generally want output surfaces from a **fixed pool bound to the
decode heap**; unbounded fresh allocations mid-stream plausibly wedge it. Next
step is a real surface pool (which the eventual backend needs anyway). Other
angles: compare against a known-good `ccdec` run on this driver; check whether
D3D12 wants surfaces created with specific attributes.

## Vendored cros-codecs patches (`vendor/cros-codecs`, via `[patch.crates-io]`)

1. **Driver-queried placeholder resolution** (was hardcoded 16x16) — fixes
   `RESOLUTION_NOT_SUPPORTED` on D3D12. One named fallback constant
   (`PLACEHOLDER_FALLBACK_DIM`) only when the driver reports no minimum.
2. **`VaapiDecodedHandle::surface()` made `pub`** — upstream is `pub(crate)`, so
   there is otherwise no way to read back from a VA-allocated surface.

## Why not cros-codecs' own frame types

Its only `VideoFrame` impls are GBM/DMA-backed, and GBM allocation fails on BOTH
available machines for unrelated reasons:
* fedora (Ivybridge/i965): i915 rejects `GBM_BO_USE_HW_VIDEO_DECODER` contiguous NV12.
* WSL (UHD 620/d3d12): the DRM node is **vgem**, a dummy device; real GPU memory
  is behind `/dev/dxg`.

VA-API itself works on both — only cros-codecs' GBM output path does not.
