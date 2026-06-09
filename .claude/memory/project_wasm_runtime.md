---
name: WASM Android Runtime project status
description: Goals, tech stack, and current milestone status for the wandr project. Full Compose-on-WASM PoC is working end-to-end on device.
type: project
originSessionId: ca7f3a70-2c6e-4c65-baae-454dc44933b5
---
Replacing Android's ART runtime with a wasmtime host that runs Kotlin/Compose WASM components, rendered via skia-safe on real GPU hardware.

**Target device:** Pixel 2 XL (taimen), LineageOS / Android 15 (API 35), aarch64.

**Status as of 2026-05-15: WORKING END-TO-END POC.**

What's verified on device:
- Kotlin/Compose application compiled to WASM Preview-2 Component, AOT-compiled for aarch64-linux-android, loaded by wasmtime via `Component::deserialize_file`
- 32 in-tree wasm-wasi klibs for compose-multiplatform-core (+ 11 sibling fat klibs that bundle the same sources for fast linking — 5 min vs 2 hr cold link due to O(N³) whole-world IR lowering on klib count)
- BasicTextField (TextFieldState API) + hardware keyboard via `on-key-event-v2`
- In-canvas soft keyboard (WasiSoftKeyboard, Paragraph-driven cursor positioning)
- Material3 widgets — Buttons, Checkbox, DropdownMenu (with workaround), Switch, Slider
- LazyColumn with scrolling
- Continuous animations (with a known wasm-linear-memory leak in indeterminate ProgressIndicator — see [[feedback_indeterminate_progress_leak]])
- Warm-resume across lifecycle events: store persists across activity pause/resume, renderer swapped, caches inherited
- Host-side WasiDrawable transforms (translation/scale/rotation/clip/alpha/shadow) for Compose layer model

**Key paths (post-rename, 2026-05-15):**
- Host: `~/wandr/wandr-host/` (Rust cdylib + APK, was `~/wandr/host/`)
- App: `~/wandr/wandr-app/` (Kotlin/Compose guest, was `~/wandr/skiko/test-app/`)
- Skiko fork: `~/wandr/skiko/skiko/` (KMP `wasmWasi` target → `skiko-wasm-wasi.klib`)
- Compose port: `~/wandr/compose-multiplatform-core/` (32 wasi klibs in mavenLocal)
- Compose siblings: `~/wandr/compose-{runtime,ui-base,ui-graphics,ui-text,ui,foundation-layout,foundation,animation-core,animation,material-ripple,material3}-wasi/` (11 fat klibs in mavenLocal)
- WIT source-of-truth: `~/wandr/wit/skiko-gfx.wit` (mirrored to `~/wandr/skiko/skiko/wit/skiko-gfx.wit`)
- NDK: `~/android-ndk-r27d/`
- SDK: `~/android-sdk/`
- Checkpoint: `~/wandr/.task-state`

**Reproduce-the-PoC docs (all in repo, all use `~` paths, all cross-link cleanly):**
- `~/wandr/wandr-app/README.md` + `BUILD.md`
- `~/wandr/wandr-host/README.md` + `BUILD.md`
- `~/wandr/skiko/README-wasmWasi.md` + `BUILD-wasmWasi.md`
- `~/wandr/compose-multiplatform-core/README-wasmWasi.md` + `BUILD-wasmWasi.md`
- `~/wandr/CLAUDE.md` (overall guide, references the others)

**Build cycle (host APK):**
```
cd ~/wandr/wandr-host
NDK=~/android-ndk-r27d/toolchains/llvm/prebuilt/linux-x86_64
ANDROID_HOME=~/android-sdk ANDROID_NDK_HOME=~/android-ndk-r27d \
CC_aarch64_linux_android=$NDK/bin/aarch64-linux-android29-clang \
CXX_aarch64_linux_android=$NDK/bin/aarch64-linux-android29-clang++ \
AR_aarch64_linux_android=$NDK/bin/llvm-ar \
PATH="$NDK/bin:$PATH" CARGO_BUILD_JOBS=2 \
cargo apk build --release
adb install -r target/release/apk/wasm_android_host.apk
```
After cold build (~11 min for skia-bindings), incremental APK rebuild <1 min.

**Build cycle (guest cwasm):** See `~/wandr/wandr-app/BUILD.md` — Kotlin → 11 MB .wasm → wasm-tools embed/new → wasmtime AOT → 63 MB .cwasm → adb push to `/sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm` → app force-stop + start.

**Linker version note:** `.cargo/config.toml` linker API level must match `Cargo.toml`'s `min_sdk_version` (currently both = 29). Mismatch → cargo invalidates cache, full rebuild every switch.

**Activity:** `com.example.wasmruntime/android.app.NativeActivity` (not `MainActivity`).

**Checkpoint `.task-state` tracks progress across sessions** — always read it first when resuming work.

**Why this matters:** The skiko PoC was proven months ago (task 01–07 in CLAUDE.md). What's new as of 2026-05-15 is that the full compose-multiplatform-core port is in tree, published, consumable, and proven to run real Compose UIs on this stack. The remaining roadmap (paint completeness, gradients, image-rect, color-filter — tasks 08–13 in CLAUDE.md) is incremental WIT-surface expansion, not architectural risk.
