---
name: reference_windows_arm64_buildable
description: "Windows ARM64 (Snapdragon X Elite / aarch64-pc-windows-msvc) IS buildable — the heavy prebuilts (skia, ffmpeg) exist for it; build natively on the device via build-host-windows.bat + winarm64 FFMPEG_DIR."
metadata:
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

wandr-host builds for **Windows on ARM64** (`aarch64-pc-windows-msvc`, e.g. a
Qualcomm Snapdragon X Elite laptop). Verified 2026-07-12 that the two would-be
blockers have arm64-windows prebuilts, so no source builds:

- **skia-safe / skia-binaries** ships `aarch64-pc-windows-msvc` binaries with
  `gl` + `textlayout` (exactly our features) → NO Skia-from-source build.
- **BtbN ffmpeg** has `winarm64` shared builds
  (`ffmpeg-n8.1-latest-winarm64-gpl-shared`) → matches the `ffmpeg-next 8.1`
  crate; the DLLs are `avcodec-62.dll` etc. (arm64).

Everything else is native-Windows or portable: Rust has the
`aarch64-pc-windows-msvc` target AND runs natively on the device; wasmtime has an
aarch64 cranelift backend; cpal (WASAPI) + nokhwa (MediaFoundation) are native
Windows APIs; the C++ Skia shim compiles with MSVC's ARM64 compiler.

**Build (native on the device = runnable + testable; preferred over cross):**
prereqs = Rust (`rustup`, native arm64), VS 2022 **ARM64 MSVC** component + C++
Clang tools (for ffmpeg-sys-next bindgen), and the ffmpeg **winarm64** prebuilt.
Then the committed `tools/scripts/build-host-windows.bat` works as-is —
`cargo build --release` on the native arm64 host builds
`aarch64-pc-windows-msvc` automatically; only override `FFMPEG_DIR` to the
winarm64 path. Launch via `run-app-windows.bat` with the winarm64 `bin` on PATH.
Cross-compiling from x86 Windows (`rustup target add aarch64-pc-windows-msvc` +
ARM64 MSVC cross tools) also works but only proves compile, not runtime — same
caveat as the macOS arm64 slice. See [[reference_host_build_scripts]].
