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

## ✅ PASS on fedora (Ivybridge / i965) — 300/300 frames

```
driver resolution limits: min=[]x[] max=[4096]x[4096]
driver memory types: ["VA", "KERNEL_DRM", "DRM_PRIME"] (raw 0x30000001)
  tier 1 (export_prime -> DMA-buf): AVAILABLE (1 dma-buf object)
  tier 2 (derive_from):             AVAILABLE
  tier 3 (vaGetImage):              USED
format: NV12 coded 1280x720 display 1280x720
decoded 300 frames, 1280x720, non-black luma: true
PASS — VA-API HW H.264 decode works (tier 3 readback, VA-allocated surfaces)
```

**Phase A goal met:** VA-API HARDWARE H.264 decode, end to end, on real hardware,
with NO GBM anywhere — the exact blocker that stopped cros-codecs. **No surface
pool was needed.**

Two things the probe caught that a hardcoded implementation would not:
* fedora reports **no minimum resolution at all** (`min=[]`), so the vendored
  patch's single named fallback (`PLACEHOLDER_FALLBACK_DIM`) is what makes it
  work — while WSL/D3D12 reports 64x64. Neither machine matches upstream's
  hardcoded 16x16.
* **tier 1 zero-copy is AVAILABLE on fedora but FAILS on WSL** — the exact
  inverse of what the advertised memory-type bits suggest on WSL (which claims
  `DRM_PRIME_2` and then errors). Probe, never trust the bits.

## ❌ WSL / D3D12: driver decode is broken (not our code)

On WSL the probe hangs after `Finishing picture POC 0` (`vaEndPicture`).
**This is a driver bug, not our code** — proven by elimination:

| test | WSL / d3d12 | fedora / i965 |
|---|---|---|
| `vainfo` capabilities | ✅ H264+HEVC+VP9 | ✅ H264 |
| **ffmpeg** `-hwaccel vaapi` (30 frames) | ❌ **hangs >2 min** | ✅ 0.32s, exit 0 |
| ffmpeg software (control) | ✅ fast | ✅ 0.32s |
| this probe | ❌ hangs | ✅ **300/300 PASS** |

ffmpeg is a mature, battle-tested VA-API client; it hangs on the same driver, so
the Mesa **d3d12 VA-API decode submission is broken** for this UHD 620 / WSL
GPU-PV setup. It enumerates capabilities perfectly and then wedges on real
decode. A surface pool was hypothesised and is **NOT** the cause — the cheap
falsifying test (run ffmpeg) killed that theory before any code was written.

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
