# Task 117 — `wandr-video`: consolidate video, drop the FFmpeg dependency

> Status: 🔲 PROPOSAL — researched 2026-07-19/20. Merges the former task 119 (which is
> retired; this file is canonical). Unblocks task 118 by removing the LGPL + soname
> problem at its root rather than packaging around it.

## Why

The desktop `wandr:video` backend (`runtime/wandr-host/src/video_desktop.rs`, 561 lines)
uses FFmpeg for VP8/VP9 encode+decode and YUV↔RGB scaling. Two problems:

1. **Licensing.** FFmpeg is LGPL-2.1-or-later, but nearly every distro builds it
   `--enable-gpl` (verified locally), which makes *that build* GPL. wandr is Apache-2.0.
2. **Distribution.** Linking system FFmpeg binds the binary to one soname — the
   `libavutil.so.58` failure when running a CI artifact locally; on macOS Homebrew bottles
   pin a minimum OS; on Windows a *release* ffmpeg is required (BtbN `master-latest` fails
   to compile: `AVCodec::pix_fmts` removed post-8.0).

Two things it is NOT about:
- **HW acceleration does not come from FFmpeg** — the OS/GPU driver provides it; FFmpeg is
  a portable wrapper. Going native keeps HW *and* drops the licence. That is already the
  model on Android (MediaCodec, task 93).
- Desktop is **software-only today**: it selects `"libvpx"`/`"libvpx-vp9"` with zero HW
  plumbing (no `hwaccel`, `hw_device_ctx`, VAAPI, VideoToolbox, D3D11VA). Desktop HW
  encode/decode is unimplemented, not merely un-accelerated.

## Finding 1 — pure Rust cannot cover what we need

Researched crates.io 2026-07-20. Preference order is **pure Rust → permissive C (static)
→ LGPL → HW-only**; pure Rust is preferred, not required.

| Codec | Pure-Rust encode | Pure-Rust decode |
|---|---|---|
| **VP8** | ✗ none | ✗ none (`oxideav-vp8` self-describes as "scaffold pending clean-room") |
| **VP9** | ✗ none | ✗ none (`vp9-parser` parses only) |
| **H.264** | ✗ only `less-avc` (minimal/lossless subset — not real-time material) | ⚠ `rust_h264` v0.4, 11K dl, first published **2026-04** |
| **H.265** | ✗ none | ⚠ `rust_h265` v0.1, 10K dl, **2026-04** |
| **AV1** | ✓ **rav1e** (BSD-2, 35M dl, mature) | ⚠ `rav1d` (BSD-2, 20K dl, young); C `dav1d` is solid |

VP8 — the codec Signal negotiates and the SoC HW-encodes — has **no** pure-Rust
implementation. So "rewrite in pure Rust" is off the table.

Also evaluated and rejected as the answer: **`video-rs`** (MIT/Apache, 301K dl, mature) is
a high-level API *over* `ffmpeg-next` (pins `=8.0.0`), so it inherits every licensing and
distribution problem unchanged; its hwaccel is **decode-only** (`Cuda, D3D11Va, Drm,
Dxva2, MediaCodec, OpenCL, Qsv, Vdpau, VideoToolbox` — notably **no VAAPI**) and
`encode.rs` has no hwaccel at all, which is exactly the gap for calls. Worth revisiting if
HW-accelerated *playback* is ever wanted. `avio` (582 dl) also wraps FFmpeg; `oxideav`
(27 dl) claims every backend but is an experiment.

## Finding 2 — pure Rust was never the requirement

The real problems are the *licence* and the *runtime `.so`*. Both are solved by
**permissively-licensed C libraries linked statically**:

| Library | Licence | Covers |
|---|---|---|
| **libvpx** | BSD-3 | VP8/VP9 encode + decode |
| **dav1d** (`libdav1d-sys` builds + statically links) | BSD-2 | AV1 decode |
| **rav1e** | BSD-2 | AV1 encode (pure Rust) |
| **openh264** (Cisco) | BSD-2 | H.264 |

BSD + static = no copyleft, no runtime dependency, ~3-4 MB instead of ~20 MB of dynamic
FFmpeg. That is the entire FFmpeg surface wandr uses today.

## Codec matrix

Grounded in `contracts/wit/video.wit`, which already declares
`enum codec { vp8, vp9, h264, h265 }` and records the device reality:
encoder = OUTGOING (our camera, host HW-encodes, guest RTP-packetizes);
decoder = INCOMING (guest pushes, host HW-decodes **to surface**, zero copy).

### Live video call (WebRTC / Signal) — the shipping use case

| Codec | Encode (outgoing) | Decode (incoming) | Why |
|---|---|---|---|
| **VP8** | ✅ **required** | ✅ **required** | WebRTC mandatory-to-implement; what Signal negotiates; the SoC HW-encodes it |
| **VP9** | ❌ skip | ✅ required | the WIT records that VP9 HW encode is **software-only on this SoC** — outgoing must prefer VP8. Peers may still send VP9 |
| H.264 | ⚪ optional | ⚪ optional | also mandatory-to-implement (RFC 7742); interop insurance, not needed for Signal |
| AV1 | ⚪ future | ⚪ future | emerging in WebRTC; `rav1e`+`dav1d` make it cheap to add |
| H.265 | ❌ | ❌ | not used in WebRTC; patent-encumbered |

**Minimum viable for calls: VP8 encode + VP8/VP9 decode — one library, libvpx.**

### Streaming media playback (no app needs this yet)

**Decode only — no encoder at all.**

| Codec | HW decode | Software fallback | Licence | Maturity |
|---|---|---|---|---|
| **H.264** | ✅ every platform | `openh264` (verify the Rust binding exposes decode — it is encode-focused); else `rust_h264` | BSD-2 / MIT+Apache | openh264 mature (546K dl); rust_h264 3 months old |
| **H.265** | ✅ every modern GPU/SoC | ⚠️ **GAP** — `rust_h265` (v0.1) or `libde265` (**LGPL**, the thing we are escaping) | — | neither is dependable |
| **VP9** | ✅ | **libvpx** | BSD-3 | mature |
| **AV1** | ⚪ newer GPUs | **dav1d** or `rav1d` | BSD-2 | dav1d mature |

**The H.265 software gap is real.** Mitigations in order: (1) lean on HW — every GPU/SoC
since ~2015 decodes HEVC and HW is the right path for playback anyway; (2) ship without
software HEVC and return the existing `no-hw-codec` WIT error; (3) revisit `rust_h265`
once it has a track record.

### Patents — an axis separate from the code licence

A permissive *code* licence does not grant *patent* rights:

| Codec | Patent status |
|---|---|
| **VP8 / VP9 / AV1** | royalty-free by design (Google / AOMedia) — no exposure |
| **H.264** | MPEG-LA pool. Cisco's OpenH264 royalty coverage applies **only to Cisco's prebuilt binary**, NOT to source you compile — the reason Firefox downloads it at runtime |
| **H.265** | most encumbered: MPEG-LA, HEVC Advance, Velos |

Another argument for **HW decode** of H.264/H.265: the codec then lives in the user's
driver, already licensed by their hardware vendor, instead of inside our binary.

## What else FFmpeg gives (the "did we miss something" check)

Codecs are the small part of FFmpeg.

**A/V sync — FFmpeg does NOT do this.** `libavformat` hands over PTS/DTS and a time base;
the *application* (ffplay, mpv, VLC) owns the clock, drift correction and frame dropping.
Sync is player code either way — dropping FFmpeg loses nothing here.

| FFmpeg piece | Calls? | Playback? | Permissive replacement |
|---|---|---|---|
| Demuxers (MP4/MKV/WebM) | ❌ RTP; guest packetizes | ✅ | `symphonia` (MPL-2.0, 8.6M dl), `mp4` (MIT, 11.4M), `matroska`, `mp4parse` |
| Audio decode (AAC/MP3/FLAC/Vorbis) | ❌ Opus is in the guest's WebRTC stack | ✅ | `symphonia`; Opus via `audiopus` (ISC) |
| Resample (`libswresample`) | ❌ | ✅ | `rubato` (MIT/Apache, 8M dl) |
| Scale / YUV↔RGB (`libswscale`) | ✅ | ✅ | `libyuv` (BSD-3) or a Rust YUV crate |
| Subtitles | ❌ | ✅ | `subparse` (srt/ass); rendering via **libass — ISC** |
| HLS / DASH | ❌ | ⚪ | `hls_m3u8`, `dash-mpd` (MIT) |
| HTTP(S) | ❌ | ✅ | reqwest/hyper already in-tree |
| Seeking / probing / metadata | ❌ | ✅ | comes with the demuxers |
| **Bitstream filters** (`h264_mp4toannexb`) | ❌ | ⚠️ **easily forgotten** — required to feed a HW decoder from MP4 (length-prefixed → Annex-B). No crate; ~100 lines |
| Filters (crop/deinterlace/overlay) | ❌ | ❌ | — |
| RTSP / RTMP | ❌ | ⚪ IP-camera only | out of scope |

**Calls: nothing missing.** **Playback:** everything exists permissively, but it is
assembling ~6 crates instead of one, plus the bitstream filter — and FFmpeg's real moat,
decades of robustness against malformed files, is not reproducible.

## Proposed crate: `wandr-video`

A backend-**dispatch** crate, NOT a codec implementation. `wandr:video` (WIT) is already
the abstraction; this implements it once for every platform.

```
wandr-video/
  src/lib.rs           # Encoder/Decoder traits, codec + capability enums
  backends/
    mediacodec.rs      # Android — HW (exists today inside wandr-host)
    vaapi.rs           # Linux   — HW via cros-codecs (BSD-3, 1.47M dl, HW encode+decode)
    videotoolbox.rs    # macOS   — HW (videotoolbox crate, MIT/Apache)
    mediafoundation.rs # Windows — HW (windows crate, MIT/Apache)
    libvpx.rs          # portable software VP8/VP9 (BSD-3, static)
    openh264.rs        # portable software H.264 (BSD-2, static) — optional
    av1.rs             # rav1e encode / dav1d decode (BSD-2) — optional
```

Runtime selection: **try HW for the requested codec, fall back to the static software
backend** — what FFmpeg did for us, minus the licence and the `.so`.

There is no "cpal for video": no crate combines cross-platform coverage with maturity.
`cros-codecs` is the strongest evidence this is tractable (real HW encode+decode) but is
Linux-only. The unifying layer is ours to write — and we already own it as `wandr:video`.

## Sequencing

1. **libvpx software backend first** — VP8 encode + VP8/VP9 decode. This alone replaces
   every FFmpeg use in `video_desktop.rs` and is provably enough for Signal calls.
2. Delete the FFmpeg dependency; verify a desktop call end-to-end. → task 118 simplifies.
3. Add HW backends per platform, keeping libvpx as the fallback.
4. H.264 / AV1 only when an app needs them.
5. Feature-gate the whole video backend so a plain desktop `cargo build` needs no media
   library at all (`image` handles `decode-image` independently — verified).

## Explicitly NOT doing

- Containers/muxing, subtitles, HLS/DASH — see the table; build none of it until an app
  actually asks for playback.
- Filters, scaling beyond YUV↔RGB.
- Audio: `symphonia` (MPL-2.0) if ever needed; wandr uses `wasi:audio`/cpal.
- Removing FFmpeg wholesale before the native paths are proven on real hardware. Make it
  optional; keep it as the long-tail fallback.

## Starting points (for whoever picks this up)

Everything needed is in-tree; this task is self-contained.

**Read first** (memory, recalled by relevance):
- `[[project_desktop_video_nokhwa]]` — the current desktop path. Records that VP8 is
  all-pass, that the **WSLg RDP camera truncates above 640x480**, and the `--run-once`
  harness. That camera caveat will otherwise look like a codec bug.
- `[[project_wandr_video_host]]` — the Android HW path (camera→HW-VP8→SURFACE/PiP) and the
  Surface upcast gotcha. The MediaCodec backend to preserve.
- `[[project_wandr_call_video_track]]` — Signal specifics: RED PT-120, TWCC mandatory,
  rotation via the container matrix. Constrains what the encoder must emit.

**The code:**
- `runtime/wandr-host/src/video_desktop.rs` (561 lines) — the ONLY file using FFmpeg. The
  surface is ~12 APIs: `codec::{context::Context, Id}`, `decoder::{find, Video}`,
  `encoder::{find_by_name, video::Encoder}`, `format::Pixel`, `software::scaling`,
  `util::frame::video::Video`, `util::picture`, `init`.
- `runtime/wandr-host/src/video.rs`, `video_host_impl.rs` — WIT plumbing; should not need
  to change, the trait boundary stays.
- `contracts/wit/video.wit` — the abstraction. **Do not change it**; the point is that the
  backend swaps underneath.
- `runtime/wandr-host/Cargo.toml` lines ~141-151 — the `nokhwa` + `ffmpeg-next` block to
  replace. `image` stays (it serves `decode-image` independently — verified).

**Verify with:**
1. `wandr-host --probe-video` (`main.rs:205`) — camera → encode → decode, reports fps and
   first-frame latency. Fastest signal.
2. `repros/nokhwa-camera-probe` — the standalone camera→VP8→decode reproducer.
3. A real Signal desktop video call — the acceptance test.

**Done when:** `ffmpeg-next` is out of `Cargo.toml`, a Signal desktop call works, and a
plain `cargo build` needs no system media library.
