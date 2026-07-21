---
name: reference_wsl_vaapi_d3d12_hw_decode
description: "VA-API HW video decode WORKS in WSL2 on old Intel iGPUs via Mesa d3d12 — exact env vars, and why cros-codecs' GBM path still fails there"
metadata: 
  node_type: memory
  type: reference
  originSessionId: ed607dbd-a38a-49fd-b8ec-bfd92f150821
  modified: 2026-07-21T12:04:22.618Z
---

**VA-API hardware decode works in WSL2 on the dev box** (Intel UHD 620, 8th-gen —
*below* Microsoft's documented 11th-gen floor). Verified 2026-07-21:

```
vainfo: Driver version: Mesa Gallium driver 25.0.7-2 for D3D12 (Intel(R) UHD Graphics 620)
  VAProfileH264ConstrainedBaseline/Main/High : VAEntrypointVLD
  VAProfileHEVCMain / HEVCMain10             : VAEntrypointVLD
  VAProfileVP9Profile0 / Profile2            : VAEntrypointVLD
```
(no AV1 — that needs Gen11+/Arc.)

## The exact working setup (3 undocumented pieces)

```bash
sudo modprobe vgem                    # creates /dev/dri/card0 — vgem IS the intended
                                      # DRM placeholder for WSL; dxgkrnl does NOT make one
LIBVA_DRIVER_NAME=d3d12 \
MESA_LOADER_DRIVER_OVERRIDE=vgem \    # NOT =d3d12; without it: "MESA-LOADER: failed to
                                      # retrieve device information"
GALLIUM_DRIVER=d3d12 \                # ← REQUIRED and undocumented; without it:
                                      # "d3d12_drv_video.so init failed", vaInitialize error 2
vainfo --display drm --device /dev/dri/card0
```

Microsoft's blog documents only `LIBVA_DRIVER_NAME=d3d12` — that alone FAILS.
`GALLIUM_DRIVER=d3d12` is the same var that fixes GL falling back to llvmpipe
(`glxinfo` → `D3D12 (Intel UHD 620)`), see microsoft/wslg#1394.
**An official "supported hardware" table is an auto-enablement floor, NOT a
"physically works" list** — see [[feedback_check_means_verify]].

## ‼️ cros-codecs still cannot decode here — GBM is the blocker

Two independent failures, same component (cros-codecs 0.0.6 output allocation):

1. `VaapiBackend::new()` creates a dummy **16×16** H.264 context; the D3D12 driver
   rejects it with `VA_STATUS_ERROR_RESOLUTION_NOT_SUPPORTED` (19). Patching it to
   256×256 fixes this step (verified) — it is `pub(crate)`, so it needs a fork.
2. Then GBM allocation fails: *"Error allocating contiguous buffer! NV12 1280x720"*.
   **Unfixable on WSL** — the DRM node is vgem, a dummy device; real GPU memory is
   behind `/dev/dxg`. On fedora (Ivybridge/i965) the same call fails for an
   unrelated reason (i915 rejects `GBM_BO_USE_HW_VIDEO_DECODER` contiguous NV12).

cros-codecs implements `VideoFrame` ONLY for GBM/DMA frames, so there is no
non-GBM output path inside the crate.

**Conclusion:** to use VA-API HW decode in wandr, go **direct on cros-libva** —
`vaCreateSurfaces` (no GBM) + `vaDeriveImage`/`vaGetImage` readback — using
cros-codecs only for its platform-agnostic bitstream *parsers*. This also matches
the surface ownership the eventual zero-copy `present(at-ns)` path needs, and is
the same code path a Windows/VAOn12 build would use. See task 117 M2 Phase A.
