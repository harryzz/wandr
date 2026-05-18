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
| 24 | Bisect the WasmGC-heap leak — find the real retention chain (Kotlin/Wasm continuations? kotlinx-coroutines-wasmWasi? Compose Snapshot?) | 🟡 step 1 done 2026-05-17 — minimal reproducer in `wart-leak-repro/`; leak isolated to Kotlin/Wasm `suspend` codegen (~9 MB/s, OOM in 6:37). Steps 2-4 moot; pending upstream file. |
| 25 | Diagnose the `suspend` state-machine leak: tighten repro, identify leaked structref type, locate kotlinc-wasm-backend codegen pass | 🔲 scoped, step 1 starting |
| 27 | Skiko WIT-shaped gaps: Image.makeFromEncoded, Shader.makeSweepGradient, Image.makeShader, Shader.makeBlend, Gradient-object overloads on linear/radial/sweep | ✅ device-verified 2026-05-18 — smoke card shows all four new APIs ok; Compose `Brush.sweepGradient` round-trips through the new WIT verb. Bitmap.makeShader deferred. |

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
| 28 | `tasks/28-skiko-abstract-canvas.md` | Wire the abstract org.jetbrains.skia.Canvas's 42 throw-stubs to host-side skia via a new intermediate-canvas WIT resource. Unblocks DatePicker / SegmentedButton / TimePicker — 🔲 scoped |

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
  wasm-tools component new /tmp/embedded.wasm \
      --adapt ~/skiko/wasi_snapshot_preview1.reactor.wasm \
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
| `wit-triage` | WIT validation errors, wasm-tools component issues |
| `wasm-component-build` | Run the full Kotlin → cwasm pipeline and report result |
| `skiko-kt-impl` | Implement Kotlin wasmWasi stubs for a batch of new WIT functions |

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
