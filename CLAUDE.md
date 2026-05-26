# WASM Android Runtime — Claude Code Master Guide

## What this project is

Replace Android's ART runtime with a wasmtime-based host that:
- Runs Kotlin/Compose apps compiled to WASM components
- Renders via skia-safe (Skia C++ library) on real GPU hardware
- Targets aarch64 Android devices (root via ADB, no system modification needed)
- Uses the WASM Component Model + WIT for all host/guest interfaces

**The full Compose-on-WASM PoC is working end-to-end on device as of 2026-05-15.**
Real Compose Multiplatform applications — TextField, Material3 widgets,
LazyColumn, scrolling, soft keyboard, lifecycle resume — render on the
Pixel 2 XL (Android 15 / API 35) at ~10–20 ms/frame. Tasks 01–14 are all
complete. Remaining work is polish (IME, Skottie, memory leak in
indeterminate animations) — see `## Current status` below.

---

## FIRST THING: check the checkpoint file

Before doing ANYTHING else, run:

```bash
cat .task-state 2>/dev/null || echo "NO_STATE"
```

If the file exists, read it and **resume from the recorded step**.
Do not restart completed tasks. The file format is:

```
TASK=08
STEP=5a
STATUS=in-progress
LAST_SUCCESS=Task 07 verified OK — app renders on device
NOTES=WIT updated, canvas_impl.rs pending
```

If the file does not exist, check git log to determine where you are,
then start at the lowest incomplete task.

---

## Checkpoint protocol — Claude Code must follow this exactly

### After EVERY successful verify block:

```bash
cat > .task-state << 'EOF'
TASK=<current task number>
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task <N> verified OK — <one line summary>
NOTES=
EOF
```

### When starting a step inside a task:

```bash
cat > .task-state << 'EOF'
TASK=<current task number>
STEP=<step number or name>
STATUS=in-progress
LAST_SUCCESS=<copy from previous state>
NOTES=<what was done so far>
EOF
```

### When a step fails:

```bash
cat > .task-state << 'EOF'
TASK=<current task number>
STEP=<step that failed>
STATUS=failed
LAST_SUCCESS=<copy from previous state>
NOTES=<exact error, first 3 lines>
EOF
```

Then attempt to fix using the **Known issues** section of the task.
Update `STATUS=in-progress` once fixing begins.

### On resume (STATUS=in-progress or failed):

1. Read the file — identify TASK, STEP, NOTES
2. Check what already exists on disk before redoing anything
3. Continue from the recorded STEP only

---

## Current status

**Working end-to-end PoC as of 2026-05-15.** Real Compose UIs render on the
Pixel 2 XL (Android 15 / API 35). Tasks 01–14 landed organically while
shipping the full Compose port — not as discrete sequential milestones.

| Task | Description | Status |
|------|-------------|--------|
| 01 | Rust host skeleton, Android cross-compile | ✅ complete |
| 02 | WIT interface + Rust canvas implementation | ✅ complete |
| 03 | Skiko wasmWasi port — basic canvas, text, animation | ✅ complete |
| 04–07 | Compose app, input, coroutines, AOT deploy | ✅ complete |
| 08 | Paint completeness + missing primitives (blendMode, strokeCap/Join, arc, drrect, clip-rrect, skew, matrix concat) | ✅ complete |
| 09 | Path + PathBuilder (SVG serialization) | ✅ complete |
| 10 | TextBlobBuilder — multi-run styled text | ✅ complete |
| 11 | Gradient shaders (linear, radial) | ✅ complete |
| 12 | Image rect drawing | ✅ complete |
| 13 | ColorFilter (tint, invert) | ✅ complete |
| 14 | Paragraph layout — full layout + getRectsForRange / getGlyphPositionAtCoordinate / getWordBoundary | ✅ complete |
| 15 | rsbinder pipeline — AIDL submodule, ProcessState init, codegen wiring | ✅ device-verified 2026-05-17 |
| 16 | Vibrator HAL via IVibrator binder (+ @nullable null-callback workaround) | ✅ device-verified 2026-05-17 — phone buzzes |
| 17 | Lights HAL via ILights binder + new WIT lights interface | ✅ implementation verified; graceful no-op on Pixel 2 XL (no AIDL ILights HAL on this device, Pixel 3+ has it) |
| 19 | IPower + IThermal HALs — performance hints + thermal state | ✅ device-verified 2026-05-17 — 11 sensors, CPU 39°C, throttle NONE |
| 20 | Sensors via ISensorManager (frameworks-layer AIDL) — accel/gyro/proximity/light/etc | ✅ device-verified 2026-05-17 — 29 sensors, accel z=9.61 m/s² |
| 21 | Audio playback via rsbinder to IAAudioService (Path B-AAudio variant) | ✅ device-verified 2026-05-17 — 440 Hz beep audible on Pixel 2 XL speaker |
| 18 | Compose `LocalHapticFeedback` → WIT haptics adapter | ✅ device-verified 2026-05-17 — Material3 button click buzzes Pixel 2 XL |
| 22 | ISurfaceComposer rsbinder round-trip (roadmap §5 de-risk) | ✅ device-verified 2026-05-17 — SurfaceFlinger reachable; transport validated |
| 23 | Profiling hooks (`ResourceLimiter` + `GuestProfiler`) | 🟡 MVP device-verified 2026-05-17 — 3/4 hooks live; GuestProfiler deferred (epoch-interruption cwasm-rebuild needed) |
| 24 | Bisect the WasmGC-heap leak — find the real retention chain (Kotlin/Wasm continuations? kotlinx-coroutines-wasmWasi? Compose Snapshot?) | 🟡 step 1 done 2026-05-17 — minimal reproducer in `wart-leak-repro/`; leak isolated to Kotlin/Wasm `suspend` codegen (~9 MB/s, OOM in 6:37). Steps 2-4 moot; pending upstream file. **UPDATE 2026-05-21:** root cause re-attributed — NOT Kotlin codegen but **wasmtime DRC having no auto-sweep** (sweep only fires on `memory.grow` failure). Filed upstream as [wasmtime#13403](https://github.com/bytecodealliance/wasmtime/issues/13403). fitzgen's fix PR#13422 (auto-GC trigger when the over-approx-roots list doubles) verified on device 2026-05-21: **fixes the leak** (heap bounded — desktop RSS flat 43 MB vs 4 GB baseline) **but reintroduces an ANR** via GC-frequency overhead — the inline `force_gc` fires very frequently on the Compose guest and each GC's root scan (`trace_vmctx_roots`) blocks the render thread. Not a clean fix as-is. Device reverted to the known-good 2.4.257/wasmtime-44 build. See [[wasmtime-drc-no-autoschedule]]. |
| 25 | Diagnose the `suspend` state-machine leak: tighten repro, identify leaked structref type, locate kotlinc-wasm-backend codegen pass | 🔲 scoped, step 1 starting |
| 27 | Skiko WIT-shaped gaps: Image.makeFromEncoded, Shader.makeSweepGradient, Image.makeShader, Shader.makeBlend, Gradient-object overloads on linear/radial/sweep | ✅ device-verified 2026-05-18 — smoke card shows all four new APIs ok; Compose `Brush.sweepGradient` round-trips through the new WIT verb. Bitmap.makeShader deferred. |
| 28 | Wire `org.jetbrains.skia.Canvas(bitmap)` to host-side raster surfaces — 38 new bc-* WIT verbs, full 41-stub buildout, Bitmap.surfaceId + Image.makeFromBitmap snapshot bridge | ✅ device-verified 2026-05-19 — SegmentedButton selected segment (checkmark + label) renders; DatePicker renders/swipe/year-pick work. Chevron `< >` previously blocked by Tooltip SIGILL — unblocked 2026-05-19 by task 30 adapter fork. SegmentedButton *unselected* segments rendered as opaque grey fills with no label text (pre-existing, never worked) — root-caused + fixed in task 32. |
| 29 | Diagnose the Material3 TooltipBox SIGILL on wasi — bisect Press/long-press path to `BasicTooltipState.show()` | ✅ resolved 2026-05-19 via task 30 — was not a Tooltip-specific bug; adapter State at linear-mem 0x10008 was corrupted by Kotlin/Wasm's allocator. Self-heal in adapter fork unblocks Tooltip + DatePicker chevrons. See [[wasi-adapter-state-corruption]]. |
| 30 | WASI adapter `assert_fail` + wasmtime signal-handler diagnosis (root cause of task 29) | ✅ resolved 2026-05-19. Originally framed as a Kotlin/Wasm `ScopedMemoryAllocator.destroy()` range-propagation bug — **but JetBrains rejected that framing on [KT-86415](https://youtrack.jetbrains.com/issue/KT-86415/) (2026-05-20):** real cause is a use-after-free — the WASI adapter holds canonical-ABI `realloc` memory across Kotlin's aggressive `freeAllComponentModelReallocAllocatedMemory`. See [[kotlin-wasm-scopedmemory-destroy-bug]] (UPSTREAM CORRECTION section). Our `2.4.255-SNAPSHOT` stdlib patch was an empirical leak-trade stopgap; the adapter fork additionally self-healed corrupted State. **Both superseded by task 34's Option B fixed-partition fix (device-verified 2026-05-21) — the self-heal is now removed.** ✅ fully closed 2026-05-20 — Step 4 (signal-handler) deferred with re-open criteria; Step 6 (related-bug audit) found no additional retroactive fixes (Tooltip SIGILL was the only bug with this root cause). |
| 31 | Animated `graphicsLayer` invisible inside popups — DropdownMenu/AlertDialog/Tooltip enter animations | ✅ resolved 2026-05-20. Root cause: `SkiaGraphicsLayer.requiresLayer()` sets `RenderNode.layerPaint` for any `alpha<1`; the wasi `RenderNode.drawInto` baked `saveLayer(layerPaint)` into the **parent** recording, freezing alpha at record time (≈0) for popups that record once. Fix in `skiko/.../node/RenderNode.wasi.kt`: emit the recorded `saveLayer` only for genuine filters (colorFilter/imageFilter/non-SrcOver blend); plain alpha rides the live `WasiDrawable` attr. Device-verified — DropdownMenu animates with no `LocalInspectionMode` workaround. See [[popup-overlay]]. |
| 32 | SegmentedButton unselected segments grey-filled, labels hidden | ✅ resolved 2026-05-20. Root cause: `org.jetbrains.skia.Path.makeCombining` (path boolean ops) was a `null`-returning stub on wasi. `Modifier.border` builds a non-uniform rounded border ring via `Path.op(outer, inner, DIFFERENCE)` → null → ring collapsed to a full fill (grey block over the label). Fix: new `path-combine` WIT verb — host runs `skia_safe::Path::op`, converts the EvenOdd result to a Winding-fill path (`as_winding`, since SVG carries no fill rule), returns SVG; skiko `Path.makeCombining` wired to it. General fix — all path-boolean callers now work. Device-verified. |
| 33 | Boot-model bring-up — run the runtime as a standalone privileged process (no NativeActivity) | 🟢 Steps 1, 3, 4, 5 ✅ device-verified (latest 2026-05-26); Step 2 ✅ functionally (codepath runs without Activity) but the `android-activity` dep is still pulled for the NativeActivity sibling — sub-roadmap for the post-ART display path. 5 ordered steps: (1) standalone-surface spike — non-Activity `su`-process creates a fullscreen `SurfaceControl` from SurfaceFlinger via libgui `SurfaceComposerClient`, EGL one frame [keystone de-risk] — **DONE + integrated into wart-host: `cpp/sf_probe.cpp` proved the path (solid blue frame), then `cpp/sf_surface.{cpp,bp}` (soong `cc_library_shared` `libsf_surface.so`, built in-tree) became the real shim — `wart-host --standalone` `dlopen`s it, gets a SurfaceFlinger surface with no Activity, and renders via Skia; device-verified. libgui C++ must be built in-tree (out-of-tree header vendoring is infeasible — AIDL+HIDL codegen fan-out) — see [[project-boot-model-libgui-build]]**; (2) decouple host from NativeActivity/winit-Android — ✅ `src/standalone.rs` runs the full cwasm render loop; (3) input from InputFlinger — ✅ device-verified 2026-05-22 for touch (display-clip + 90°-rotation both fixed — the rotation was an `eglQuerySurface` lie on taimen Adreno, renderer now takes geometry from `ANativeWindow`); ✅ hardware keys 2026-05-26 (`SfInputEvent` extended with `kind=10/11` + `meta_state`, `cpp/sf_surface.cpp` emits AKEYCODE_*, `input::dispatch_android_key` maps to (code-point, key-id) using the same numeric IDs the winit NativeActivity path sends — `adb shell input keyevent KEYCODE_A` types into BasicTextField; needs new `sf_request_focus()` shim export called every ~1 s because activity-backed windows (launcher, last app) steal InputDispatcher focus from non-Activity wart); (4) launch mechanism + SystemUI coexistence — ✅ device-verified 2026-05-26: `scripts/standalone-launch.sh` preflights / pushes-newer / `am force-stop com.android.systemui` + the resolved-home launcher / installs an EXIT trap that restores both, and runs wart-host in the foreground; `scripts/standalone-recover.sh` is the idempotent backup; `am force-stop` is non-persistent so a reboot is always a clean recovery. Init.rc / sepolicy domain remain deferred (production-only); (5) lifecycle / minimal arbiter — ✅ device-verified 2026-05-26: `src/lifecycle_standalone.rs` adds (a) `sigaction` for SIGTERM/SIGINT/SIGHUP → atomic flag → render loop breaks → fire `Destroyed` → drain 3 frames → return `Ok`, (b) screen-state watcher thread polls `debug.tracing.screen_state` every 500 ms and fires `Paused`/`Resumed` on Off↔On (Doze treated as Paused), (c) panic hook writes `/data/local/tmp/wart-host-crash.json`, drained + logged on next launch, removed on clean exit. Cold-start `Created → Started → Resumed` already auto-walks via the bridge. Multi-app arbiter explicitly out of scope. `bash scripts/standalone-launch.sh` is the one-line dev entry point. See `tasks/33-boot-model-bringup.md` + `post-art-roadmap.md` §11. |
| 34 | KT-86415 real fix — adapter-State use-after-free via a fixed linear-memory partition (Option B) | ✅ device-verified 2026-05-21. Static partition shipped: Kotlin stdlib `2.4.258-SNAPSHOT` root `ScopedMemoryAllocator` starts at `RESERVED_BASE=0x20000` (one const in `MemoryAllocation.kt`, `destroy()` left stock — no leak); adapter fork `State::new` places `State` at fixed `STATE_BASE=0x10000` (`[0x10000,0x20000)` reserved). Win 1: DatePicker chevrons + Tooltip long-press exercised — 0 SIGILL / 0 State-corruption. Win 2: idle wasm-linear-memory leak 0.111 MB/s = identical to known-good 2.4.257 baseline (0.114) → no regression; residual is the pre-existing wasmtime-DRC leak [[wasmtime-drc-no-autoschedule]]. Task-30 `State::with` self-heal removed (verified unnecessary). The `2.4.258` stdlib is wired via `~/.gradle/init.d/kt-86415-stdlib-override.gradle.kts`; the override stays until KT-86415 lands upstream. See `tasks/34-kt86415-fixed-partition.md`. |
| 35 | App installer (+ thin loader) — the multi-component package boundary | ✅ device-verified 2026-05-26. Six steps landed: (1) `wart-host/src/app_loader.rs` (AppRef enum + LoadedApp + AppLoader trait + WartLoader); (2) `standalone.rs` refactor; (3) `lib.rs` refactor (App.component → App.loaded; App::new + App::new_from_asset_bytes deleted; find_cwasm_on_filesystem → path-candidate list); (4) `wart-host/src/app_installer.rs` — reads `package.toml`, AOT-precompiles each component via `Engine::precompile_component`, writes spec layout `<root>/<app_id>/<version>/{package.toml, components/, cache/, cache-key.toml}` (cache-key has `wasmtime_version` + `engine_config_hash` over `precompile_compatibility_hash` + per-component sha256s); (5) `AppRef::Installed` self-healing reader in WartLoader — recomputes hashes, on drift re-precompiles + re-stamps; (6) CLI (`wart-host --install <warpkg>` + `--standalone --app <id>`) + scripts/smoke-warpkg.sh fixture + on-device smoke (install → load via Installed → frames render; cache-drift self-heal verified by deleting cwasm). New deps: `sha2`, `toml`. `WART_APPS_ROOT` env override skirts `/data/wart/` sepolicy for smoke. Single-component only — multi-component composition deferred to `tasks/36-cross-app-deps.md`. See `tasks/35-app-install.md`. |
| 36 | Cross-app dependencies + system components — the multi-package boundary | 🟡 steps 1–6 landed 2026-05-26 (library-level wiring complete; device e2e step 7 deferred). Driver: markdown renderer (system-bundled, library-like, same-Store). Q6 resolved (explicit-required `[package].composition`). New: `wit/markdown.wit` (Compose-friendly structured spans record; recursion-flat to fit WIT 0.2), `markdown-renderer/` Rust cdylib component (337 KB, pulldown-cmark backed, `wasmtime run --invoke` validated), manifest schema extension in `app_installer.rs` (`PackageKind { App, System }` + `Composition` + `Dependency` + `[dependencies]` table + `[dependencies_resolved]` cache-key section), resolver that refuses install on missing dep, loader same-Store composition via `wasmtime::component::Linker::instance` (cloned `Guest` accessor; one-dep dispatch by interface string — refactor to registry once N>1), `wart-app-md-smoke/` Kotlin/Wasm consumer with hand-written WIT bindings (wit-bindgen 0.53.1 has no Kotlin generator; skiko's header is misleading). Device-smoke matrix from step 4 passed: missing-dep rejection, system-bundle routes to `system-apps/`, consumer-with-resolved-dep installs, leaf still loads through new `/apps/` layout. Outstanding (step 7): Kotlin/Wasm + wasmtime-CLI "thrown Wasm exception" on `println` / `render()` call; wart-host needs a `wasi:cli/run`-driving mode or pre-composed consumers. See `tasks/36-cross-app-deps.md`. |

**What's verified on device:** BasicTextField + TextFieldState + hardware
keyboard, in-canvas soft keyboard, Material3 widgets (Button, Checkbox,
DropdownMenu, Switch, Slider), LazyColumn + scrolling, warm-resume across
lifecycle events, host-side WasiDrawable transforms, **vibrator HAL via
rsbinder binder transactions** (task 16), **AAudio playback (440 Hz
sine, stereo PCM-f32, 48 kHz)** via rsbinder → `media.aaudio` → MMAP
HAL (task 21), **Compose `LocalHapticFeedback` → WIT haptics adapter**
so Material3 widgets buzz the device (task 18).

**Known issue (not blocking):** indeterminate `ProgressIndicator` leaks
~0.4 MB/s in wasm linear memory due to Kotlin/Wasm continuation retention
in `while(true){ withFrameNanos {} }` loops. Mitigation: use static
`progress = { 0.5f }` widgets. See feedback memory
`feedback_indeterminate_progress_leak`.

**Next work** (scoped, not started): tasks 19 (IPower+IThermal), 20
(sensors), 21 (audio playback) — see `tasks/` for details. Longer-term
direction in `post-art-roadmap.md`. Also outstanding: IME / soft-keyboard
production polish (JNI to Android `InputMethodManager`), Skottie / shaper
for Lottie animation, more exotic ColorFilter modes, address the
ProgressIndicator memory leak. Task 18 (Compose `DefaultHapticFeedback`
adapter → WIT haptics) is the natural follow-up to tasks 16/18 once
device-side HAL is wanted from Compose `performHapticFeedback()`.

---

## Repository layout

```
wart/
  .task-state                          ← checkpoint — never delete
  CLAUDE.md                            ← this file
  .claude/agents/                      ← cargo-triage, gradle-triage, wit-triage, wasm-component-build, skiko-kt-impl
  wart-host/                           ← Rust host binary (wasmtime + skia + winit) + APK [README.md + BUILD.md]
    Cargo.toml
    build.rs
    .cargo/config.toml                 ← Android cross-compile linker (keep API level == Cargo.toml min_sdk_version)
    src/
      lib.rs, main.rs
      canvas_impl.rs                   ← WIT canvas trait → skia-safe
      paragraph_impl.rs                ← host-side Skia paragraph layout
      window_impl.rs, scheduler_impl.rs
      input.rs                         ← winit events → WIT exports
      lifecycle_impl.rs, clipboard_impl.rs, haptics_impl.rs
      locale_impl.rs, pointer_icon_impl.rs, text_segmentation_impl.rs
      egl.rs                           ← EGL context for Android GPU
      bionic_compat.rs                 ← NDK linker shims (Android only)
    cpp/wasi_drawable.cpp              ← SkDrawable subclass with mutable sk_sp<SkPicture>
    assets/skiko-component.cwasm       ← default WASM component, embedded in APK
  wart-app/                            ← Kotlin/Compose guest application [README.md + BUILD.md]
    src/wasmWasiMain/kotlin/           ← Main.kt + per-feature smoke-test files
  wit/
    skiko-gfx.wit                      ← WIT interface — SOURCE OF TRUTH
  skiko/                               ← symlink → ~/skiko (skiko fork) [README-wasmWasi.md + BUILD-wasmWasi.md]
    skiko/src/wasmWasiMain/kotlin/     ← Skia / Skiko stubs + WIT bindings
      generated/                       ← WIT-bindgen output (hand-edited as needed)
      org/jetbrains/skia/              ← SkiaTypes.wasi.kt, paragraph/, icu/, ...
      org/jetbrains/skiko/             ← WasiCanvas.kt, SkiaLayerWasi.kt, wasi/RendererImpl.kt
    skiko/wit/skiko-gfx.wit            ← MIRROR — must stay byte-identical to ../wit/skiko-gfx.wit
  compose-multiplatform-core/          ← in-tree port: 32 wasm-wasi klibs [README-wasmWasi.md + BUILD-wasmWasi.md]
  wart-leak-repro/                     ← minimal Kotlin/Wasm + skiko-wasm-wasi-only repro for task 24 — confirms the WasmGC-heap leak is in `suspendCoroutine` codegen (~9 MB/s, OOM in 6:37 on Pixel 2 XL)
  compose-runtime-wasi/                ← sibling fat klibs (11 dirs) — bundle compose-multiplatform-core
  compose-ui-base-wasi/                  source dirs via srcDirs, package into one klib per dir.
  compose-ui-graphics-wasi/              Used for fast linking (5 min vs 2 h on 32 granular klibs).
  compose-ui-text-wasi/
  compose-ui-wasi/
  compose-foundation-layout-wasi/
  compose-foundation-wasi/
  compose-animation-core-wasi/
  compose-animation-wasi/
  compose-material-ripple-wasi/
  compose-material3-wasi/
  scripts/
    build-aot.sh                       ← AOT-compile .wasm → .cwasm for Android
    build-host-android.sh              ← cargo build wrapper for the rust host
    deploy.sh                          ← push + run on device via ADB
  tasks/                               ← 08–14 task notes (all complete)
```

---

## Task file index

WIT-surface expansion notes (tasks 8–14) describe how each Skia/Compose
feature was originally scoped — implementations landed organically during
the Compose port. HAL-pipeline tasks (15–17) are recent and were executed
sequentially; tasks 19–21 are scoped follow-ups for the runtime-to-HAL
boundary from `post-art-roadmap.md` §3.

| # | File | What it covers |
|---|------|----------------|
| 8 | `tasks/08-paint-completeness.md` | blendMode, strokeCap/Join, arc, drrect, clip-rrect, skew, matrix concat — ✅ |
| 9 | `tasks/09-path-pathbuilder.md` | Path + PathBuilder via SVG string, drawPath, clipPath — ✅ |
| 10 | `tasks/10-textblob-builder.md` | TextBlobBuilder multi-run text, drawTextLine — ✅ |
| 11 | `tasks/11-gradient-shaders.md` | linear + radial gradient shaders, Shader class — ✅ |
| 12 | `tasks/12-image-rect.md` | drawImageRect, Image Kotlin class — ✅ |
| 13 | `tasks/13-color-filter.md` | ColorFilter tint + invert — ✅ |
| 14 | `tasks/14-paragraph.md` | Paragraph layout + getRectsForRange / getGlyphPositionAtCoordinate / getWordBoundary — ✅ |
| 15 | `tasks/15-rsbinder-pipeline.md` | rsbinder + rsbinder-aidl pipeline; AOSP HAL AIDL submodule; SDK 29→30 bump — ✅ |
| 16 | `tasks/16-vibrator-hal.md` | Vibrator HAL via IVibrator binder; @nullable workaround via manual parcel — ✅ device-verified |
| 17 | `tasks/17-lights-hal.md` | New WIT lights interface; ILights binder; graceful no-op on devices w/o AIDL HAL — ✅ |
| 19 | `tasks/19-power-thermal-hal.md` | IPower performance hints + IThermal read-only state — ✅ device-verified; submodule bumped to android-15.0.0_r36 |
| 20 | `tasks/20-sensors-hal.md` | ISensorManager (frameworks-layer AIDL); pull-model sensor sample polling; first Bn-callback server — ✅ device-verified |
| 21 | `tasks/21-audioflinger-playback.md` | Audio playback via rsbinder→IAAudioService; primitives factored for reuse (binder_shared_memory + eventfd_signal) — ✅ device-verified |
| 18 | `tasks/18-compose-haptic-adapter.md` | Compose `LocalHapticFeedback` provider → WIT haptics; closes the Compose-UI ↔ vendor-vibrator-HAL loop set up in task 16 — ✅ device-verified |
| 22 | `tasks/22-isurfacecomposer-roundtrip.md` | rsbinder probe of `SurfaceFlingerAIDL` (`android.gui.ISurfaceComposer`); roadmap §5 de-risk for the eventual boot-model migration — ✅ device-verified |
| 23 | `tasks/23-profiling-hooks.md` (+ scope `tasks/scope-profiling-tools.md`) | Wire `ResourceLimiter` + `GuestProfiler` + per-frame data_size + call_hook behind a `profile` cargo feature; characterizes the ProgressIndicator leak quantitatively + breaks down the ~10–20 ms/frame budget — 🟡 MVP done |
| 24 | `tasks/24-bisect-wasm-leak.md` (+ reproducer `wart-leak-repro/`) | Bisect the ~8 MB/min WasmGC-heap leak from task 23 down to its root. Step 1 done: bare `suspendCoroutine` loop with no Compose / no kotlinx-coroutines leaks at ~9 MB/s, OOMs Pixel 2 XL in 6:37. Isolated to Kotlin/Wasm `suspend` codegen — 🟡 step 1 done |
| 25 | `tasks/25-diagnose-suspend-leak.md` | Diagnostic deep-dive on task 24's leak: tighten reproducer (eliminate WasiScheduler), identify the leaked structref class via wasm-tools dump + patched wasmtime live-object summary, read kotlinc-wasm-backend codegen to find the missing slot-clear — 🔲 scoped |
| 26 | `tasks/26-store-worker-thread.md` | Move wasmtime Store to a worker thread to avoid ANR from long Store::gc cascades. Implemented end-to-end + device-tested; eliminated ANR but introduced worse input-lag accumulation (5-6 s after minutes). Reverted as net regression — ❌ attempted+reverted |
| 27 | `tasks/27-skiko-image-shader-gaps.md` | Implement the WIT-shaped skiko stubs: Image.makeFromEncoded, Shader.makeSweepGradient, Image.makeShader, Shader.makeBlend, Gradient-object overloads — ✅ device-verified 2026-05-18. Bitmap.makeShader deferred (no host-side Bitmap state) |
| 28 | `tasks/28-skiko-abstract-canvas.md` | Wire the abstract org.jetbrains.skia.Canvas's 41 throw-stubs to host-side skia via 38 new bc-* WIT verbs + per-Bitmap host raster Surface. Unblocked SegmentedButton (checkmark + unselected labels visible) and DatePicker render/swipe/year-pick. Chevron taps remain blocked by an orthogonal Material3 TooltipBox bug ([[tooltip-sigill-wasi]]). ✅ device-verified 2026-05-19 |
| 29 | `tasks/29-tooltip-sigill-bisect.md` | Diagnose the Material3 TooltipBox SIGILL on wasi. **Characterized end-to-end 2026-05-19, workaround deferred.** Trigger: clickable Press / long-press timeout → `BasicTooltipState.show()` → `suspendCancellableCoroutine`+`withTimeout` → kotlinx `Delay` → WASI adapter `poll_oneoff` → Rust `assert!` → SIGILL the wasmtime signal handler fails to convert to a Trap on Android. Mitigation in place: wart-app side disables TooltipBox-wrapped widgets (DatePicker chevrons, etc.). Step 4 deferred — see task doc "Step 4 decision" for the four workaround paths considered + why none was taken now. 🟡 characterized |
| 30 | `tasks/30-wasi-adapter-assert-and-wasmtime-signal-handler.md` | Spin-out of task 29 Step 3 — investigate and patch the root cause. Two stacked bugs: (a) WASI P1 reactor adapter trips a Rust `assert!` inside `poll_oneoff` when driven by kotlinx-coroutines `Delay`/`withTimeout` on wasi; (b) wasmtime's signal handler on Android fails to intercept the registered `unreachable` trap, so the process aborts to debuggerd instead of returning Err. 6 steps: capture assert msg via wasi-stderr→logcat, rebuild adapter with DWARF, identify failing precondition, diagnose wasmtime sigaction-on-Android, verify against `TooltipInspectionCard` (kept in place for this purpose), audit related bugs ([[kotlin-wasm-suspendcoroutine-leak]] etc). ✅ fully closed 2026-05-20 (Step 4 deferred w/ re-open criteria; Step 6 audit found no extra retroactive fixes). |
| 33 | `tasks/33-boot-model-bringup.md` | Boot-model bring-up sub-roadmap — get the runtime running as a standalone privileged process (no `NativeActivity`), owning display + input directly, per `post-art-roadmap.md` §11. 5 ordered steps, keystone first: standalone-surface spike (non-Activity `su`-process → fullscreen `SurfaceControl` from SurfaceFlinger via libgui → EGL one frame). Self-contained for a fresh session. 🔲 scoped |
| 34 | `tasks/34-kt86415-fixed-partition.md` | KT-86415 real fix (Option B) — the WASI adapter's 64 KB `State`, `cabi_realloc`'d into Kotlin's `ScopedMemoryAllocator` bump region, was reused after `freeAll` → use-after-free. Fix: static linear-memory partition — `[0x10000,0x20000)` reserved for `State`, Kotlin's root allocator starts at `RESERVED_BASE=0x20000`; one constant in `MemoryAllocation.kt` + the adapter's `State::new`. Kills UAF + leak, no heuristic, `destroy()` stays stock. ✅ device-verified 2026-05-21 — see the status-table row above. |
| 35 | `tasks/35-app-install.md` | App installer (+ thin loader) — the multi-component package boundary. `wart-host/src/app_loader.rs` (AppRef enum + LoadedApp + AppLoader trait + WartLoader; self-healing `AppRef::Installed` reader) + `wart-host/src/app_installer.rs` (WartInstaller calls `Engine::precompile_component`, writes spec-shaped install dir + `cache-key.toml`). CLI: `wart-host --install <warpkg>` + `--standalone --app <id>`. Cache layout parallels Android dex2oat `/data/dalvik-cache/` — apps ship `.wasm`, runtime AOTs `.cwasm` per-device + invalidates on engine/wasm drift. Single-component only — multi-component composition deferred to `tasks/36-cross-app-deps.md`. ✅ device-verified |
| 36 | `tasks/36-cross-app-deps.md` | Cross-app dependencies + system components — `[dependencies]` table walking (host-WIT / runtime-bundled / user-app), two composition modes (same-Store library-like vs separate-Store service-like with host proxy), install-time resolution against `/data/wart/{apps,system-apps}/`, cache-key extension recording dep hashes (any dep update → A's hash flips → re-precompile). Wasmtime stable APIs (`Linker::instance`/`substituted_component_type`/`resource`/`instantiate`) cover everything; true lazy linking still unstable (separate-Store + `OnceCell` is the workaround for cold-path / heavy deps). Open: Q6 (same-Store vs separate-Store default + manifest syntax) — needs a concrete second component to drive. 🔲 scoped |

---

## Architecture: how the layers connect

```
Kotlin wart-app (wasmWasiMain)
  └─ calls: org.jetbrains.skia.Canvas / Paint / Path / Shader / ...
       └─ WasiCanvas.kt delegates to → WIT imports (generated/SkikoUi.kt)
            └─ WIT interface: wit/skiko-gfx.wit
                 └─ Rust host: wart-host/src/canvas_impl.rs implements WIT trait
                      └─ calls: skia_safe::Canvas / Paint / Path / Shader / ...
```

**One-way data flow for a draw call:**
1. Kotlin builds a `Paint`, sets color/blendMode/shader
2. `WasiCanvas.witAttrs()` serializes Paint to a flat `PaintAttrs` WIT record
3. `WitCanvas.Import.drawRect(x, y, w, h, paintAttrs)` crosses the WASM boundary
4. Rust `draw_rect()` calls `make_paint(&attrs)` → `canvas.draw_rect(rect, &paint)`

**Text blob path** (different from draw because host owns the font):
1. Kotlin calls `WitCanvas.Import.createTextBlob(text, family, size, weight, italic)` → `u32` ID
2. Kotlin calls `drawTextBlob(id, x, y, paintAttrs)`
3. Kotlin calls `dropTextBlob(id)` — host frees the resource

**Shader path** (task 11):
1. Kotlin calls `WitCanvas.Import.createLinearGradient(...)` → `u32` shader ID
2. Kotlin stores ID in `Paint.shader`
3. `witAttrs()` includes `shader_id` in the record
4. Rust `make_paint()` looks up shader by ID and applies it

---

## Key decisions already made

- **GPU path:** EGL direct — `libEGL.so` from Android sysroot, EGL context from
  `ANativeWindow`, skia-safe GL backend. Avoids wgpu/Vulkan complexity.

- **wasmtime execution:** AOT on Android (`Component::deserialize_file`), JIT on
  desktop. SELinux blocks W^X without root.

- **Font loading:** `FontMgr::default().match_family_style()` returns
  zero-metrics typefaces on this device. Always load fonts via
  `FontMgr::new_from_data(&ttf_bytes, None)` after reading raw TTF bytes.

- **Text rendering:** CPU rasterize on `raster_n32_premul` surface → blit to GPU
  canvas via `draw_image`. Required because GPU text path needs a different
  skia-safe setup.

- **Path serialization (task 09):** SVG path string format. Kotlin builds the
  SVG string (M/L/C/Q/A/Z commands). Rust host parses with
  `skia_safe::Path::from_svg()`. No custom binary format needed.

- **Shader resources (task 11):** Handle-based (`create-*-gradient` → `u32` ID,
  `drop-shader`). Stored in `HashMap<u32, skia_safe::Shader>` on host side.
  `paint-attrs` extended with `shader_id: u32` (0 = none).

- **Hot-reload workflow:**
  ```bash
  adb push skiko-component.cwasm \
    "/sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm"
  # then restart the app — no APK rebuild
  ```
  Downloads directory is blocked by scoped storage.
  Use the app-specific external dir above (no permission needed).

- **Build pipeline** (Kotlin → cwasm). Full step-by-step in
  `~/wart/wart-app/BUILD.md`; minimal form:
  ```bash
  # 1. (only if you changed Skiko itself) republish skiko-wasm-wasi.klib (~1m 40s)
  cd ~/wart/skiko/skiko
  ./gradlew publishWasmWasiPublicationToMavenLocal \
      -Pskiko.wasmWasi.enabled=true \
      -Dorg.gradle.configureondemand=false \
      --console=plain --no-daemon
  # 1b. (after step 1) republish every compose-*-wasi module that consumes
  #     skiko — symptoms of skipping are subtle behavioral drift, not link
  #     errors. Use the helper script (~15-30 min):
  bash ~/wart/scripts/rebuild-compose-wasi-skiko-depend.sh

  # 2. compile the app to .wasm (links against the 11 sibling fat klibs — ~2 min)
  cd ~/wart/wart-app
  ./gradlew compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon

  # 3. embed WIT + adapt P1→P2 + AOT-compile for aarch64-android
  wasm-tools component embed \
      --world my:skiko-gfx/skiko-ui \
      ~/wart/wit/skiko-gfx.wit \
      build/compileSync/wasmWasi/main/productionExecutable/kotlin/wart-app.wasm \
      -o /tmp/embedded.wasm
  # ⚠ Use the wart-tree fork of the wasi preview1 reactor adapter, NOT
  # ~/wart/skiko/wasi_snapshot_preview1.reactor.wasm. The wart fork
  # patches `State::new` to place the adapter's 64 KB `State` at the
  # fixed address [0x10000,0x20000) instead of via `cabi_realloc` — the
  # KT-86415 Option B fix (task 34). It must be paired with the
  # `2.4.258-SNAPSHOT` Kotlin stdlib (root ScopedMemoryAllocator starts
  # at RESERVED_BASE=0x20000), wired via the init.d override. Mismatched
  # halves = State corruption / SIGILL. See [[kotlin-wasm-scopedmemory-destroy-bug]].
  # Build once (release profile, ~54 KB stripped):
  #   cd ~/wart/wasmtime-src && cargo build \
  #     -p wasi-preview1-component-adapter \
  #     --target wasm32-unknown-unknown --release
  wasm-tools component new /tmp/embedded.wasm \
      --adapt ~/wart/wasmtime-src/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm \
      -o /tmp/skiko-component.wasm
  wasmtime compile --target aarch64-linux-android \
      --wasm component-model --wasm gc --wasm function-references --wasm exceptions \
      -o /tmp/skiko-component.cwasm /tmp/skiko-component.wasm

  # 4. hot-reload onto device (no APK rebuild)
  adb shell am force-stop com.example.wasmruntime
  adb push /tmp/skiko-component.cwasm \
      "/sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm"
  adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
  ```

---

## WIT sync rule

**Whenever `wit/skiko-gfx.wit` changes, sync to skiko repo:**

```bash
cp ~/wart/wit/skiko-gfx.wit \
   ~/wart/skiko/skiko/wit/skiko-gfx.wit
```

Then regenerate or hand-edit the Kotlin bindings in
`skiko/src/wasmWasiMain/kotlin/generated/`.

---

## Environment

- Rust toolchain with `aarch64-linux-android` target
- Android NDK r27 at `~/android-ndk-r27d`
- `adb` connected to rooted Android device (API 29+, arm64)
- `wasmtime` CLI on dev machine
- `wasm-tools` at `~/.cargo/bin/wasm-tools`
- WASI adapter at `~/wart/skiko/wasi_snapshot_preview1.reactor.wasm`
- Kotlin/Gradle at `~/wart/skiko/` (wasmWasi-capable compiler in mavenLocal)
- Java 17+, Gradle 8+

---

## Agents available

Use these agents to keep build output out of the main context:

| Agent | When to use |
|-------|-------------|
| `cargo-triage` | Rust/Cargo build fails, especially Android cross-compile |
| `gradle-triage` | Kotlin/Gradle build fails |
| `wasm-component-build` | Run the full Kotlin → cwasm pipeline and report result |
| `libgui-shim-build` | C++ libgui shim build fails (task 33 standalone path) |
| `rsbinder-triage` | Android binder runtime failures (AVC denials, parcelable drift) |
| `surfaceflinger-triage` | Native display bring-up failures (SurfaceComposerClient, EGL-on-SurfaceControl) |
| `app-installer-triage` | App-installer / loader / precompile_component / cache-key failures (task 35) |

---

## Skiko wasmWasiMain — file reference

| File | Purpose |
|------|---------|
| `generated/SkikoUi.kt` | WIT-generated public API — `Canvas.Import.*` calls |
| `generated/InternalSkikoUi.kt` | Low-level `@WasmImport` external function declarations |
| `org/jetbrains/skia/SkiaTypes.wasi.kt` | Canvas, Paint, Rect, RRect, Font, Typeface, TextBlob, Path stubs |
| `org/jetbrains/skiko/WasiCanvas.kt` | Concrete Canvas implementation — delegates to WIT imports |
| `org/jetbrains/skiko/SkiaLayerWasi.kt` | SkiaLayer stub — beginFrame/endFrame, renderDelegate |
| `org/jetbrains/skiko/wasi/RendererImpl.kt` | WIT renderer export — renderFrame, onPointerEvent, onKeyEvent, onResize |

---

## Do NOT

- Delete or overwrite `.task-state`
- Add Compose dependencies until Skiko API is complete (tasks 08–13)
- Use `FontMgr::default().match_family_style()` — returns zero-metrics typefaces;
  always use `new_from_data(&ttf_bytes, None)`
- Run AOT `.cwasm` on desktop — arm64 only
- Push cwasm to Downloads — use app-specific external dir instead
- Skip `wasm-tools component embed` — `component new` will fail without it
- Use `adb -j2` flag — not supported by cargo-apk; use `CARGO_BUILD_JOBS=2`
- Remove the leading `freeAllComponentModelReallocAllocatedMemory()` from
  the skiko WIT-import bindings (see
  [[wasi-realloc-allocator-pollution]]). Even with the
  Kotlin/Wasm KT-86415 patch landed, the `freeAll` is still required
  — without it, the next `withScopedMemoryAllocator` will
  `IllegalStateException` on `check(reallocAllocator == null)`. The
  KT-86415 fix only makes the freeAll's downstream `destroy()` chain
  no longer leak State's range; it doesn't lift the
  reallocAllocator-must-be-null nesting constraint. To drop the
  freeAll workaround you'd need a separate upstream change making
  `withScopedMemoryAllocator` suspend/resume an active
  `reallocAllocator`. Bigger than KT-86415.
