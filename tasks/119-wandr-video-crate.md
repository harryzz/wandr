# Task 119 — `wandr-video`: consolidate video behind one crate, drop FFmpeg

> Status: 🔲 PROPOSAL — 2026-07-20. Supersedes the "replace FFmpeg" half of task 117
> (117 keeps the HW-backend research). Depends on nothing; unblocks task 118 (shipping
> binaries) by removing the LGPL/soname problem at the root.

## The finding that shapes everything

**Pure Rust is not an option for the codecs wandr actually needs.** Researched 2026-07-20
on crates.io:

| Codec | Pure-Rust encode | Pure-Rust decode |
|---|---|---|
| **VP8** | ✗ none | ✗ none (`oxideav-vp8` self-describes as "scaffold pending clean-room") |
| **VP9** | ✗ none | ✗ none (`oxideav-vp9` likewise a scaffold; `vp9-parser` parses only) |
| **H.264** | ✗ only `less-avc` (minimal/lossless subset — not real-time call material) | ⚠ `rust_h264` 0.4, 11K dl, first published 2026-04 — too young to trust |
| **H.265** | ✗ none | ⚠ `rust_h265` 0.1, 10K dl, 2026-04 — likewise |
| **AV1** | ✓ **rav1e** (BSD-2, 35M downloads, mature) | ⚠ `rav1d` (BSD-2, 20K dl, young); C `dav1d` is solid |

VP8 is exactly what a Signal/WebRTC call needs and exactly what the Pixel HW-encodes — and
it has no pure-Rust implementation at all. So "rewrite in pure Rust" is off the table.

**But pure Rust was never the actual requirement.** The two real problems with FFmpeg are
(a) LGPL/GPL licensing and (b) a runtime `.so`/dylib dependency. Both are solved by
**permissively-licensed C libraries linked statically** — no copyleft, no runtime
dependency, no "install exactly this version":

| Library | Licence | Static? | Covers |
|---|---|---|---|
| **libvpx** | BSD-3 | yes | VP8/VP9 encode + decode |
| **dav1d** (`libdav1d-sys` builds+statically links it) | BSD-2 | yes | AV1 decode |
| **rav1e** | BSD-2 | pure Rust | AV1 encode |
| **openh264** (Cisco) | BSD-2 | yes | H.264 encode + decode |

That is the whole FFmpeg surface wandr uses, at BSD, statically linkable.

## Codec matrix — what we actually need

Grounded in `contracts/wit/video.wit` (which already declares `enum codec { vp8, vp9,
h264, h265 }`) and its device notes.

### Live video call (WebRTC / Signal) — the shipping use case

| Codec | Encode (outgoing camera) | Decode (incoming peer) | Why |
|---|---|---|---|
| **VP8** | ✅ **required** | ✅ **required** | WebRTC mandatory-to-implement; what Signal negotiates; the SoC HW-encodes it |
| **VP9** | ❌ skip | ✅ required | WIT notes VP9 HW encode is **software-only on this SoC** — outgoing must prefer VP8. Peers may still send VP9 |
| H.264 | ⚪ optional | ⚪ optional | also WebRTC mandatory-to-implement (RFC 7742); interop insurance, not needed for Signal |
| AV1 | ⚪ future | ⚪ future | emerging in WebRTC; `rav1e`+`dav1d` make it the cheapest to add later |
| H.265 | ❌ no | ❌ no | not used in WebRTC; patent-encumbered |

**Minimum viable for calls: VP8 encode + VP8/VP9 decode.** That is one library — libvpx.

### Streaming media playback (a video player app)

**Decode only — no encoder at all.** Preference order is pure Rust → permissive C
(static) → LGPL → HW-only.

| Codec | HW decode | Software fallback | Licence | Maturity |
|---|---|---|---|---|
| **H.264** | ✅ every platform | `openh264` (Cisco) — verify the Rust binding exposes decode, it is encode-focused; else `rust_h264` | BSD-2 / MIT+Apache | openh264 mature (546K dl); rust_h264 v0.4, first published **2026-04** |
| **H.265** | ✅ every modern GPU/SoC | ⚠️ **GAP** — `rust_h265` (Main/Main10) or `libde265` (**LGPL**, which is what we are trying to escape) | MIT+Apache / LGPL | rust_h265 v0.1, 10K dl, **2026-04** — too young to rely on |
| **VP9** | ✅ | **libvpx** | BSD-3 | mature |
| **AV1** | ⚪ newer GPUs only | **dav1d** (C, static) or `rav1d` (pure Rust) | BSD-2 | dav1d mature; rav1d 20K dl, young |

**The H.265 software gap is real** and there is no mature permissive option — every
alternative is either months old or LGPL. Mitigations, in order:
1. **Lean on HW.** Every GPU/SoC since ~2015 decodes HEVC; for *playback* HW is the right
   path anyway (power, 4K). A software HEVC fallback is only needed on machines without HW
   support, which are rare.
2. Ship without software HEVC and report `no-hw-codec` (the WIT error already exists).
3. Revisit `rust_h265` once it has a track record — pure Rust would be ideal here.

### Patents — a separate axis from the code licence

Worth deciding before distributing binaries, because a permissive *code* licence does not
grant *patent* rights:

| Codec | Patent status |
|---|---|
| **VP8 / VP9 / AV1** | royalty-free by design (AOMedia / Google) — no exposure |
| **H.264** | MPEG-LA pool. Cisco's OpenH264 royalty coverage applies **only to Cisco's prebuilt binary**, NOT to source you compile yourself — the reason Firefox downloads that binary at runtime |
| **H.265** | most encumbered: multiple pools (MPEG-LA, HEVC Advance, Velos) |

This is a strong argument for **HW decode** on H.264/H.265: the codec then lives in the
user's OS/driver, already licensed by the hardware vendor, rather than in our binary.

Note the use case does not exist in wandr today — no app needs playback. Do not build it
until one does.

### Screen recording / casting (hypothetical)

Encode H.264 or VP8. Not currently a wandr feature — listed only so the matrix is honest
about what is speculative.

## Proposed crate: `wandr-video`

A backend-dispatch crate — NOT a codec implementation. The `wandr:video` WIT is already the
abstraction; this is the thing that implements it once, for every platform.

```
wandr-video/
  src/lib.rs        # Encoder/Decoder traits, codec + capability enums
  backends/
    mediacodec.rs   # Android  — HW (exists today in wandr-host)
    vaapi.rs        # Linux    — HW via cros-codecs (BSD-3)
    videotoolbox.rs # macOS    — HW
    mediafoundation.rs # Windows — HW
    libvpx.rs       # portable software VP8/VP9 (BSD-3, static)
    openh264.rs     # portable software H.264 (BSD-2, static) — optional
    av1.rs          # rav1e encode / dav1d decode (BSD-2) — optional
```

Selection at runtime: **try HW for the requested codec, fall back to the static software
backend.** That is what FFmpeg was doing for us, minus the licence and the `.so`.

### Why this is worth doing

- **Licence**: BSD everywhere → no LGPL obligations, no GPL risk from distro builds.
- **Distribution**: static → no `libavutil.so.58`, no Homebrew bottle floor, no Windows
  ffmpeg pinning. Task 118's macOS/Linux bundling problem largely evaporates.
- **Size**: replaces ~20 MB of dynamic FFmpeg with ~3-4 MB of static libvpx.
- **HW accel actually gets implemented** — desktop is software-only today.

### Sequencing

1. **libvpx software backend first** (VP8 encode + VP8/VP9 decode). This alone replaces
   every FFmpeg use in `video_desktop.rs` and is provably enough for Signal calls.
2. Delete the FFmpeg dependency; verify a desktop call end-to-end.
3. Add HW backends per platform (task 117's research), keeping libvpx as the fallback.
4. H.264/AV1 only when an app needs them.

### Explicitly NOT doing

- Containers/muxing (FFmpeg's `libavformat`). WebRTC carries raw frames over RTP; the
  guest already packetizes. Add `muxide`/`symphonia` only if file playback lands.
- Filters, scaling beyond YUV↔RGB (use `libyuv` or a Rust YUV crate).
- Audio — `symphonia` (MPL-2.0) covers that if ever needed; wandr uses `wasi:audio`/cpal.
