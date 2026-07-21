# libde265-bench (task 117 M2)

Answers: is the ~6 fps of our pure-Rust HEVC decoder "software HEVC is just slow
at 720p" or "this decoder is immature"? Decodes the same real HEVC file through
both `oxideav-h265` (pure-Rust MIT) and `libde265` (mature C, LGPL) in tight loops.

## Result (bbb-h265, 1280x720, i7-8565U)

| Decoder | Threads | fps |
|---|---|---|
| oxideav-h265 (Rust) | 1 | 4.3 |
| libde265 (C) | 1 | **59.5** |
| libde265 (C) | 8 | **107.1** |

**Software HEVC IS real-time-capable — libde265 does 2× real-time single-threaded.
oxideav-h265 is 13.7× slower**, i.e. correct (300/300, pixel-verified elsewhere)
but not yet real-time. So for real-time software HEVC without HW, libde265 (LGPL —
a preference, not a wall) is the pragmatic choice; oxideav-h265 is the pure-Rust
correctness fallback until it matures. HW (VAAPI/MediaCodec) remains ideal.

## Run

```
# libde265.so.0 must be present (Debian: it ships with libheif; or apt install
# libde265-dev). The linker needs a `libde265.so` name:
mkdir -p ~/.local/de265lib && ln -sf $(ls /usr/lib/*/libde265.so.0) ~/.local/de265lib/libde265.so
RUSTFLAGS="-L $HOME/.local/de265lib" \
VPX_LIB_DIR=… cargo run --release   # (same VPX_* env as the host build)
```
