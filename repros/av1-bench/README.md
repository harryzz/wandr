# av1-bench (task 117 M2)

Investigates the AV1 software decoder for playback: benchmark oxideav-av1
(pure-Rust) vs dav1d (mature C) vs the option of rav1d (pure-Rust dav1d port).
Mirrors libde265-bench for HEVC.

## Result (bbb-av1.webm, 1280x720, i7-8565U)

| Decoder | fps | notes |
|---|---|---|
| oxideav-av1 (pure-Rust) | **FAILED** | `UnexpectedEnd` on standard matroska AV1 OBU framing — needs oxideav's own mkv demuxer's reframing (the spike's CLI decoded it 300/300 that way). Out-of-box integration does not work. |
| **dav1d** (C) | **~355 fps** | 12× real-time, works out-of-box with matroska frames, static-linkable via meson. |

**Decision: dav1d for AV1.** It works, it is fast, and — unlike HEVC's LGPL
libde265 — **dav1d is BSD-2**, so AV1 sits in task 117's *permissive-static* tier
(second choice), not the LGPL tier. oxideav-av1 is not viable here without
adopting oxideav's demuxer, and would likely be slow anyway (cf. oxideav-h265 at
~4 fps). rav1d (pure-Rust dav1d port, BSD-2) is the future pure-Rust option once
it matures.

## Run

```
# libdav1d present (ships with libheif on Debian) OR built internally:
SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=always \
cargo run --release   # clones + meson-builds dav1d static (needs meson/ninja/nasm/git)
```
