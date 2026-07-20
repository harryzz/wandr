# oxideav spike — findings (2026-07-20)

Goal: evaluate `oxideav` as a **ready component** for wandr's media playback
(task 117 M2), the way we already consume skia / wasmtime / symphonia — not to
build codecs ourselves. Two machines, two GPU situations.

## TL;DR

The pure-Rust decoders are real and work (H.265, AV1 decoded 300/300 frames from
actual Big Buck Bunny clips). But the **integration layer around them is early**,
and one bug is a showstopper for our use case: on both machines, H.264 with
hardware enabled decodes **0 frames and reports success** — a silent failure that
`--no-hwaccel` (software) fixes at 300/300. That is precisely the failure mode
the priority-registry + auto-fallback design exists to prevent, and it does not
prevent it.

**Verdict: do not adopt yet; re-run this spike per release.** The design is right
and the trajectory is steep (daily commits). When the fallback actually falls
back, it becomes a serious option for the desktop HW lane.

## Setup

- `fetch-samples.sh` — 4× Big Buck Bunny (CC-BY), one per codec, ~1 MB each.
- `run-spike.sh` — decode each sample twice (auto vs `--no-hwaccel`), report
  `frames decoded / packets in` for each. A row where auto < sw is a silent bug.
- Built the `oxideav` CLI from git HEAD (135 sub-crates via
  `scripts/update-crates.sh`; the published crates.io aggregator is v0.0.3 from
  May, months behind). Binary needs ≤ GLIBC_2.35, so it ran on Pop!_OS
  (glibc 2.39) with no rebuild.

## Results

### This box — WSL2, Intel UHD 620, no GPU reachable

Backends registered: `h264_vulkan` (HW, pri 20) + `h264_sw` (pri 100). VAAPI
loaded but "no H.264 decode profile advertises VLD" (WSL has no `/dev/dri`), NVIDIA
absent.

| codec | auto | --no-hwaccel | verdict |
|---|---|---|---|
| av1 | 300/300 | 300/300 | ok (pure-Rust; no HW backend) |
| h264 | **0/300** | 300/300 | **AUTO DECODED NOTHING** |
| h265 | 300/300 | 300/300 | ok |
| vp9 | ERR | ERR | **no impl registered** |

### popos — Pop!_OS 24.04, real `/dev/dri`, Intel HD 4000 + NVIDIA GT 650M

Backends registered (note the rich list a real GPU brings): H.264 → `h264_nvdec`
(pri 5) + `h264_nvenc` + `vaapi-h264` (pri 10) + `h264_vulkan` (pri 20) +
`h264_sw` (pri 100); HEVC/VP9 → nvdec. **Same result:**

| codec | auto | --no-hwaccel | verdict |
|---|---|---|---|
| av1 | 300/300 | 300/300 | ok |
| h264 | **0/300** | 300/300 | **AUTO DECODED NOTHING** |
| h265 | 300/300 | 300/300 | ok |
| vp9 | ERR | ERR | **no impl registered** |

The nvidia kernel driver was not actually up (`nvidia-smi` failed), so `h264_nvdec`
at priority 5 wins dispatch, produces nothing, and never falls through to the four
lower-priority backends behind it — including `h264_sw`, which works.

## Three concrete bugs (upstream-reportable)

1. **H.264 HW decode = silent 0-frame success, no fallback.** Reproduced on two
   machines and across NVDEC/VAAPI/Vulkan. `--no-hwaccel` → 300/300. This is the
   dealbreaker: a "HW-first with automatic fallback" registry that does not fall
   back on a HW decode that yields zero frames.
   - NOT an avcC/hvcC length-prefix issue: H.265 from the same MP4 muxing decodes
     300/300. It is H.264-dispatch-specific.

2. **VP9 registers nothing.** `oxideav-vp9` is v0.0.12 with a real decoder, is in
   `oxideav-meta`'s `video` feature and its `build.rs` list — but
   `crates/oxideav-vp9/src/lib.rs:867` is `pub fn register(_ctx) {}` (empty body).
   So `oxideav info vp9` → "no implementations registered".

3. **H.265 mis-typed as Audio.** `oxideav info h265` reports `media_type: Audio`
   (the encoder sets Video correctly at `oxideav-h265/src/encoder.rs:188`, so the
   decoder registration is wrong). Decode still works; the metadata lies.

Also: the `y4m` muxer needs a `pixel_format` the decode path doesn't supply, so
raw-video output fails — a fourth integration seam, minor.

## What this tells wandr M2

- **The "evaluate a ready component" experiment worked** — a day of spike, no
  wiring into wandr-host, and we have a clear go/no-go with reproducible evidence.
- **libvpx stays.** Our shipped VP9 path decodes; oxideav's VP9 does not even
  register. This is the concrete version of "more risk, no requirement".
- **The silent-HW-failure bug is the single most important thing to watch.** It is
  the exact risk in our own step-3 HW-first plan. Whatever we adopt or build, the
  fallback must be driven by *frames actually produced*, not by the absence of an
  error return.
- **Re-run `run-spike.sh` per oxideav release.** When bugs 1–3 clear, re-evaluate
  for the desktop HW lane. Near-zero-risk interim use unchanged: `oxideav-vp9`
  (once it registers) as a cross-check oracle against libvpx in tests.

## To reproduce elsewhere

```
./fetch-samples.sh
# build the CLI once (see README) OR copy the prebuilt binary if glibc >= 2.35
OXIDEAV_CLI=/path/to/oxideav ./run-spike.sh
```
