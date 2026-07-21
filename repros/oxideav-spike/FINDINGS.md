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

## Follow-up (2026-07-21): a THIRD machine, and the HW bug is systemic

Ran the same probe on **fedora** (Fedora 43, Intel HD 4000 Ivy Bridge, real
`/dev/dri/renderD128`, i965+iHD drivers, world-accessible render node, no NVIDIA).

`oxideav list` registered `vaapi-h264` at priority 10 — and that registration is
**proof the hardware VAAPI H.264 decode path is real**: `oxideav-vaapi/src/lib.rs:101`
gates registration on `host_supports_codec_decode("h264")`, a real
`vaQueryConfigEntrypoints` for `VAEntrypointVLD`. On WSL that returned false and it
*skipped*; on fedora it returned true and *registered*. So libva initialized, the
driver advertises an H.264 VLD **decode** profile, and `vaGetConfigAttributes`
succeeded — the capability is there.

But the decode itself: **auto (vaapi-h264 wins) = 0/300; `--no-hwaccel` = 300/300.**
Identical to nvdec on popos and vulkan on WSL. So the silent-HW-failure is
**systemic across all three of oxideav's HW backends** (NVDEC, Vulkan, VAAPI), not
one bad path — a registry that never falls back when a HW decoder yields zero
frames.

**vainfo (2026-07-21, user installed it + joined `render`) — direct confirmation:**
```
Driver: Intel i965 driver for Ivybridge Mobile 2.4.0.pre1
  VAProfileH264ConstrainedBaseline : VAEntrypointVLD
  VAProfileH264Main                : VAEntrypointVLD
  VAProfileH264High                : VAEntrypointVLD   <- bbb is High
  VAProfileH264StereoHigh          : VAEntrypointVLD
```
No HEVC/VP9/AV1 VLD → HD 4000 is **H.264-decode-only** (Ivy Bridge predates HW
HEVC/VP9). The default `iHD` (nonfree) driver fails to init on this GPU (it is
Broadwell+); libva falls back to `i965`, the correct driver. Our backend should
prefer `LIBVA_DRIVER_NAME=i965` on Ivy Bridge / let libva auto-fall-back.

**Ruled out driver selection (2026-07-21).** fedora's default libva tries the
`iHD` nonfree driver first, which fails to init on Ivy Bridge, then falls back to
`i965`. So the earlier 0/300 could have been a wrong-driver artifact. It was not:
forcing `LIBVA_DRIVER_NAME=i965` — with libva confirming `va_openDriver() returns
0` (i965 loads) and vainfo confirming H.264 VLD profiles — still gives **0/300**.
The `--debug` output shows oxideav-vaapi trying to open a nonexistent
`hybrid_drv_video.so` and never creating a surface/context. So oxideav's VAAPI
decode is broken in its own data path, independent of driver — the same
silent-0-frame as nvdec and vulkan, now confirmed against a driver we KNOW works.

**Two conclusions for wandr's VAAPI plan:**
1. The hardware + i965/iHD driver + render-node access on fedora are ready; VAAPI
   H.264 decode is available on this GPU. fedora is the target box (popos was
   down; its NVIDIA driver was dead anyway).
2. oxideav cannot be leaned on for HW decode yet, so **we must write our own VAAPI
   backend** — and it must key fallback on frames produced, exactly the contract
   the wandr-video registry already enforces. This probe de-risked it: only a
   correct decode data-path remains.

## The README's own HW table confirms it (found 2026-07-21)

oxideav's README has a hardware-backend table (behind a collapsed "Hardware
acceleration" section). **Every HW backend is marked 🚧 in-progress, none ✅**;
encode is mostly "— stub" / "— empty":

| Module | Decode | Encode |
|---|---|---|
| oxideav-videotoolbox (macOS) | 🚧 H.264+HEVC+ProRes+… | 🚧 |
| oxideav-vaapi (Linux) | 🚧 H.264 | — stub |
| oxideav-vdpau (NVIDIA legacy) | 🚧 H.264+HEVC+VP9+MPEG2 | — stub |
| oxideav-nvidia (NVENC/NVDEC) | 🚧 VP9+AV1+MPEG2 | — |
| oxideav-vulkan-video | 🚧 H.264+HEVC+AV1 *capability queries* | — empty |

The vaapi/vulkan notes describe **capability probing** ("EntrypointMatrix …
capability probe", "capability queries") — NOT a working decode loop. So the 🚧
is honest: the piece that's done is the probe (which is exactly what registered
`vaapi-h264` and passed the registration check), while the decode data-path is
unfinished. That fully explains the 0/300 on all three machines: an incomplete HW
backend registers at priority 10, wins dispatch, and can't decode — with no
fallback.

So this is less "a bug in finished code" and more "unfinished HW that registers
and wins dispatch anyway". Either way, for wandr: **do not wait for oxideav HW —
build our own VAAPI backend** (hardware + i965 driver + H.264 VLD are confirmed
working). The value from oxideav is its mature SOFTWARE decoders, next section.

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
