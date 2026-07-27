# Task 117 — M3: video output-format negotiation (10-bit / P010)

> **Status: SCOPED. M3 deliverable decided 2026-07-27 — "play most videos": an
> explicit, LOGGED 8-bit down-convert fallback so 10-bit content plays instead of
> freezing. Quality is deliberately NOT the M3 goal — true 10-bit precision +
> zero-copy + HDR are deferred to M3b/M4 (this is a runtime that may drive a
> 10-bit/HDR panel, so preserving 10-bit is a real future milestone, just later).**
> Continuation of task 117 (desktop video via GStreamer); M2 proved the 8-bit NV12
> zero-copy path on all three OSes. Host-side only — no guest, no WIT, no container
> change.

## Problem (observed 2026-07-27)

Streaming real Jellyfin titles through `wandr.jellyfin`:

| Title | Codec | Profile | Pixel format | Bit depth | Result |
|---|---|---|---|---|---|
| Crime 101 | HEVC | Main | `yuv420p` | 8 | ✅ plays |
| Apex | H.264 | High | `yuv420p` | 8 | ✅ plays |
| **Carry-On** | HEVC | **Main 10** | `yuv420p10le` | **10** | ❌ freeze |
| **Measure for Measure** | AV1 | Main | `yuv420p10le` | **10** | ❌ freeze |

The differentiator is **bit depth, not codec or container**. Verified directly:
Carry-On's MKV is 503 `SimpleBlock`s / 0 `BlockGroup` (so the container keyframe
flag is present and correct — an earlier guest "BlockGroup keyframe" hypothesis
was wrong and has been reverted). ffprobe confirms `pix_fmt=yuv420p10le`.

## Root cause

`runtime/wandr-host/crates/wandr-video/src/backends/gstreamer.rs` pins the
appsink output caps to an **8-bit** format on every zero-copy/HW lane:

- Windows: `appsink caps="video/x-raw(memory:D3D11Memory),format=NV12"`
- macOS:   `appsink caps="video/x-raw,format=NV12"`
- Linux:   `appsink caps="video/x-raw(memory:DMABuf),format=DMA_DRM"` — and the
  dma-buf drm-format parser only recognises the NV12 4CC.
- CPU lane: `videoconvert ! video/x-raw,format=I420` (8-bit).

A 10-bit decode emits **P010** (the 10-bit sibling of NV12: same 4:2:0 biplanar
layout, 16-bit samples). The appsink demands NV12; nothing converts P010→NV12;
GStreamer returns **`not-negotiated`** → zero frames → freeze. This is *not* a
hardware limit — Intel UHD 620 decodes HEVC Main 10 natively and hands back a
P010 surface (it has no AV1 HW decoder, so AV1 is dav1d/SW, but that 10-bit
output hits the same wall). It is purely the host's fixed-format output stage.

The zero-copy GL importers were written for NV12 only: on Windows the ANGLE
import samples the NV12 D3D11 texture as two textures (R8 luma + RG88 chroma);
Linux dma-buf and macOS IOSurface imports assume NV12 the same way. So NV12 was
pinned to match the one thing the importer understands — a shortcut, not a design.

## Premise confirmed (2026-07-27, `gst-launch` on the real file)

Reproduced locally in WSL against Carry-On's bytes (fetched from the server) —
no host, no Windows, no code. The failure is exactly a pinned-8-bit appsink
rejecting the decoder's native 10-bit output:

```
BASE = filesrc location=carryon.mkv ! matroskademux ! h265parse ! avdec_h265
```

| Pipeline (decoder → caps) | Result | Meaning |
|---|---|---|
| `ffmpeg -i carryon.mkv -f null -` (full decode) | ✅ decodes | bitstream + container are fine |
| `BASE ! fakesink` (native output caps) | `format=I420_10LE` | decoder emits **10-bit** |
| `BASE ! video/x-raw,format=NV12 ! fakesink` | ❌ **not-negotiated** | **reproduces the freeze** (host HW lane) |
| `BASE ! video/x-raw,format=I420 ! fakesink` | ❌ **not-negotiated** | even an 8-bit pin fails without a converter |
| `BASE ! video/x-raw,format=I420_10LE ! fakesink` | ✅ OK | **fix A** — negotiate the native format |
| `BASE ! videoconvert ! video/x-raw,format=NV12 ! fakesink` | ✅ OK | **fix B** — convert lane (8-bit downconvert) |
| `BASE ! videoconvert ! video/x-raw,format=P010_10LE ! fakesink` | ✅ OK | convert to 10-bit semi-planar (zero-copy-friendly) |

Conclusions: (1) decode + demux are not the problem; (2) the decoder's native
format here is `I420_10LE`; (3) pinning `NV12` with no converter is *precisely*
what yields `not-negotiated` — which is what the host's HW/zero-copy lane does;
(4) both accepting the native format **and** inserting a convert clear it. This
also explains why the host's CPU lane (`videoconvert ! I420`) already plays 10-bit
(it has a converter) while the default HW lane freezes — so `gstreamer-sw` would
pass, but this `gst-launch` proof supersedes that app-level test.

## M3 deliverable — decided 2026-07-27 (staged)

Goal: **most videos play**, 10-bit included (Carry-On, Measure). Output stays
8-bit for now (no quality gain); true 10-bit / HDR is deferred (see "Quality is a
later milestone"). **Target for the HW path is B: convert on the GPU and KEEP
zero-copy** — a GPU→CPU readback would make HW decode pointless. Reached in two
steps.

**Common gating (both steps):** detect the decoded bit depth at runtime from
`bit-depth-luma` on the `h264parse` / `h265parse` / `av1parse` src caps — derived
from the SPS/sequence-header in the AUs the guest already sends, so no WIT change
and no hardcode (derive-from-input). 8-bit content is untouched — it keeps its
existing zero-copy NV12 lane, no regression.

### M3.1 — software first (OS-independent, the starting point)

For >8-bit content, use the **SW decoder** (`avdec_*`) + the existing
`videoconvert ! video/x-raw,format=I420` system-memory lane → 8-bit upload. This
is identical on all three OSes (core GStreamer elements), a single change, and
gets Carry-On + Measure playing. **No zero-copy is sacrificed here** — a SW
decoder outputs into system memory and always uploads a texture anyway, so the
"GPU→CPU roundtrip wastes HW decode" problem does not apply to the SW path.

### M3.2 — HW keep-zero-copy (B, per-OS, the target)

For >8-bit content on the **HW decoder**, insert a **GPU-side** convert
(`d3d11convert` on Windows / `vapostproc` on Linux / `glcolorconvert` on macOS)
that turns `P010` → `NV12` **staying in GPU memory**, so the existing NV12
zero-copy import consumes it with **no readback**. Output is still 8-bit. This is
per-backend (3 changes) and is what avoids the useless HW→CPU roundtrip.

### The down-convert is temporary and lossy — it MUST be logged, never silent

(Applies to both steps — see Logging below.)

### Logging (REQUIRED)

Each down-converted stream emits **one** `warn!` when the lane is chosen (once per
stream, NOT per frame), naming the codec + source format + that quality is
reduced. Example shape:

```
WARN wandr_video: h265 10-bit (I420_10LE) down-converted to 8-bit I420 —
     no 10-bit output path yet (task 117 M3b); display quality reduced
```

Rationale — the project's "no silent caps" rule: a bounded/lossy path must
announce itself. On a 10-bit/HDR panel it must be obvious from the log why the
picture isn't full quality, and the line must be greppable when M3b lands.

### Quality is a later milestone (M3b/M4), not a non-goal

Preserving 10-bit needs the whole chain ≥10-bit: a 10-bit texture import
(per-OS: D3D11 `R16`/`RG16`+ANGLE, dma-buf P010, IOSurface `x420`) **and** a
10-bit render target / swapchain (`RGB10A2`/`RGBA16F`) in the host compositor —
today that surface is 8-bit `RGBA8888` (to be confirmed). HDR adds an HDR
swapchain + HDR10 metadata + an SDR tone-map fallback. That's real future work
gated on real display capability; M3 just stops the freeze first.

## Non-goals / anti-pattern to avoid

The anti-pattern is a **silent, unconditional** `... ! videoconvert ! NV12 ! ...`
that hides an 8-bit downconvert on content the GPU could import natively. M3's
convert lane is the opposite: **gated** on runtime-detected 10-bit AND **logged**
(above), taken only because the 10-bit output path doesn't exist yet. The
negotiate-native / zero-copy path below stays the north star — M3 is the honest
interim toward it, not a permanent pin.

## Realistic format set (movie content only)

Consumer streams are effectively 4:2:0 at 8 or 10 bit. 4:2:2 / 4:4:4 do not
appear in streamed movies (pro/broadcast only) and are out of scope here.

| Subsampling | Depth | HW / zero-copy (native) | CPU / SW (planar) |
|---|---|---|---|
| 4:2:0 | 8  | **NV12** | **I420** (`yuv420p`) |
| 4:2:0 | 10 | **P010** | **I010** (`yuv420p10le`) |

Per-OS surface names for P010: D3D11 `DXGI_FORMAT_P010`, VA-API `P010`,
VideoToolbox `x420` (10-bit biplanar).

## Design — the M3b+ north star: negotiate, don't pin (deferred)

> The following is the quality-preserving target (M3b/M4), kept here as the
> direction M3's logged fallback is heading toward. M3 itself ships only the
> down-convert + warn above.

1. **Let the decoder advertise its native format.** Replace the fixed
   `format=NV12` in each lane's appsink caps with a caps *set* the lane can
   actually consume — `{ NV12, P010 }` for the HW/zero-copy lanes,
   `{ I420, I010 }` (via `videoconvert`) for the CPU lane — and read the
   negotiated `format` back from the sample caps (`VideoInfo::from_caps`, already
   done in `repack`/`gpu` paths) instead of assuming NV12.

2. **Import lane knows its format.** Extend the GL/D3D11/IOSurface import to
   handle P010 as well as NV12: same biplanar 4:2:0 shape, but 16-bit samples —
   luma as `R16`, chroma as `RG16` (Windows DXGI `P010`; GL `R16`/`RG16`), and a
   10-bit → normalized sample in the shader (values live in the high 10 bits). NV12
   stays the R8/RG88 path unchanged.

3. **Fallback lane for anything not importable.** If a decoder emits a format the
   importer doesn't (yet) support, fall back to the CPU lane with `videoconvert`
   producing `I420`/`I010` readback rather than failing. Never `not-negotiated`
   to a freeze — always have a decode-to-screen path, log when the slow lane is
   taken (per the "no silent caps" rule).

4. **No new hardcodes.** The chosen format comes from the negotiated caps at
   runtime; the only constants are the *supported-set* lists (one named place per
   lane), which is policy, not a magic value.

## Touch points (host-side only)

- `crates/wandr-video/src/backends/gstreamer.rs` — the four appsink caps strings
  (lines ~367–376), `encoded_caps`, the dma-buf `drm-format` parser (~line 282),
  and `repack`/`gpu` format read-back.
- The GL import (`wasm_android_host::video_gl` on Windows; the dma-buf and
  IOSurface import paths on Linux/macOS) — add the P010 (R16/RG16) binding + a
  10-bit shader branch.

## Acceptance

**M3.1 (software first):**
- Carry-On (HEVC Main 10) and Measure-for-Measure (AV1 10-bit) **play on screen**
  (SW decode, down-converted to 8-bit), smooth, on all three OSes — no
  `not-negotiated`, no freeze.
- Exactly **one `warn!` per down-converted stream** is logged, naming codec +
  source format (the message above), and it does NOT repeat per frame.
- 8-bit titles (Crime 101, Apex) unchanged — same zero-copy NV12 path, no
  regression, no extra copy, no warn.
- Host `gstreamer_decode` test extended with a 10-bit fixture: decodes through the
  convert lane to the correct dimensions.

**M3.2 (HW keep-zero-copy, B):**
- 10-bit content on the HW decoder plays with the GPU-side convert and **no CPU
  readback** (verify the zero-copy import path is still taken — e.g. GPU-memory
  buffer, not I420 readback, in the logs).
- Same one-warn-per-stream logging; 8-bit path still untouched.

Premise validated by the `gst-launch` reproduction above (no code needed).
`run-app-windows.bat wandr.jellyfin gstreamer-sw` is an optional end-to-end sanity
check on the real host.

**Deferred to M3b/M4 (quality):** true 10-bit preserved end-to-end (10-bit import
+ 10-bit render target), and HDR — see the north-star design below.

## Reference — the decoded-format landscape

Decoded video formats vary along three axes: **chroma subsampling** (how much
color is kept), **bit depth** (8/10/12), and **memory layout** (semi-planar =
GPU-style interleaved chroma, vs planar = CPU-style separate U/V planes).

### The ones that actually come out of movie decoders (H.264 / HEVC / AV1 / VP9)

| Subsampling | Bit depth | Semi-planar (GPU / HW) | Planar (CPU / SW) |
|---|---|---|---|
| **4:2:0** (99% of movies) | 8-bit  | **NV12**          | **I420** (`yuv420p`) |
| 4:2:0                     | 10-bit | **P010**          | **I010** (`yuv420p10le`) ← Carry-On/Measure |
| 4:2:0                     | 12-bit | P012 / P016       | `yuv420p12le` |
| 4:2:2 (broadcast/pro)     | 8-bit  | NV16, YUY2, UYVY  | I422 |
| 4:2:2                     | 10-bit | P210, Y210        | `yuv422p10le` |
| 4:4:4 (rare, screen/HQ)   | 8-bit  | NV24, AYUV        | I444 |
| 4:4:4                     | 10-bit | Y410              | `yuv444p10le` |
| monochrome 4:0:0          | 8/10   | —                 | GRAY8 / GRAY10 |

Same format, different name per GPU API:
- **D3D11/DXGI** (Windows): `NV12`, `P010`, `P016`, `YUY2`, `Y210`, `AYUV`, `Y410`
- **VA-API** (Linux): `NV12`, `P010`, `P012`, `YUY2`, `444P`
- **VideoToolbox** (macOS): `420v` (NV12), `x420` (10-bit biplanar ≈ P010)

### What this means for our negotiation path

You do **not** need the whole zoo. Consumer movie content is essentially:

- **8-bit 4:2:0** → NV12 (HW) / I420 (SW) — what we handle today
- **10-bit 4:2:0** → P010 (HW) / I010 (SW) — HDR & Main-10 content, what's failing now

4:2:2 and 4:4:4 basically never appear in streamed movies (pro/broadcast only).
So a correct, non-hardcoded host path realistically negotiates a **small set**:
`{NV12, P010}` on the zero-copy/HW lane and `{I420, I010}` on the CPU lane — and
picks whichever the decoder advertises, converting only when the importer can't
consume the native one.

That's the honest scope for the task-117 continuation: not "support every FOURCC,"
but **"stop pinning NV12 — accept the decoder's native 4:2:0 format at 8 and
10-bit, and import or convert accordingly."**

## Session findings — 2026-07-27 (M3.1 shipped; surface probe; transcode option)

### M3.1 shipped + verified
- Implemented in `crates/wandr-video/src/backends/gstreamer.rs` (probe_bit_depth +
  first-frame HW→SW rebuild), committed `2321eaa`, Windows host rebuilt.
- **Verified on Windows**: Carry-On (HEVC Main 10, 1920×960) plays — log shows
  `d3d11h265dec (HARDWARE)` → `WARN … 10-bit down-converted …` → `avdec_h265
  (software)` → frames. 8-bit path untouched.
- Detection is accurate: bit-depth read from `h265parse`/`av1parse` caps. "Almost
  every HEVC is 10-bit" is REAL (x265 movie rips default to Main 10, even SDR) —
  so M3.1 drops **most** of a library to SW.

### M3.1's ceiling — SW decode doesn't scale (measured)
- **Die My Love** (HEVC Main 10, **2876×2156 ≈ 6.2 MP**, no audio) does NOT play:
  `avdec_h265` software decodes it at **~13 fps** (measured: frame 60@12:15:04 →
  180@12:15:13 = 120 frames / 9 s) vs ~24 fps realtime. Frames decode but the
  guest is perpetually starved (`in-flight 0`) → no usable playback.
- Carry-On (1920×960, 4× fewer px) is fine on SW; Die My Love is not. **Software
  10-bit HEVC won't hit realtime at this resolution on this CPU** → the SW drop is
  a floor, not the answer. Confirms M3.2/10-bit (keep HW decode) is required.

### 10-bit GL surface probe (the deciding fact for pure-10-bit)
`eglinfo`, window-capable `R10 G10 B10 A2` configs:

| Target | 10-bit **window** surface |
|---|---|
| **Real Linux GPU (popos)** | ✅ **9 configs**, visual `AR30` (ARGB2101010) |
| WSLg (desktop dev backend) | ❌ pbuffer-only, no `win` (software/virtual path) |
| Windows/ANGLE, macOS | unprobed — DXGI/Metal support RGB10A2 → high confidence |

→ **Pure-10-bit is feasible on real hardware** (popos proves it). WSLg's "no" is a
dev-backend limitation. So an **8-bit fallback is mandatory** (WSLg + plain panels),
but on real GPUs we do 10-bit properly. **popos is where the Linux 10-bit path can
be built AND tested for real** (real VA HW decode + real 10-bit window surface);
WSLg cannot.

### Skia / compositor state
- Skia **can** render 10-bit (`ColorType::RGBA1010102` / `F16`) — not the blocker.
- Compositor is currently hardwired **8-bit**: `canvas_impl.rs:623/631`
  (`Format::RGBA8` / `RGBA8888`), glutin config `with_alpha_size(8)` (`:668-669`).
- Video composites **through Skia**: `video_desktop.rs:203-246`
  `composite_video_surfaces(canvas)` → `canvas.draw_image_rect`. So flipping to
  10-bit means the ONE shared surface all apps render into (small code: glutin
  config → RGB10_A2 + Skia ColorType → RGBA1010102 + per-OS 10-bit import).

### Memory per frame (deterministic)
| Format | B/px | Die My Love 2876×2156 | Carry-On 1920×960 |
|---|---|---|---|
| NV12 / I420 | 1.5 | 9.3 MB | 2.8 MB |
| P010 (10-bit YUV) | 3 | 18.6 MB | 5.5 MB |
| RGBA8 (present today) | 4 | 24.8 MB | 7.4 MB |
| **RGB10A2 (10-bit target)** | 4 | **24.8 MB (= RGBA8!)** | 7.4 MB |
- 10-bit render target costs **zero** extra memory (RGB10A2 = RGBA8). P010 (18.6)
  is *less* than the RGBA8 we buffer today — pure-10-bit zero-copy is not more mem.

### The transcode escape hatch (browser model)
- Chrome (esp. Linux) has **no HEVC decode** — the web UI plays Die My Love only
  because **Jellyfin transcodes it server-side to H.264 8-bit (downscaled)**. The
  browser gets pre-chewed video; the *server* does the hard work.
- wandr.jellyfin does **DirectPlay** → it decodes the original HEVC 10-bit itself,
  which is why it hits the wall the browser never sees.
- **We have the same escape hatch, unused**: narrow the DeviceProfile so the
  server transcodes the cases we can't DirectPlay well (10-bit / oversized / no-HW
  codecs). Trade-off: server CPU + quality (downscale) vs full-quality client HW
  decode. Two legitimate roads — DirectPlay+fix-decode, or transcode-fallback.

### Revised plan
- **8-bit fallback (M3.2: HW decode + GPU P010→NV12 convert) is foundational** —
  mandatory where 10-bit surfaces aren't grantable (WSLg, 8-bit panels), and it
  fixes Die My Love on every target by keeping HW decode.
- **Pure-10-bit is the enhancement branch** of the SAME per-OS HW path (import
  P010 as AR30 where the window config is granted; convert→NV12 where not) —
  "negotiate the surface", buildable/testable on popos.
- **AV1 is a SEPARATE issue** (no HW AV1 on UHD 620 → always dav1d SW; a
  frame-delay / streaming-emit problem, not the 10-bit output pin) — under analysis.

## Cross-refs

- `[[reference_gstreamer_desktop_backend_spike]]` — the GStreamer consolidation;
  notes `not-negotiated ≠ HW-limit`.
- `[[reference_vaapi_zerocopy_real_players]]` / `[[reference_dxva_h264_windows_decode]]`
  — the NV12 two-texture import this milestone generalises to P010.
