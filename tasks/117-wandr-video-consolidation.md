# Task 117 — `wandr-video`: consolidate video, drop the FFmpeg dependency

> **M1 — drop FFmpeg: ✅ DONE 2026-07-20.** VP8/VP9 is statically-linked libvpx
> (BSD-3) via the new `wandr-video` + `wandr-vpx-sys` crates. All four CI legs green;
> a Signal desktop **video call works on Windows** (user-verified). Merges the former
> task 119 (retired; this file is canonical). Unblocks task 118 by removing the
> LGPL + soname problem at its root rather than packaging around it.
>
> **M2 — media playback: 🟢 CORE DONE / real-client proof + upstream remain.** The decoder is no longer RTP-shaped —
> opaque `decoded-frame` + `present(at-ns)` + `timestamp-us`/`flush` shipped, and
> `apps/user/wandr.video.player` plays a real MP4 on-screen with **HW decode +
> zero-copy** on all three desktop backends: **macOS** VideoToolbox (IOSurface→
> `GL_TEXTURE_RECTANGLE`), **Windows** d3d11/DXVA (ANGLE), **Linux** VAAPI — each
> **smooth and tear-free, user-verified 2026-07-24**. Two fixes landed it:
> (1) anchor the guest playback clock at the FIRST decoded frame, not decode-start,
> so HW reorder-window latency doesn't peg every `present(at-ns)` in the past and
> starve the host loop to ~5Hz (`e0cb6114`); (2) desktop GL surface to vsync
> (`SwapInterval::Wait`) to kill tearing during the present-rate ramp (`fc2df90`).
> See `[[reference_video_player_present_clock_anchor]]`. **Android MediaCodec
> player path — ✅ VERIFIED ON SCREEN, Pixel 2 XL / --no-art 2026-07-24.**
> `bbb-h264.mp4` plays on the panel: HW decode (`OMX.qcom.video.decoder.avc`)
> decode-to-surface, **real-time 30 fps** (measured 30.2/30.0/29.3 fps across the
> run, `at_ns` a stable +120–170 ms ahead), behind-ui hole-punch showing the video
> under the guest's captions, correct colour. Three fixes landed it:
> (1) **present-path wiring** — `present(at-ns)` →
> `AMediaCodec_releaseOutputBufferAtTime(codec, idx, at_ns)` (zero-copy; `at_ns`
> is already CLOCK_MONOTONIC so no conversion — the reason `host_clock` chose it);
> `submit-timed`/`next-decoded`/`flush`(EOS)/`reset`(`AMediaCodec_flush`) all in
> `src/video.rs` `mod android`. `decoded-frame` on Android is a HELD MediaCodec
> output-buffer index (not CPU pixels), guarded by an `Arc<AtomicBool>` liveness
> flag so a frame outliving its decoder no-ops instead of UAF-releasing a stale
> index; holding the index is itself the back-pressure. `video_host_impl.rs`
> needed zero changes (already generic), and the guest already emits Annex-B with
> in-band SPS/PPS, so **no host `mp4toannexb`/CSD wiring was needed**.
> (2) **pacing** (`wandr.video.player` `next_frame_delay` 200 ms→16 ms) — on
> Android present is HW-scheduled by SurfaceFlinger, so there is NO host-side
> scheduled queue and thus NO independent video wake source (unlike desktop's
> `time_until_next_scheduled`); the guest MUST request a frame-rate cadence or it
> pumps at ~5 Hz and runs at ~0.5×, every deadline in the past.
> (3) **behind-ui transparency** (`src/egl.rs` +`EGL_ALPHA_SIZE, 8`) — the
> standalone EGL config requested no alpha, so `eglChooseConfig` picked an RGBX
> config and Skia's transparent clear stored opaque → the hole-punch composited
> black over the video; adding alpha + the existing `sf_set_opaque(false)` reveals
> it (safe — the app SF layer is `eLayerOpaque` by default, so opaque-clearing
> apps are unaffected). Also: the player reconciles its video rect via `set-rect`
> in `on_resize` (no restart, fixing the placeholder-rect issue), and gained a
> device `[[mounts]]` alt (`~` resolves to `/wandr/…` under `HOME=/` on device).
> **All three desktop backends + Android now play a real MP4.**
>
> **Decode consolidation — ✅ DONE 2026-07-26.** The desktop decode side is now a
> SINGLE **GStreamer** backend (`crates/wandr-video/src/backends/gstreamer.rs`,
> `gstreamer-hw` prio 15 / `gstreamer-sw` prio 95): one library wrapping every OS's
> HW+SW decoders (VA / DXVA / VideoToolbox + `avdec_*`) via
> `appsrc→parser→decoder→appsink`, with per-OS zero-copy — Linux **dma-buf**,
> Windows **D3D11** texture (ANGLE device-inject), macOS **IOSurface**
> (`GstCoreVideoMeta`→`CVPixelBuffer`→existing `import_iosurface`, no host change).
> The hand-written per-OS decoders this proposal's codec matrix called for
> (`openh264`/`libde265`/`oxideav`/`dav1d` SW + `vaapi`/`d3d11`/`videotoolbox` HW)
> were **RETIRED and deleted** — the "H.265 software gap" and the `libde265` LGPL
> exposure are gone (GStreamer's `avdec_h265` covers SW HEVC; HW covers the rest).
> The GPU zero-copy glue moved into `backends/gpu_interop.rs`, gated on the
> `gstreamer` feature. `libvpx` stays — VP8/VP9 **encode** for Signal, which
> GStreamer does not do. One feature set — `--features p3-async,gstreamer` — builds
> the full decode stack on Linux/Windows/macOS; all CI legs green. Build guide:
> `runtime/wandr-host/docs/building-desktop.md`. See
> `[[reference_gstreamer_desktop_backend_spike]]`, `[[reference_libde265_windows_win32cond_crash]]`.
> ⚠️ The "Codec matrix" and "M2 — media playback (🔲 NEXT)" sections below are
> SUPERSEDED by this consolidation — kept for the sequencing history only.
>
> **Remaining:** the Jellyfin/YouTube real-client proof + upstream proposal —
> split out to **`tasks/119-media-player-real-client-proof.md`** (Jellyfin DirectPlay
> first, then YouTube via the Invidious REST API; both feed the upstream `wandr:video`
> proposal). 117's engine work is DONE; 119 is the "prove with real consumers" tail.
>
> **What shipped in M1** (see "Outcome (M1)" for the deltas from this proposal):
> `runtime/wandr-host/crates/wandr-video` (desktop-only codec dispatch) +
> `crates/wandr-vpx-sys` (own Apache-2.0 bindings; builds `vendor/libvpx` v1.16.0
> from source). `video_desktop.rs` is now a thin adapter; `video.rs` and the whole
> Android MediaCodec path are UNTOUCHED.

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

‼️ **The `oxideav` line below is OUT OF DATE — see "Correction: OxideAV
re-evaluated" in M2.** Re-checked 2026-07-20: the org has 145 crates, the video
decoders claim conformance-corpus byte-exact output, and the HW bridges
(`oxideav-vaapi` et al.) are real and well-designed. Still not adopted, but for
maturity reasons, not because it is vapourware.

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

> ⚠️ **SUPERSEDED 2026-07-26.** This per-crate decode matrix was NOT what shipped:
> desktop decode is now the single GStreamer backend (SW `avdec_*` + HW
> va/d3d11/vtdec), so `openh264`/`libde265`/`dav1d` were never adopted / were deleted,
> and the H.265 software gap below is closed by `avdec_h265`. See the M2 header block.

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

**Done when (M1):** `ffmpeg-next` is out of `Cargo.toml`, a Signal desktop call works,
and a plain `cargo build` needs no system media library. — ✅ all three met.

---

## Outcome (M1, 2026-07-20) — where the implementation DIVERGED from this proposal

Read this before trusting the design sketch above; four things changed.

1. **The crate is DESKTOP-ONLY and `video.rs` was NOT touched.** The plan had the
   shared types moving into `wandr-video`. Wrong: Android encodes *and* decodes in
   HW via MediaCodec and must not link a codec library. `wandr-video` sits in the
   same `cfg(not(target_os = "android"))` table `ffmpeg-next` did and owns only a
   *codec* vocabulary (`Codec`/`CodecError`/`EncoderParams`/`DecoderParams`/`Packet`);
   the host keeps its WIT-shaped types and `video_desktop.rs` maps at the boundary.
   Camera facing, preview rects and z-layer never belonged in a codec crate.
2. **Own `wandr-vpx-sys` instead of `env-libvpx-sys`.** That crate is MPL-2.0 (odd
   in a licence-driven task) and its build script can only *consume* a prebuilt
   libvpx. Ours compiles `vendor/libvpx` into `OUT_DIR`, or honors `VPX_LIB_DIR`
   (the Windows/vcpkg path). This is what makes a plain `cargo build` self-contained.
3. **Windows uses vcpkg** `libvpx[core,realtime]:x64-windows-static-md`, because
   libvpx's build is a POSIX configure emitting `vpx.sln` for msbuild. The triplet
   matters: `-static-md` = static lib + *dynamic* CRT (`/MD`), matching rustc's
   msvc target. Plain `-static` (`/MT`) → LNK4098; plain `x64-windows` → a `vpx.dll`,
   reintroducing the very problem this task removed.
4. **`set_bitrate` is now REAL** (`vpx_codec_enc_config_set`) — it was a no-op under
   ffmpeg-next, so desktop never honored REMB/TWCC. Also, a camera frame whose size
   differs from the encode size is now resized rather than dropped.

### Four gotchas that cost time — see `[[reference_libvpx_wandr_video]]`

* `rc_target_bitrate` is **kilobits/s**; ffmpeg's `set_bit_rate` took bits/s.
* Colorspace must be **BT.601 + limited range on BOTH directions** (swscale's default).
* `vpx_enc_frame_flags_t` is C `long` → **64-bit on LP64, 32-bit on LLP64**. A
  hand-written `i64` constant compiles on Linux and fails Windows with E0308.
* `mem::zeroed()` on `vpx_codec_enc_cfg_t` is **UB** (niche field) and aborts.

None of the first three *error* — they produce well-formed packets and plausible-looking
video — which is why `tests/roundtrip.rs` asserts on decoded PIXELS with an empirically
measured threshold (correct 1.68 MAE; BT.709 mixup 7.93; full-range mixup 9.36 → bar 4.0).
An earlier guessed threshold of 20 passed both bugs.

### Verified

| Platform | Evidence |
|---|---|
| linux-x86_64 | CI ✓ · selfview 59.1/60 fps · camera→VP8→decode 60/60/60 · `ldd` clean |
| macos-x86_64 | Intel Mac (macOS 12.7.6) ✓ build + 5/5 codec tests + camera selfview 52 fps; `otool -L` shows **only system frameworks**; `minos 12.0` |
| macos-aarch64 | CI ✓ |
| windows-x86_64 | CI ✓ · **Signal video call works (user-verified)** |
| android-aarch64 | CI ✓ (MediaCodec path unaffected) |

Not covered: HW backends (VAAPI/VideoToolbox/MediaFoundation) — still the sequencing
step 3 above, with libvpx as the fallback. H.264/AV1 remain unbuilt until an app asks.
**M2 below is that ask.**

---

# M2 — media playback (🔲 NEXT)

> Scoped 2026-07-20. Order is deliberate: **make the host able to play media, prove it
> end-to-end with real apps, then propose upstream** — the `wasi:canvas` path (it earned
> its shape carrying Compose, Slint, dioxus, Avalonia and OpenSwiftUI before it was ever
> a proposal document). We are NOT building host capability because a Jellyfin client
> needs it; a Jellyfin client is how we prove the host capability is right.

## The gap: the decoder is RTP-shaped, so it cannot play a file

M1 finished the *codec* lane. What is missing is the *playback* shape. Today
`wandr:video`'s decoder takes only `{data, timestamp: u32 (90 kHz RTP), keyframe}`:

- **No PTS.** That `u32` is a transport clock that wraps every ~13.25 h, not a
  presentation time.
- **No present feedback.** The only observability is `ready() -> bool` and
  `decoded-frames() -> u64` (a diagnostic counter). The guest cannot learn what is on
  screen, so **it cannot slave video to `wasi:audio playback.position()`** — the only
  real media clock in the system. A/V sync is therefore impossible today.
- **No flush/reset**, so a seek means dropping and reopening the decoder.
- **No EOS, pause, or rate control.**
- `queue-full` is documented as *"frame dropped, resend after a keyframe"* — lossy by
  design. Correct for RTP, wrong for a file.

Concrete evidence this is real rather than theoretical: **`wasi:media-session` (shipped,
wired both directions — `media_session_host_impl.rs`) already delivers a `seek-to`
transport intent that the video decoder has no verb to honor.** The transport vocabulary
arrived before the decoder could implement it.

## What already exists (do NOT rebuild)

| Need | Status |
|---|---|
| PCM out + media clock (`position()`) | ✅ shipped, device-verified (`wasi:audio`) |
| Guest-side audio decode | ✅ shipped — Symphonia in `wandr.audio.player` (FLAC/MP3/AAC/WAV/OGG/MP4) |
| Now-playing + transport intents | ✅ **shipped and wired** (`wasi:media-session`) |
| Demux / containers / HLS / DASH / ABR | ✅ deliberately **guest-side** — see `proposals/wasi-media-source/NOTES.md`; no host contract needed |
| HTTPS + range requests | ✅ works (`wandr-reqwest`; arbitrary headers pass through) |
| Frame callback + on-demand pacing | ✅ `on-frame(nanos)`, `next-frame-delay` |
| DRM | 🔲 `wasi:eme` sketch, ClearKey-only |
| HW audio decode offload | 🔲 `wasi:audio-codec` sketch — optional, Symphonia already suffices |

Two stale doc claims found while scoping: `wasi:media-session`'s header says `NOT WIRED`
(it *is* wired — host impl + `.ok()`-probed guest export), and task 108 M2 is unmarked in
`STATUS.md`. Fix both.

`proposals/wasi-media-source/NOTES.md` item 3 already anticipated this milestone:
*"a real A/V MSE player would exercise it and may surface a need for a true presentation
timestamp."* It does. That is M2.

## The delta — extend `wandr:video` first

`wasi:audio-codec` is the in-tree precedent and already has the right shape (it calls
itself "the audio sibling of the in-tree `wasi:video-decoder`"): `timestamp-us: s64`
(WebCodecs unit), a real `flush()`, `probe()` = `isConfigSupported`, and the
TRANSCODE-vs-TUNNEL duality. Video kept the call shape because it was factored out of
the call contract. Bring them into symmetry.

**W3C has two models and neither fits alone.** WebCodecs `VideoDecoder` gives the app
frames (`decode(chunk)` → `output` callback with `VideoFrame{timestamp}`, plus `flush()`,
`reset()`, `decodeQueueSize`) and the **app presents** — but that means pixels crossing
the boundary, which kills decode-to-surface. `HTMLMediaElement` (`<video>`) keeps pixels
host-side but the **UA owns the clock** (`currentTime`, `playbackRate`) — one sync policy
for every player. wandr is `<video>`-shaped with *no* clock: currently the worst of both.

The resolution is what the hardware already does — **Android
`MediaCodec.releaseOutputBuffer(index, renderTimestampNs)`**, Apple
`AVSampleBufferDisplayLayer` (enqueue `CMSampleBuffer` with a presentation time), and the
Media Foundation / VAAPI equivalents: **the app schedules presentation of an opaque
buffer by timestamp, without ever seeing pixels.** WebCodecs' control flow, `<video>`'s
zero-copy.

```wit
/// Opaque — pixels never cross the boundary.
resource decoded-frame {
    timestamp-us: func() -> s64;
    /// Schedule presentation (= releaseOutputBuffer(idx, renderTimestampNs)).
    present: func(at-ns: u64);
    /// Late frame, or dropped by a seek.
    discard: func();
}

submit:     func(chunk: encoded-chunk) -> result<_, codec-error>;  // timestamp-us
next-frame: func() -> option<decoded-frame>;
flush:      func();               // = WebCodecs flush()  — end of stream
reset:      func();               // = WebCodecs reset()  — SEEK
queue-size: func() -> u32;        // backpressure, replacing the lossy queue-full
```

**Sync stays guest-side** — the guest slaves `present(at-ns)` to
`wasi:audio playback.position()`. Not because of any dependency rule, but because sync
POLICY differs per player: live vs VOD, frame-drop vs audio-stretch, seek-accuracy vs
latency. A host-owned clock gives every player one policy. The guest already has
`on-frame(nanos)` + `next-frame-delay` to pace with, and the audio player already does
anchor-based clock work guest-side.

Keep the frame **opaque** (a resource handle, not a pointer or fd) — that is what leaves
interposition/virtualization open later, and it costs nothing now.

**Order:** extend the fused, shipping `wandr:video` → prove → *then* factor into
`proposals/wasi-video-decoder` (which today is a re-factoring of the same call shape and
does NOT close this gap) for the eventual upstream proposal.

## Contract hygiene M2 must fix while it is in there

A deep audit of the whole `wasi:audio*` / `wasi:media-*` / `wasi:video-*` /
`wasi:eme` family (2026-07-20) turned up three things M2 touches anyway. Fix them
here rather than leaving them to be discovered by the next consumer.

1. **The "same error vocabulary" claim is FALSE.** `wasi:audio-codec` states
   *"Same vocabulary as wasi:video-decoder so guest fallback is uniform"* — but
   audio has `bad-data` + `sink-unavailable` while video has `bad-frame` +
   `surface-unavailable`. A guest cannot write one match arm across both, which
   was the entire stated goal. M2 already replaces the lossy `queue-full` with
   `queue-size` backpressure, so it is editing this enum — converge the two.

2. **Timebase sprawl: four units across five packages, no conversion authority.**
   device frames (`wasi:audio`), microseconds (`wasi:audio-codec`), 90 kHz `u32`
   (`wandr:video` / `wasi:video-decoder`), `f64` seconds (`wasi:media-session`).
   M2's move to `timestamp-us: s64` on video is not just a fix for the wrapping
   `u32` — it **collapses this to three and aligns video with its audio sibling**.
   State that as an explicit goal so it is not undone later. Seconds stay at the
   `media-session` edge (W3C-mandated) and frames stay at the `wasi:audio` edge
   (device-mandated); µs becomes the one codec-lane unit.

3. **`playback-rate` is an orphan.** `wasi:media-session`'s `position-state`
   *publishes* a rate, but no package has a verb to SET or APPLY one — repo-wide
   there is exactly one mention. M2 lists rate control, so it must either claim
   the verb or explicitly defer it. (Note rate also implies audio resampling, so
   it is not a video-only decision.)

## Open questions inherited from `wasi-media-source` (closed 2026-07-20)

That package is now formally closed — no WIT will be written. Two of its four
"needs talks" questions were unowned and move here:

- **Live / LL-HLS jitter buffering** — host primitive, or is
  `buffered-frames` + `position()` enough for the guest to self-pace? Recorded
  lean: guest is enough, *unproven*. The YouTube/adaptive cell of the matrix
  below is what settles it.
- **Container/segment edge formats** — confirm no codec needs HW-only init data
  that cannot pass through `decoder-config.description` (fMP4/CMAF init segments,
  HLS TS). Expected answer: no host work.

(The other two are resolved: DRM → `wasi:eme` ClearKey-only; presentation
timestamp → this milestone.)

## Codecs — M1's sequencing steps 3 and 4, now triggered

### Device capability — MEASURED 2026-07-20, not read from a config file

`wandr-host --probe-video codecs` (new) opens each MIME, asks
`AMediaCodec_getName` which component answered, then configures + starts it at
1920x1080. Pixel 2 XL / msm8998, under `--no-art`:

| Codec | Decode | Encode |
|---|---|---|
| H.264 | ✅ **HW** `OMX.qcom.video.decoder.avc` | ✅ HW |
| H.265 | ✅ **HW** `OMX.qcom.video.decoder.hevc` | ✅ HW |
| VP8 | ✅ HW | ✅ HW |
| VP9 | ✅ HW | ⚠️ SW `c2.android.vp9.encoder` |
| **AV1** | ✅ **SW** `c2.android.av1-dav1d.decoder` | SW |
| MPEG-4, H.263 | ✅ HW | ✅ HW |

‼️ **AV1 decode exists on device and the vendor XML does not mention it.**
`/vendor/etc/media_codecs*.xml` declares no AV1 at all, and lists only `OMX.*`
entries so every Codec2 (`c2.*`) component is invisible to it. Trusting the file
would have led to bundling dav1d for Android unnecessarily. **Ask the API.**

Two consequences for the plan below:
- **The H.265 software gap is DESKTOP-ONLY.** On device it is hardware.
- **Do not bundle dav1d for Android** — the platform already ships it. Desktop
  still needs its own AV1 decoder if we ever want AV1 there.

Caveat: `configure`+`start` proves the component opens and the HAL is reachable
under `--no-art`. It does NOT prove it survives a real bitstream — that is what
step 1 of the matrix exercises.

### Correction (2026-07-20): OxideAV re-evaluated — M1's dismissal was wrong

M1's Finding 1 dismissed `oxideav` in one line ("27 dl, claims every backend but
is an experiment"). Re-checked in depth; that is no longer accurate and was
unfair even then. **Note the GitHub repo *descriptions* are stale — read the
READMEs.** Actual claimed state (145 repos in the org, MIT, pushed daily):

| Crate | Description says | README says |
|---|---|---|
| `oxideav-h264` | "I-slice only" | I/P/B, CAVLC+CABAC, MBAFF, PAFF, 4:2:0/4:2:2/4:4:4, **byte-exact vs a reference binary** |
| `oxideav-h265` | "NAL/SPS/PPS parse" | end-to-end, **16/16 conformance fixtures byte-exact**, I/P/B pyramid, SAO |
| `oxideav-av1` | "partial intra" | **16/16 independent corpus byte-identical to a third-party decoder**, KEY+P inter, 10/12-bit |
| `oxideav-vp9` | "partial intra" | intra + inter P-frame end-to-end, pixel-accurate encoder |

More interesting for us than the codecs: **the HW bridges exist and are real** —
`oxideav-vaapi` (~130 KB of Rust: `decoder.rs` 48 KB, `sys.rs` 27 KB), plus
`-videotoolbox`, `-nvidia`, `-vdpau`, `-vulkan-video`. The design is exactly what
step 3 needs and is worth reading before we write ours:

* **runtime `libloading`** — no compile-time `libva` dep, no `*-sys`, no headers
  shipped; the build works on a machine with no GPU stack at all;
* **priority registry** — HW factories register at priority 10, pure-Rust at 100+,
  lower wins, so HW-first is the default and fallback is automatic;
* **two distinct fallback paths** — load failure (no `libva.so`, no `/dev/dri`)
  and init failure (`VAStatus` != 0 for this resolution/profile);
* `require_hardware: true` to opt OUT of silent degradation.

**Decision: do NOT adopt the SW codecs for M2. Do read the HW bridge design.**
Reasons, in order:
1. libvpx is BSD-3, statically linked, ~20 years of production hardening, and is
   already shipped and verified on 5 platforms. Replacing it with a 3-month-old
   decoder is strictly more risk for no requirement — 117's own preference order
   says pure Rust is *preferred, not required*.
2. Codecs are the component class where bugs produce **plausible-looking wrong
   output** rather than errors. This task hit that twice (kilobits-vs-bits,
   BT.601-vs-709) and only pixel-level checks caught it.
3. Maturity is genuinely early: the org is 3 months old, `oxideav-vaapi` is
   v0.0.3 with ~181 downloads, `oxideav-h264` ~1000. Conformance corpora are
   16 fixtures where the real suites (JCT-VC, Argon) run to hundreds/thousands.
   The "round 420 / Hat-2 clean" cadence indicates heavily automated development.

**SPIKE RUN 2026-07-20 — `repros/oxideav-spike/` (FINDINGS.md).** Evaluated it as
a ready component on two machines (WSL2/no-GPU, and Pop!_OS with a real Intel+NVIDIA
GPU). The pure-Rust decoders are real (H.265 + AV1 decoded 300/300 frames from
actual clips), but the integration layer is early and has one showstopper:
**H.264 with HW enabled decodes 0 frames and reports SUCCESS, on both machines,
across NVDEC/VAAPI/Vulkan — `--no-hwaccel` fixes it at 300/300.** That is exactly
the silent-HW-failure the priority-registry design is meant to prevent, and it
doesn't. Plus VP9 registers nothing (`oxideav-vp9/src/lib.rs:867` is an empty
`register()`), and H.265 is mis-typed as Audio. Conclusion holds: libvpx stays;
re-run `run-spike.sh` per oxideav release; the silent-failure bug is the precise
risk to design against in our OWN step-3 HW lane — **fallback must key on frames
produced, not on the absence of an error return.**

**Revisit when** the corpora and adoption grow — the AV1 + H.265 story in
particular could delete real work. A cheap near-term use with none of the risk:
run `oxideav-vp9` as an independent **cross-check oracle** against our libvpx
output in tests; two independent decoders disagreeing is an excellent bug
detector.

### Correction: LGPL is a preference, not a wall

M2 earlier wrote off software H.265 partly because `libde265` is LGPL. That
over-hardened M1's own rule — the stated order is **pure Rust → permissive C
(static) → LGPL → HW-only**, so LGPL is third choice, *allowed*. `libde265`
therefore stays on the table as the H.265 software fallback if HW is
unavailable and we decide we need one.

### Correction: openh264 and libde265 are SOFTWARE-ONLY

They are not "libraries that handle HW/SW" — Cisco's openh264 is a software H.264
codec and libde265 a software HEVC codec. Neither touches VAAPI / VideoToolbox /
Media Foundation. So the HW lane is a **separate, per-platform thing we write**:

| Platform | HW lane (we implement) | SW fallback |
|---|---|---|
| Linux | VA-API — `cros-codecs` (BSD-3), or the `oxideav-vaapi` design | openh264 / libde265 |
| macOS | VideoToolbox (also has native `present-at-time`) | " |
| Windows | Media Foundation / D3D11VA | " |
| Android | MediaCodec — ✅ already shipped | platform's own |

Four HW backends is the real work in step 3; the codec libraries are the easy part.

### AV1, per platform

* **Android** — nothing to do: the device already ships
  `c2.android.av1-dav1d.decoder` (measured above).
* **Desktop SW** — `dav1d` (BSD-2, mature) via `libdav1d-sys`.
* **Desktop HW** — only Intel Xe/Arc, NVIDIA 30-series+, AMD RDNA2+. Most
  existing laptops will decode AV1 in software regardless.

### Step 2 result (2026-07-21) — backend registry + software H.264

**The registry (the decoupling seam).** Runtime codec selection modelled on
oxideav's design: `CodecBackend` (name / kind{Hw|Sw} / priority / supports /
open_*), a `Registry` that walks candidates by priority and falls to the next on
`Err`, and `Preferences { no_hardware, require_hardware }`. Adding a backend — a
HW lane, or oxideav once its fallback bug is fixed — is one new file + one line in
`default_registry()`. The spike's lesson is the registry's **fallback contract**:
a backend must return `Err` from `open_*` if it will not actually work, because
the registry can only fall through on `Err`; `require_hardware` surfaces the HW
error instead of silently degrading.

**Software H.264 via openh264** (Cisco, BSD-2, built from source; desktop-only —
Android uses MediaCodec). Decode MAE 1.64 (VP9 is 1.68), PTS preserved+monotonic,
registry routing + the require_hardware guard all teeth-checked. openh264 encodes
too, so the test round-trips in-process with no file or demuxer.

‼️ **KNOWN LIMIT → step 2b.** openh264's decoder returns no per-frame timestamp,
so PTS is paired by FIFO — valid ONLY without B-frames. Our encoder is
`CameraVideoRealTime` (no B-frames) so the round-trip is sound, but a real H.264
*file* can carry B-frames (decode order != presentation order) where the FIFO
mispairs. The reorder buffer + feeding a real MP4 (the `h264_mp4toannexb`
question) is **step 2b**, still to do. Also not yet done: **wiring H.264 into the
host** (extend `video::Codec`, enable the `openh264` feature on the host dep) —
the equivalent of step 1b for H.264.

### Step 2b result (2026-07-21) — H.264 wired into the host, real MP4 plays

H.264 is now decodable end-to-end through the actual host, on real content:

* **Host codec path wired** — `video::Codec::H264` + `"video/avc"`, `codec2b`
  accepts it, `codec_of` maps it, the desktop dep enables the `openh264` feature.
  This lights up H.264 on BOTH backends: desktop software (openh264), Android
  MediaCodec HW. H.265 stays rejected (no desktop software decoder).
* **`h264_mp4toannexb`** implemented (~40 lines, no crate) — MP4's length-prefixed
  avcC NALs → Annex-B, SPS/PPS at each keyframe.
* **Reorder buffer** — the desktop present queue now inserts in PTS order, so
  B-frame streams (decode order != presentation order) present correctly. VP8/VP9
  pay nothing (they always append).
* **`--video-decode-file <mp4>`** plays a real file through the host VideoDecoder.

**MEASURED, bbb-h264.mp4 (1280x720, real B-frames): 300 samples → 300 decoded,
300 presented, 0 dropped, presentation order OK, drift avg 0.8 ms.** The desktop
"local file" matrix cell is green on real content, and the snapshot is a correct
decoded frame.

**Real bug found and fixed along the way:** openh264 defaults to flushing after
every decode, which the crate's own docs warn against; on a real B-frame file it
overflows the reorder buffer at the 2nd GOP and loses 236/300 frames while
returning success per frame. The backend now opens with `Flush::NoFlush` +
`flush_remaining` at EOS. This is exactly the "real files are a different test
than a self-encoded loopback" lesson — the in-crate round-trip never hit it. See
`repros/h264-mp4-decode`.

STILL OPEN in M2: HW backends (VAAPI on popos first), and H.265 (HW-only on
desktop). The `--video-decode-file` diagnostic + reorder buffer are the reusable
scaffolding for the rest of the matrix.

### Step 2c result (2026-07-21) — software H.265 via oxideav-h265

The H.265 software gap is FILLED, and without LGPL. Adopted `oxideav-h265`
(pure-Rust MIT, ~99% conformance) as a wandr-video backend — using ONLY the codec
crate, bypassing oxideav's registry and its unfinished HW backends. Wired into the
host exactly like H.264: `video::Codec::H265`, `"video/hevc"`, the openh264-style
adapter, and `--video-decode-file` extended to parse hvcC.

**MEASURED, bbb-h265.mp4 (1280x720, real B-frames): 300/300 decoded, presentation
order OK, snapshot pixel-correct.** Both platforms build (Android decodes HEVC in
MediaCodec HW).

Two things this surfaced:
* **A latent PTS-mispairing bug, caught by verification.** oxideav-h265's doc says
  `take_decoded` returns display order; measured (POC 0,5,3,1,2,4,…) it returns
  DECODE order. The first cut used pop-min pairing, which would have silently
  desynced A/V while passing a no-loss check. Fixed to FIFO (like openh264). Only
  found by reading the decoder's POC sequence — a green "no loss" was not enough.
* **Software HEVC is not real-time here.** ~6 fps for 720p on an i7-8565U, so the
  paced player drops most frames. Decode is CORRECT; it is just slow. This is why
  HEVC playback should lean on HW — VAAPI (hardware confirmed on fedora via
  vainfo) / MediaCodec is the real-time path; oxideav-h265 is the correctness
  fallback for non-real-time or lower resolutions.

Codec status after this step: **VP8/VP9 (libvpx), H.264 (openh264), H.265
(oxideav-h265) all decode real files on desktop; HW backends are the remaining
work for real-time HEVC/AV1.**

### Step 2d result (2026-07-21) — real-time HEVC (libde265) + AV1 (dav1d)

Step 2c's honest caveat — pure-Rust `oxideav-h265` decodes correctly but at ~4–6
fps, too slow to *play* — pushed the software floor to two mature C decoders,
both **built from source and linked statically** (no runtime `.so`), added as
registry backends with no change to the call path:

* **H.265 real-time — `libde265` (LGPL-2.1).** ~60 fps single-thread / ~107 fps
  multi-thread @720p vs oxideav-h265's ~4 fps (`repros/libde265-bench`). LGPL is
  task 117's third-choice tier, allowed for a real need; static linking carries a
  §6 relink obligation that wandr's open-source build satisfies (packaging detail
  is a task-118 concern — see the backend's module doc). Registered at **priority
  50**, so when both HEVC software decoders are compiled in, the real-time one
  wins and `oxideav-h265` remains the permissive-MIT correctness fallback.

* **AV1 — `dav1d` (BSD-2).** ~258–355 fps @720p (`repros/av1-bench`), and unlike
  `oxideav-av1` — which fails outright on standard matroska AV1 framing
  (`UnexpectedEnd`) — it decodes the whole file. BSD-2 is the permissive-static
  tier, so AV1 is licence-cleaner than HEVC. New `Codec::Av1` (`"video/av01"`)
  threaded through `wandr-video` → `video.rs` → `codec2b`/`codec_of`; the
  `Dav1dBackend` (priority 50, `BackendKind::Software`, decode-only) repacks 8-bit
  I420 and passes PTS natively via `Picture::timestamp()` (no FIFO — dav1d emits
  display order). Built statically by `dav1d-sys`'s internal meson path
  (`SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=always`, now set in all three desktop build
  scripts + CI; Windows uses that same meson path, NOT vcpkg, because meson is
  MSVC-native unlike libvpx's POSIX configure).

**MEASURED, `bbb-av1.webm` (1280×720, 300 frames):** the WIRED path
(`open_decoder(Av1)` → registry → `Dav1dBackend` → dav1d) decodes **300/300**, all
1280×720, PTS preserved in display order, Y plane tightly packed — and its
`flush()` recovers the one trailing frame a naïve raw-dav1d drain loop drops
(299→300). Both platforms build (Android never enables these features — it decodes
HEVC in MediaCodec HW and AV1 via `c2.android.av1-dav1d.decoder`).

Codec status after this step: **VP8/VP9 (libvpx), H.264 (openh264), H.265
(libde265 real-time + oxideav-h265 fallback), AV1 (dav1d) all decode real files on
desktop.** The software matrix is complete; **HW backends (VAAPI first — confirmed
on fedora) are the remaining work**, for battery and 4K real-time.

### HW HEVC on Windows via DXVA2 (2026-07-25) — pixel-correct

The Windows DXVA `d3d11` backend, which shipped H.264 HW decode, now decodes
**H.265 in hardware** (`wandr-host` 669af40). New `backends/hevc_dxva.rs`
(exact `DXVA_PicParams_HEVC`/qmatrix/short-slice from dxva.h + LSB-first bitfield
packers) + `backends/hevc.rs` (`HevcDxva`): reuses the H.264 backend's D3D11
device/pool/submit/readback, drives cros-codecs' pure-Rust HEVC parser +
`PictureData::new_from_slice` (POC 8.3.1), and a thin driver for the RPS
derivation (8.3.2), the DPB (make-room bump C.5.2.2 + display-order bumping),
and RASL drop (8.1.3). The driver builds RefPicListL0/L1 from the DXVA
`RefPicSetStCurrBefore/After/LtCurr` sets we fill. `d3d11.rs` probes
`HEVC_VLD_MAIN(10)` and routes `Codec::H265` to it; H.264 path unchanged.

VERIFIED pixel-correct via `--video-decode-file` (headless, from WSL over the
Windows GPU): `bbb-h265.mp4` (1920×1080, B-frames) → **300/300 decoded,
presentation order OK**, decoded snapshot eyeballed clean, logs clean.

Four bugs found+fixed in the bring-up, each caught by the pixel/decode loop:
(1) DPB `max_num_pics` never set → store failed immediately; (2) parser
re-priming stored `nalu.as_ref()` (no start code) not `nalu.data` → PPS not
re-registered; (3) only the *current* picture's RPS sets were kept, evicting the
`foll` references a later picture needs → "no ref" on future frames; (4)
before-decoding `remove_unused` + C.5.2.2 bump was missing.

‼️ The "unavailable ref" debug lines at stream/GOP boundaries are spec-expected
(generic RPS entries pointing before the IRAP / across POC-wrap; the accelerator
handles them — cros-codecs logs the same). The frame drops in the diagnostic are
**1080p CPU-readback pacing** in the strict real-time harness (non-deterministic
run-to-run), not a decode defect; the zero-copy present path removes the readback.
The build is `d3d11`-only locally (drops libvpx/dav1d/etc. to skip
cmake/nasm/vcpkg) — CI/full builds keep the software fallbacks.

### On-screen + zero-copy (2026-07-25) — user-verified

The full guest player (`wandr.video.player`) plays `bbb-h265.mp4` on the Windows
desktop through the DXVA HEVC HW decoder, decode-to-surface at real-time 30 fps —
**user-confirmed looking correct on screen** (the last visual glance; the
`--video-decode-file` PNG had already proven the pixels, and the windowed run
proved the HW path end-to-end: `h265 d3d11 HARDWARE`, `decoder OPEN ✓
decode-to-surface`).

**Zero-copy present path DONE + verified** (wandr-host 21231bd): the HEVC backend
now emits `Frame::gpu` (a D3D11 NV12 texture on ANGLE's device) instead of the CPU
readback — the host imports it into ANGLE GL via `EGL_ANGLE_image_d3d11_texture`
(pixels never leave the GPU). Proven from the log: `d3d11 decode will use ANGLE's
device` + `live textures 18` (the CPU lane shows `live textures 0`), i.e. the
decoded textures ARE imported into GL. This also removes the per-frame 1080p
readback that caused the diagnostic's frame-drop edge. Decode-on-ANGLE's-device +
`export_texture` + shared `D3d11Owner`/`GpuTex`, mirroring the H.264 backend.

‼️ WSL-launch capture gotcha: a GL window launched via WSL→Windows interop does
NOT present to the visible window (renders white for ALL codecs — H.264 too, so
not codec-specific); run on the real desktop to SEE it. Headless
`--video-decode-file` + the log signals fully verify the HW path from WSL.

Remaining HW: VAAPI HW HEVC on Linux (confirmed on fedora via vainfo; not yet
wired). Windows HW H.265 is COMPLETE — decode pixel-correct, zero-copy, on-screen.

### The steps

Real content is H.264/HEVC/AV1, so playback forces what M1 deferred:

1. **H.264 decode** — HW on every platform; `openh264` (BSD-2) as the software floor.
2. **H.265 decode** — HW everywhere since ~2015; the software gap is real and documented
   above (`rust_h265` v0.1 / LGPL `libde265`). Mitigation stands: lean on HW, else return
   `no-hw-codec`. Patents are an axis separate from the code licence — HW decode also puts
   the codec in the user's already-licensed driver rather than in our binary.
3. **HW backends** — VAAPI (`cros-codecs`) / VideoToolbox / MediaFoundation / MediaCodec,
   with libvpx as the software fallback. `present(at-ns)` maps onto each natively.
4. **AV1** — `dav1d` decode when something needs it.

‼️ **Do not forget the bitstream filter.** Feeding a HW decoder from MP4 needs
`h264_mp4toannexb` (length-prefixed → Annex-B). No crate exists; ~100 lines. It is in the
FFmpeg audit table above under "easily forgotten" and will otherwise present as "HW decode
silently outputs nothing".

## Step 1 result (2026-07-20) — contract added, and one thing it CANNOT prove

The playback shape is implemented in `wandr-video` and green on desktop/VP9:
`Chunk{data, timestamp_us: i64}` in, `I420Ref.timestamp_us` out, plus `flush()`
and `reset()`. PTS rides through libvpx in `user_priv` (the exact mechanism —
libvpx guarantees PTS-order output and a packet may yield zero frames, so an
external FIFO would silently desync). `tests/playback.rs` covers PTS survival,
seek, EOS, awkward/large PTS values, and pacing against an external clock.

‼️ **`flush()` and `reset()` are NOT proven by step 1, and cannot be.** Each was
verified by injecting a no-op implementation: every test still passed. That is
not a test defect — on libvpx VP8/VP9 both verbs genuinely have nothing to do:

- a keyframe resets all references by definition, so once the caller honours
  "feed a keyframe after reset" there is no observable difference. (Probing with
  a delta frame instead does not work either — measured: libvpx rejects an
  out-of-order delta with `BadFrame` whether or not reset ran.)
- with `g_lag_in_frames = 0` and realtime CBR, VP9 emits no alt-ref/hidden
  frames, so the decoder never holds a tail for `flush` to drain.

Both verbs earn their place on backends that queue work asynchronously, where
they map to a real discard — `AMediaCodec_flush` drops in-flight buffers, and a
HW decoder with B-frames genuinely holds a tail. **So the matrix below must
validate flush/reset on MediaCodec (step 2+), and the desktop row cannot stand in
for it.** The tests carry this limitation in their doc comments so the next
person does not read green as proof.

What step 1 DID prove, and it is the load-bearing part: presentation timestamps
survive the codec unchanged and correctly paired, seek-by-reset resumes at the
right frame with the right PTS, and frames can be paced against an external clock
within one 60 Hz tick — i.e. **A/V sync is expressible**, which is exactly what
the call-shaped decoder made impossible.

### Step 1b result (2026-07-20) — desktop cell GREEN

`--video-playback-test` drives the playback path end-to-end headlessly: encodes a
synthetic VP9 clip, decodes ahead into a cushion, and presents each frame when an
independent clock reaches its PTS.

    150/150 presented, 0 dropped | drift avg 1.2 ms / max 8.9 ms
    (frame interval 33 ms) | wall 4.97 s vs media 5.00 s | order OK

Teeth-checked: with the PTS dropped it fails on four independent signals at once
(38/150 presented, 112 dropped, wall 0.41 s vs media 5.00 s, order BROKEN).

The desktop decoder now carries `submit_for_playback` / `present_due` /
`finish_playback` / `seek_reset`. Frame-drop policy (present the newest due
frame, count the rest dropped) lives on the host adapter, NOT in the codec —
different players want different policy, which is why sync is guest-side.

Desktop holds a queue of RGBA because frames return to the CPU. **Android will
not**: `AMediaCodec_releaseOutputBufferAtTime` hands the PTS straight to the HW
compositor and the pixels never leave the GPU. The desktop queue is the stand-in
for that primitive, and step 2 should use the native one rather than porting this.

## Done when — the proving matrix

One player on one backend proves nothing portable. Both axes must be green, which is the
same bar `wasi:canvas` cleared with five UI frameworks:

| | libvpx (desktop SW) | MediaCodec (Android HW) | VideoToolbox / MF |
|---|---|---|---|
| Local file (MP4/MKV) | | | |
| Jellyfin (direct-play **and** server-transcode) | | | |
| YouTube (adaptive / DASH) | | | |

Plus, on every cell: **A/V stays in sync over a long play**, **seek is accurate and fast**
(`reset()`, not reopen), **pause/resume**, and **EOS is clean**.

Jellyfin is the richer first client — its `DeviceProfile` lets the client declare codec
support and the server transcodes the rest, so it exercises both direct-play (real H.264/
HEVC decode) and a fallback path we control. YouTube adds adaptive bitrate and segment
switching.

## Explicitly NOT in M2

- **A `wandr:media` composition package.** RETIRED 2026-07-20 — see
  `docs/wandr-media-scope.md`. It reserved A/V sync + transport, but transport
  went to the shipped `wasi:media-session` and its sync justification (*"neither
  side can see the other's clock from the guest"*) was dissolved by its own
  prerequisite: `playback.position()` shipped in task 108 M1. Do not resurrect it
  as a home for M2's verbs — that is the duplicate this audit was run to prevent.
- **Re-opening `wasi:audio-codec` / `wasi:audio-effects`.** Their original
  trigger was task 108 M4's battery problem, which was solved by a *different*
  mechanism (role-based deep-buffer) after measurement showed the Pixel 2 XL has
  no HW audio decoder and no `COMPRESS_OFFLOAD` at all. Symphonia already carries
  audio. Reopening them needs a NEW justification.
- Containers/demux/HLS/DASH host-side — guest work, per `wasi-media-source/NOTES.md`.
- DRM beyond the existing ClearKey sketch (`wasi:eme`); Widevine needs a TEE + a
  Google-provisioned CDM and is device-only.
- HW audio decode (`wasi:audio-codec`) — Symphonia already carries audio; offload only if
  M1's decode-CPU/battery numbers justify it (task 108 M4's trigger).
- Encoding anything. Playback is decode-only.
- Posting to WASI. Prove first.
