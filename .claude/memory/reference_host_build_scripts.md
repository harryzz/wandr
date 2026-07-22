---
name: reference_host_build_scripts
description: "Canonical wandr-host build scripts (linux/windows/android/macos) — use these, never inline `cargo build`. p3-async is ON by default (the recurring wasip3 footgun)."
metadata:
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
  modified: 2026-07-22T08:21:39.145Z
---

**Build wandr-host ONLY through these committed scripts** (`tools/scripts/`) —
never hand-roll `cargo build`. Each bakes in the right target + flags. Written
2026-07-10 after repeatedly clobbering the p3 binary with plain builds.

| Script | Target | Notes |
|---|---|---|
| `build-host-linux.sh`   | `x86_64-unknown-linux-gnu` release | desktop dev/prod backend |
| `build-host-windows.bat`| MSVC x64 release | needs `VCVARS` + `FFMPEG_DIR` + `LIBCLANG_PATH` (env-overridable) |
| `build-host-android.sh` | `aarch64-linux-android` release | builds host **+** arbiter; sources `env-android.sh` |
| `build-host-macos.sh`   | `x86_64`/`aarch64-apple-darwin` release + universal | `ARCHS`/`UNIVERSAL` knobs; macOS only |

**p3-async is ON by default on all four** (`--features p3-async`); `P3=0` opts to
the p2-only flavor. THE FOOTGUN: default and p3-async build to the SAME binary
path, so a plain `cargo build` silently clobbers the p3 one → the guest panics at
instantiate with `component imports instance wasi:sockets/types@0.3.0 … resource
implementation is missing`. Current guests that NEED p3: Signal AND
audio.player (audio.player fetches album metadata over wasi:sockets/tls@0.3 to
musicbrainz.org). See [[project_task115_wasip3_async]].

**The stale-binary trap (task 117 M2, 2026-07-21):** the scripts build to the
EXPLICIT-target path — linux = `runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host`
— but an OLD host-triple build may still sit at `runtime/wandr-host/target/release/wasm-android-host`
and it is the one you reach for by habit. Running it fails at instantiate with
`resource implementation is missing` for a resource the host DOES implement,
which reads exactly like the stale-zygote error ([[reference_missing_instance_error_stale_zygote]])
and sends you diagnosing the wrong thing. Check the binary's mtime against the
source you just changed BEFORE believing any "missing implementation" error.

**Clean-machine Linux deps are WIDER than CI's list (2026-07-22, popos):** CI's
apt line is not a complete set — GitHub's ubuntu runners preinstall things a real
machine does not, so `build-host-linux.sh` fails on a fresh box with packages CI
never names. Found so far, beyond the CI list: **`libssl-dev`**, pulled in by
`openssl-sys` <- `curl` <- **build-dependency of `libde265-sys`**, whose build
script DOWNLOADS the libde265 source. So the host build needs OpenSSL headers and
NETWORK ACCESS purely to fetch a tarball for the LGPL H.265 decoder — nothing in
wandr uses OpenSSL at runtime (TLS is rustls, see [[reference_wandr_wasi_tls_transport]]).
Drop the `libde265` feature and the requirement goes with it. `libva-dev` is also
needed for the `vaapi` feature (task 117 stage 3). NOT needed despite being absent:
libudev-dev, libdbus-1-dev — no matching `-sys` crate is in the graph. Diagnose
this class with `cargo tree -i <sys-crate>`; it names the real culprit in one shot.

**Per-target gotchas (each cost real time this session):**
- **Android: run from the crate dir**, not the repo root with `--manifest-path`.
  Cargo reads `.cargo/config.toml` relative to CWD, and
  `runtime/wandr-host/.cargo/config.toml` carries the NDK linker + libc++ search
  paths — miss it and the link fails with `unable to find library -lc++_static /
  -lc++abi`. p3-async flips `wasm_component_model_async(true)`, which changes the
  AOT precompile config hash → device `.cwasm` re-precompile (task-115 M4). If the
  device is ALREADY p3, deploying a p3 host is same-hash = no re-precompile.
- **Windows: run the exe with `%FFMPEG_DIR%\bin` on PATH** — avcodec-*.dll are
  load-time deps; a detached launch without that PATH fails to start. Skia
  from_encoded has NO codecs in the Windows prebuilt (image decode falls back to
  the `image` crate; see [[feedback_no_hardcoding]] region / wasi_canvas decode).
- **macOS: an Intel Mac can cross-build the arm64 slice** — Apple's toolchain is
  universal (no extra linker/sysroot). Monterey **12.7.6 → Xcode 14.2**, which
  HAS the Apple-Silicon SDK (arm64 SDK is Xcode 12+/macOS 11+), so it is NOT
  x86-only. BUT cross-building proves COMPILE + LINK only, NOT runtime: arm64's
  weak memory model (vs x86 TSO) against our `unsafe impl Send` codec/audio code,
  the Skia C++ shim / ffmpeg ABI, and CoreAudio/AVFoundation/Metal only exercise
  when RUN. Verify the arm64 binary on a real Apple Silicon Mac (or a `macos-14`
  CI runner) — same rule as: we cross-build aarch64-linux-android on x86 Linux but
  only TRUST it after running on the Pixel.

**General principle:** a cross-build passing ≠ it works on the target arch — it
only proves the compiler/linker accept it. Runtime correctness needs a run ON the
target. Prereqs: rust arm64 host toolchain; system ffmpeg (`brew install ffmpeg
pkg-config` on mac; the BtbN shared prebuilt on Windows); skia-safe fetches its
own per-target prebuilts.
