---
name: WASM Android Runtime project status
description: Goals, tech stack, and current milestone status for the wasm-android-runtime project
type: project
originSessionId: ca7f3a70-2c6e-4c65-baae-454dc44933b5
---
Replacing Android's ART runtime with a wasmtime host that runs Kotlin/Compose WASM components, rendered via skia-safe on real GPU hardware.

**Target device:** Pixel 2 XL (taimen), LineageOS 22.2, aarch64.

**Key paths:**
- Host: `/home/harry/wasm-android-runtime/host/` (Rust cdylib)
- NDK: `/home/harry/android-ndk-r27d/`
- Checkpoint: `/home/harry/wasm-android-runtime/.task-state`
- Deploy: push to /sdcard, then `su -c cp` into `/data/app/~~GasO63VDwIBeGawH4BpECw==/com.example.wasmruntime-PrejjKVN_F9dopj0I3cyMg==/lib/arm64/`

**Build command:**
```
export PATH="/home/harry/android-ndk-r27d/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH" ANDROID_NDK_HOME="/home/harry/android-ndk-r27d"
cargo build --release --target aarch64-linux-android
```

**Build for APK (required for device):**
```
export PATH="/home/harry/android-ndk-r27d/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export ANDROID_HOME=/home/harry/android-sdk  # fake SDK stub with symlinks to /usr/bin tools
export ANDROID_NDK_HOME=/home/harry/android-ndk-r27d
cargo apk build --release  # panics at end (bin vs cdylib mismatch) but APK is valid
adb install -r target/release/apk/wasm_android_host.apk
```

**Status:** Task 04 complete — Kotlin/Compose WASM rendering at 60fps on Android. Root cause of store poisoning was `on_resize` WASM trap in the `Resized` event handler (silently discarded with `let _`). Fixed by removing `dispatch_resize` call — component gets size via `canvas::surface_size()` instead. Full render pipeline confirmed: clear + drawRect + drawOval + drawLine all fire each frame.

**WIT note:** Component WIT uses `surface-size: func() -> tuple<u32, u32>`. The Kotlin core WASM imports it as `surface-width`/`surface-height` separately; the component adapter splits the tuple at the boundary.

**Why:** Checkpoint `.task-state` tracks progress across sessions. Always read it first.
