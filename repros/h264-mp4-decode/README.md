# h264-mp4-decode (task 117 M2 step 2b)

Decode a real H.264 MP4 through `wandr-video`'s openh264 backend and prove the
file-playback path end to end. Not part of the host build.

```
# needs a built vendor/libvpx (VPX_LIB_DIR etc.) — same env as the host
cargo run --release [path/to.mp4]     # defaults to ../oxideav-spike/samples/bbb-h264.mp4
```

## What it exercises

- **`h264_mp4toannexb`** — MP4 stores length-prefixed NALs (avcC); openh264 wants
  Annex-B start codes, with SPS/PPS prepended at each keyframe. ~40 lines, no crate.
- **Real-file decode** — real SPS/PPS, multiple GOPs, B-frames.
- **Reorder** — openh264 emits in *decode* order; a depth-4 buffer restores
  *presentation* order.

## What it found

1. **`Flush::NoFlush` is mandatory.** openh264 defaults to flushing after every
   decode; on this file that overflows the reorder buffer at the 2nd GOP (OOM,
   then cascading no-param-sets — 236/300 frames lost). With `NoFlush` +
   `flush_remaining` at EOS: **300/300, zero errors.** The fix is in the backend
   (`openh264.rs`); this repro is how it was found.
2. **openh264 outputs in decode order** — a B-frame file needs a reorder buffer
   for presentation. That is player policy (a call never reorders), so it belongs
   in the host adapter, not the codec.

bbb-h264.mp4 comes from `../oxideav-spike/samples/` (run that repro's
`fetch-samples.sh` first). The oxideav spike independently decoded the same file
300/300, confirming the content is good and the earlier loss was ours.
