---
name: reference_libde265_windows_win32cond_crash
description: libde265 SW H.265 intermittently 0xC0000005-crashes on Windows (racy win32cond worker-thread emulation); fix = single-thread on Windows
metadata: 
  node_type: memory
  type: reference
  originSessionId: 215f1733-fbc2-4004-aac8-cacd9719553d
  modified: 2026-07-25T09:07:03.655Z
---

**Symptom:** SW H.265 (libde265) intermittently crashes with **0xC0000005**
(STATUS_ACCESS_VIOLATION, exit `-1073741819`) on Windows — reliably in the
video player, only the SW-H.265 combo. HW H.265 (d3d11), SW H.264 (openh264),
HW H.264 all fine (none touch libde265).

**Root cause (source-confirmed, libde265 1.0.15 `threads.h` + `extra/win32cond.c`):**
On `_WIN32` libde265 uses `de265_thread=HANDLE`, `de265_mutex=HANDLE`,
`de265_cond=win32_cond_t` — a **hand-rolled condition-variable EMULATION** (the
Schmidt `SignalObjectAndWait` pattern), NOT native `CONDITION_VARIABLE`. libde265
spins 4 decode **worker threads** synchronizing through it; under contention it
races and faults inside the pool. It's a **threading race** → load-dependent and
intermittent: fires in the busy player process (pool contends with GL/audio
threads) but NOT in quiet headless `--video-decode-file` (50+ sequential and 30
concurrent runs never tripped it — do not trust headless as the repro; the
**player** is the reliable repro).

**Fix (wandr-host `2ec5213`, supersedes the wrong `f25a660`):** in
`crates/wandr-video/src/backends/libde265.rs` `H265Decoder::new`, pin the pool to
**exactly 1 worker on Windows** — `let n = if cfg!(target_os="windows") { 1 } else
{ available_parallelism…min(4) }`. BOTH bounds matter, learned the hard way:
- **0 workers CORRUPTS** (first attempt `f25a660`, shipped + user-rejected): libde265
  finishes a frame's chroma/deblocking ON the pool; no pool → partial garbage
  (green, striped). The decode-file "ok" RESULT only checks timing/order, NOT
  pixels — inspect the dumped PNG.
- **≥2 workers CRASHES** (the win32cond multi-waiter race above).
- **1 worker = correct AND safe**: async decode is bit-identical (md5) to the
  4-thread output (verified on bbb-h265 1080p), and per libde265 `threads.cc`
  workers park only on `pool->cond_var` / main only on a separate `cond`, so one
  worker ⇒ ≤1 waiter per cond ⇒ the racy broadcast handshake is never entered.

**STATE (postponed 2026-07-25):** shipped fix is n=1 (`2ec5213`), but **n=1 is too
SLOW for smooth 1080p playback** (~9 fps; player in-flight buffer drains 17→1 →
choppy → "not usable"). ALSO: the CI/windows-latest artifact **miscompiles
libde265** (corrupt output) independent of the code — proven by building the EXACT
CI codec set locally (nasm+meson+vcpkg-libvpx installed at `C:\Users\harry\vcpkg`)
which decodes CORRECTLY, so it's the CI toolchain (cl.exe/bindgen), not a codec
symbol collision. **Local VS2022 full build is correct + fast enough is UNKNOWN at
n≥2.** NEXT STEP when resumed: the only fast+correct+safe fix is to replace
libde265's racy `win32cond` emulation with native `CONDITION_VARIABLE` + `SRWLOCK`
(patch `threads.h`/`threads.cc` WIN32 branch in `vendor/libde265-sys/build.rs`,
alongside FIX 1-5) then use the multi-thread pool on Windows. Local build tree
`C:\Users\harry\wandr-host-build` has the full toolchain wired
(`build-libde265-repro.bat`). Separately: still owe a CI-toolchain fix (pin MSVC/
bindgen) so the shipped artifact isn't corrupt.

**Verifying pixels:** `--video-decode-file` dumps `/tmp/decode-file.png` (Windows:
`C:\tmp\`). GOTCHA: it only rewrites on a successful decode→snapshot; a failed/empty
run leaves the STALE file, so `rm` it first and check md5/mtime, else you compare
old pixels (this bit me — H264/oxideav/de265 all showed one identical stale frame).
Correct bbb-h265 last frame md5 = `dbf9ea6bcdebadc55c94138fe938784e` (the tree scene).

**Diagnosis loop that worked:** built a MINIMAL Windows host locally with just
MSVC (`features=["libde265","oxideav-h265"]` + host `d3d11` — no libvpx/dav1d/
openh264, so no vcpkg/nasm/meson needed), forced the backend with
`WANDR_VIDEO_BACKEND=libde265` / `WANDR_VIDEO_NO_HW=1`, read the downloaded
libde265 source under `target/release/build/libde265-sys-*/out/`.

Notes: the desktop feature set (`runtime/wandr-host/Cargo.toml` ~line 157) ships
`libde265` but NOT `oxideav-h265`, so Windows SW H.265 has no pure-Rust fallback
— libde265 is the only SW H.265 path there. See [[reference_libvpx_wandr_video]],
[[reference_media_codec_strategy]], [[reference_dxva_h264_windows_decode]].
