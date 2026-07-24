---
name: reference_dxva_h264_windows_decode
description: "Windows DXVA2/D3D11 H.264 HW decode correctness traps (CABAC start code, cros DPB, SPS-sized pool) — verified pixel-exact"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 25b6eb4c-9122-4870-8734-7e515af11a68
  modified: 2026-07-23T20:03:32.868Z
---

Native-Windows H.264 HW decode via DXVA2/`ID3D11VideoDecoder`, in
`runtime/wandr-host/crates/wandr-video/src/backends/d3d11.rs` (feature `d3d11`,
`target_os="windows"`). Decodes on ANGLE's D3D11 device so the NV12 output is a
same-device alias the host imports zero-copy via `EGL_ANGLE_image_d3d11_texture`
(host `src/video_gl.rs`). Proven **pixel-exact** on Big Buck Bunny 1280x720
High/CABAC/8x8/hierarchical-B (IDR + deep-stream frame); committed 13a7ae5.

**Traps that only a High-profile CABAC B-pyramid clip exposes** (a Main/CAVLC clip
like `repros/samples/test-25fps.mp4` decodes bit-exact while hiding ALL of these —
so always test decode on PIXELS with a real-world clip, e.g.
`repros/oxideav-spike/samples/bbb-h264.mp4`):

1. **Bitstream needs a FIXED 3-byte start code `{0,0,1}` per slice**, with
   `DXVA_Slice_H264_Short.SliceBytesInBuffer = 3 + nal_len` and
   `BSNALunitDataLocation` pointing at it — matching ffmpeg `dxva2_h264.c`
   `commit_bitstream_and_slice_buffer`. Store the raw NAL WITHOUT its start code
   (cros `nalu.as_ref()`, not `nalu.data` which includes a variable 3/4-byte
   code). A 4-byte code misaligns the CABAC engine → decodes a couple MB rows
   then diverges into a smooth colour-gradient "garbage" (CAVLC is tolerant, so
   the bug is invisible on CAVLC clips).

2. **Reference management MUST use cros-codecs' `Dpb` + `PictureData`** (POC types
   0/1/2, `sliding_window_marking` / `mmco_op_1..6`, `update_pic_nums`,
   `store_picture`, `bump_as_needed`). A hand-rolled sliding window ignores MMCO
   (`memory_management_control_operation`, which real encoders use heavily) →
   keeps retired references → driver picks stale refs → garbage. cros's DPB
   couples reference-retention with output via `needed_for_output`, and only
   `bump_as_needed`/`sliding_window_marking` call `remove_unused` — so you must
   drive OUTPUT through `bump_as_needed` too, not a separate reorder buffer.
   Decouple the decoded pixels from the decode pool slot with a per-picture id +
   `pending: HashMap<id,Decoded>` and `slot_of: HashMap<id,slot>`; reconcile the
   pool free-list against `dpb.entries()` each frame. `Dpb<u64>` holds `Rc` →
   needs `unsafe impl Send for D3d11Decoder` (single-threaded use, like the vaapi
   backend). See [[reference_media_codec_strategy]].

3. **Size the decode-surface pool from the SPS** (`num_ref_frames` capped at the
   16-entry `RefFrameList` + current + slack), never a fixed count — a 16-ref clip
   "pool exhausted" on an 8-slot pool at the 9th reference. See [[feedback_no_hardcoding]].

4. `DXVA_PicParams_H264.wBitFields` bit 14 = **MinLumaBipredSize8x8Flag =
   `(level_idc >= 31)`**, NOT `direct_8x8_inference` (that has its own scalar
   field). B-frame-only, but wrong. `Reserved16Bits = 3` works on Intel UHD 620
   (didn't need ffmpeg's `0x34c` ClearVideo value). `dxva.h` structs are
   `#pragma pack(1)` — `DXVA_Slice_H264_Short` must be `#[repr(C, packed)]`.

Windows-from-WSL build/run: `tools/scripts/run-host-windows.ps1` (`-Window` runs
the guest `wandr.video.player` on screen; default is a headless
`--video-decode-file` PNG). Beware PowerShell `-Stop` + `2>&1 | cmdlet` turning
cargo's stderr warning into a terminating error — redirect native output to a file
under `EAP=Continue` instead.
