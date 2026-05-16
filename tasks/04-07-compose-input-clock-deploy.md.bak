# Task 04 — Minimal Compose App (wasmWasi)

## Goal

A Compose app compiles to a WASM component targeting `wasmWasi`. It renders a
rotating rectangle and a circle. No input handling yet (Task 05).

---

## Steps

### 1. Create the app module structure

```bash
mkdir -p wasm-android-runtime/app/src/wasmWasiMain/kotlin
```

### 2. Create `app/build.gradle.kts`

```kotlin
plugins {
    kotlin("multiplatform")
    id("org.jetbrains.compose")
}

kotlin {
    wasmWasi {
        binaries.executable()
    }

    sourceSets {
        val wasmWasiMain by getting {
            dependencies {
                // Skiko wasmWasi build from Task 03
                implementation(project(":skiko"))
                // Compose multiplatform — wasmWasi target
                // NOTE: Compose does not officially support wasmWasi yet.
                // Use the commonMain subset that compiles: runtime + foundation.
                // Remove ui/material if they don't compile — draw directly via Canvas.
                implementation("org.jetbrains.compose.runtime:runtime")
                implementation("org.jetbrains.compose.foundation:foundation")
            }
        }
    }
}
```

> **If Compose runtime doesn't compile for wasmWasi:** use Skiko directly
> without Compose. The `renderDelegate` in SkiaLayer receives a raw Canvas —
> draw with Skiko's Canvas API directly. This is sufficient for the PoC.
> Compose can be added incrementally once the base works.

### 3. Create `app/src/wasmWasiMain/kotlin/Main.kt`

**Version A — with Compose (attempt first):**

```kotlin
import org.jetbrains.skiko.*
import org.jetbrains.compose.ui.unit.dp
import org.jetbrains.compose.ui.*
import org.jetbrains.compose.foundation.*
import org.jetbrains.compose.runtime.*

fun main() {
    val layer = SkiaLayer()
    currentSkiaLayer = layer

    layer.setContent {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(androidx.compose.ui.graphics.Color(0xFF1A1A2E))
        ) {
            // Animated rotating rectangle
            val angle by animateFloatAsState(
                targetValue = 360f,
                animationSpec = infiniteRepeatable(
                    animation = tween(2000, easing = LinearEasing)
                )
            )
            Canvas(modifier = Modifier.fillMaxSize()) {
                translate(size.width / 2, size.height / 2)
                rotate(angle)
                drawRect(
                    color = androidx.compose.ui.graphics.Color(0xFF00D4FF),
                    topLeft = Offset(-60f, -60f),
                    size = androidx.compose.ui.geometry.Size(120f, 120f),
                )
            }
        }
    }
}
```

**Version B — raw Skiko (fallback if Compose doesn't compile):**

```kotlin
import org.jetbrains.skiko.*
import org.jetbrains.skia.*

fun main() {
    val layer = SkiaLayer()
    currentSkiaLayer = layer

    var angle = 0f

    layer.renderDelegate = object : SkikoRenderDelegate {
        override fun onRender(canvas: Canvas, width: Int, height: Int, nanoTime: Long) {
            // Background
            canvas.clear(0xFF1A1A2E.toInt())

            // Rotating rectangle
            canvas.save()
            canvas.translate(width / 2f, height / 2f)
            canvas.rotate(angle, 0f, 0f)
            canvas.drawRect(
                Rect.makeLTRB(-60f, -60f, 60f, 60f),
                Paint().apply {
                    color = 0xFF00D4FF.toInt()
                    mode  = PaintMode.FILL
                    isAntiAlias = true
                }
            )
            canvas.restore()

            // Static circle
            canvas.drawOval(
                Rect.makeLTRB(20f, 20f, 100f, 100f),
                Paint().apply {
                    color = 0xFFFF6B6B.toInt()
                    mode  = PaintMode.STROKE
                    strokeWidth = 3f
                    isAntiAlias = true
                }
            )

            angle = (angle + 1f) % 360f
        }
    }
}
```

Start with Version B — it has no Compose dependency, compiles immediately once
Skiko wasmWasi works. Add Compose in Version A incrementally.

### 4. Build the app to WASM

```bash
cd wasm-android-runtime/app
./gradlew wasmWasiMainClasses 2>&1 | tail -20
```

### 5. Convert core WASM to component

```bash
# Find the output .wasm
WASM=$(find . -name "*.wasm" -path "*/wasmWasi/*" | head -1)
echo "Found: $WASM"

# Get the WASI adapter from bashor's repo (one-time download)
# This adapter matches the cm-prototype compiler's WASI calls
curl -L -o wasi_snapshot_preview1.reactor.wasm \
  "https://github.com/bashor/kotlin-wasm-cm-experiments/raw/main/wasi_snapshot_preview1.reactor.wasm"

# Convert to component
wasm-tools component new "$WASM" \
  --adapt wasi_snapshot_preview1.reactor.wasm \
  -o compose-app.wasm

echo "Component size: $(du -h compose-app.wasm | cut -f1)"
```

## Verify

```bash
wasm-tools component wit compose-app.wasm | head -30
# Expected: shows the skiko-ui world with:
#   import my:skiko-gfx/canvas
#   export render-frame: func(nanos: u64)
#   export on-pointer-event: ...
#   export on-resize: ...
```

### ✅ Checkpoint — write after WIT check passes

```bash
cat > wasm-android-runtime/.task-state << 'EOF'
TASK=04
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 04 verified OK — compose-app.wasm built and converted to component with correct exports
NOTES=
EOF
```

---

# Task 05 — Input Events + Lifecycle

## Goal

Touch and keyboard events flow from winit → Rust host → WIT export →
Kotlin guest → Compose/Skiko input system. App is interactive.

---

## Steps

### 1. Update `host/src/input.rs`

```rust
// host/src/input.rs

use wasmtime::component::Instance;
use wasmtime::Store;
use crate::HostState;
use crate::bindings::SkikoUi;

/// Dispatch a pointer event into the WASM guest.
pub fn dispatch_pointer(
    bindings: &SkikoUi,
    store: &mut Store<HostState>,
    kind: u8,    // 0=down 1=up 2=move 3=scroll
    x: f32, y: f32,
) -> anyhow::Result<()> {
    use crate::bindings::my::skiko_gfx::renderer::PointerKind;
    let kind = match kind {
        0 => PointerKind::Down,
        1 => PointerKind::Up,
        2 => PointerKind::Move,
        _ => PointerKind::Scroll,
    };
    bindings.my_skiko_gfx_renderer()
        .call_on_pointer_event(store, kind, x, y)?;
    Ok(())
}

pub fn dispatch_key(
    bindings: &SkikoUi,
    store: &mut Store<HostState>,
    kind: u8, key_code: u32,
) -> anyhow::Result<()> {
    use crate::bindings::my::skiko_gfx::renderer::KeyKind;
    let kind = if kind == 0 { KeyKind::Down } else { KeyKind::Up };
    bindings.my_skiko_gfx_renderer()
        .call_on_key_event(store, kind, key_code)?;
    Ok(())
}

pub fn dispatch_resize(
    bindings: &SkikoUi,
    store: &mut Store<HostState>,
    w: u32, h: u32,
) -> anyhow::Result<()> {
    bindings.my_skiko_gfx_renderer()
        .call_on_resize(store, w, h)?;
    Ok(())
}
```

### 2. Update `host/src/main.rs` — wire events in `window_event`

In the `window_event` match, replace the stubs:

```rust
WindowEvent::Touch(touch) => {
    let kind: u8 = match touch.phase {
        TouchPhase::Started   => 0,
        TouchPhase::Ended
        | TouchPhase::Cancelled => 1,
        TouchPhase::Moved     => 2,
    };
    if let Some(b) = &state.bindings {
        let _ = input::dispatch_pointer(
            b, &mut state.store,
            kind,
            touch.location.x as f32,
            touch.location.y as f32,
        );
        state.window.as_ref().map(|w| w.request_redraw());
    }
}

WindowEvent::CursorMoved { position, .. } => {
    state.last_cursor = (position.x as f32, position.y as f32);
    if let Some(b) = &state.bindings {
        let _ = input::dispatch_pointer(
            b, &mut state.store, 2,
            position.x as f32, position.y as f32,
        );
    }
}

WindowEvent::MouseInput { state: btn_state, button, .. } => {
    let kind: u8 = if btn_state == ElementState::Pressed { 0 } else { 1 };
    if let Some(b) = &app_state.bindings {
        let _ = input::dispatch_pointer(
            b, &mut app_state.store, kind,
            app_state.last_cursor.0, app_state.last_cursor.1,
        );
        app_state.window.as_ref().map(|w| w.request_redraw());
    }
}

WindowEvent::KeyboardInput { event, .. } => {
    let kind: u8 = if event.state == ElementState::Pressed { 0 } else { 1 };
    let code = match event.physical_key {
        PhysicalKey::Code(c) => c as u32,
        _ => 0,
    };
    if let Some(b) = &app_state.bindings {
        let _ = input::dispatch_key(b, &mut app_state.store, kind, code);
    }
}

WindowEvent::Resized(size) => {
    app_state.renderer.resize(size.width, size.height);
    if let Some(b) = &app_state.bindings {
        let _ = input::dispatch_resize(
            b, &mut app_state.store, size.width, size.height);
    }
    app_state.window.as_ref().map(|w| w.request_redraw());
}
```

## Verify

Run on desktop:
```bash
cd host && cargo run
# Touch/click the window — no crash, events dispatched
# (Compose will respond if Task 04 Compose path is used;
#  raw Skiko path ignores input at this stage)
```

### ✅ Checkpoint — write after desktop input test passes

```bash
cat > wasm-android-runtime/.task-state << 'EOF'
TASK=05
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 05 verified OK — touch/click events dispatched without crash
NOTES=
EOF
```

---

# Task 06 — Coroutines Clock (wasmWasi actual)

## Goal

`kotlinx.coroutines` compiles for `wasmWasi`. Animations driven by
`LaunchedEffect` / `animateFloatAsState` work. The clock uses WASI monotonic
clock, not the browser's `performance.now()`.

---

## Steps

### 1. Check if coroutines already compile

```bash
cd app
./gradlew wasmWasiMainClasses 2>&1 | grep -i "clock\|dispatcher\|coroutine\|unresolved"
```

If no errors about clock/dispatcher → skip to Verify.

### 2. Add `wasmWasiMain` actual for the clock

In the Skiko fork (or a separate module), add:

File: `skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/ClockWasi.kt`

```kotlin
package org.jetbrains.skiko.wasi

// WASI monotonic clock — imported by Kotlin/Wasm WASI stdlib
// The exact import name depends on the cm-prototype stdlib version.
// Check: jar tf ~/.m2/.../kotlin-stdlib-wasm-wasi-*.jar | grep clock

@WasmImport("wasi:clocks/monotonic-clock@0.2.0", "now")
private external fun wasiClockNow(): Long   // returns nanoseconds

object WasiClock {
    fun nanoTime(): Long = wasiClockNow()
}
```

### 3. Provide `Dispatchers.Main` actual if missing

```kotlin
// skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/WasiDispatcher.kt
package org.jetbrains.skiko.wasi

import kotlinx.coroutines.*
import kotlin.coroutines.CoroutineContext

/**
 * A single-threaded coroutine dispatcher driven by the render-frame loop.
 * All coroutines queued here run when [tick] is called from render-frame.
 */
object WasiMainDispatcher : CoroutineDispatcher() {
    private val pending = ArrayDeque<Runnable>()

    override fun dispatch(context: CoroutineContext, block: Runnable) {
        pending.addLast(block)
    }

    /** Called from [exportedRenderFrame] before onRender. */
    fun tick() {
        // Drain the queue — each tick processes all pending continuations
        repeat(pending.size) {
            pending.removeFirstOrNull()?.run()
        }
    }
}
```

Update `SkiaLayerWasi.kt` to call `tick()`:

```kotlin
@WasmExport("render-frame")
fun exportedRenderFrame(nanos: Long) {
    WasiMainDispatcher.tick()          // run pending coroutines first
    currentSkiaLayer?.doFrame(nanos)
}
```

### 4. Register WasiMainDispatcher as Dispatchers.Main

```kotlin
// skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/WasiMainDispatcherFactory.kt
package org.jetbrains.skiko.wasi

import kotlinx.coroutines.MainCoroutineDispatcher
import kotlinx.coroutines.internal.MainDispatcherFactory

// Registers WasiMainDispatcher as Dispatchers.Main via ServiceLoader-like mechanism
class WasiMainDispatcherFactory : MainDispatcherFactory {
    override val loadPriority: Int get() = 0
    override fun createDispatcher(allFactories: List<MainDispatcherFactory>)
        : MainCoroutineDispatcher = WasiMainDispatcher as MainCoroutineDispatcher
}
```

> If `MainDispatcherFactory` isn't accessible, set Dispatchers.Main explicitly
> in `main()` before launching any coroutines:
> ```kotlin
> Dispatchers.setMain(WasiMainDispatcher)
> ```

## Verify

```bash
cd app
./gradlew wasmWasiMainClasses 2>&1 | grep -E "error:|BUILD"
# Expected: BUILD SUCCESSFUL, no coroutine/clock errors
```

### ✅ Checkpoint — write after build succeeds

```bash
cat > wasm-android-runtime/.task-state << 'EOF'
TASK=06
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 06 verified OK — coroutines compile for wasmWasi, no clock errors
NOTES=
EOF
```

---

# Task 07 — AOT Compile + ADB Deploy

## Goal

`compose-app.wasm` AOT-compiled to `compose-app.cwasm` for `aarch64-linux-android`.
Host binary cross-compiled for arm64. Both pushed to device via ADB.
App runs, renders, accepts touch input, no ART involved.

---

## Steps

### 1. Create `scripts/build-aot.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

WASM_INPUT="${1:-compose-app.wasm}"
OUTPUT="compose-app.cwasm"
TARGET="aarch64-linux-android"

echo "=== AOT compiling $WASM_INPUT for $TARGET ==="

wasmtime compile \
  --target "$TARGET" \
  --wasm component-model \
  -o "$OUTPUT" \
  "$WASM_INPUT"

echo "=== Output: $OUTPUT ($(du -h $OUTPUT | cut -f1)) ==="
```

```bash
chmod +x wasm-android-runtime/scripts/build-aot.sh
```

### 2. Create `scripts/build-host-android.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

: "${ANDROID_NDK_HOME:?ANDROID_NDK_HOME must be set}"

NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
export PATH="$NDK_BIN:$PATH"
export CC_aarch64_linux_android="$NDK_BIN/aarch64-linux-android35-clang"
export CXX_aarch64_linux_android="$NDK_BIN/aarch64-linux-android35-clang++"
export AR_aarch64_linux_android="$NDK_BIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"

echo "=== Building host for aarch64-linux-android ==="
cd host
cargo build --target aarch64-linux-android --release 2>&1

HOST_BIN="target/aarch64-linux-android/release/wasm-android-host"
echo "=== Host binary: $(du -h $HOST_BIN | cut -f1) ==="
```

```bash
chmod +x wasm-android-runtime/scripts/build-host-android.sh
```

### 3. Create `scripts/deploy.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

DEVICE_DIR="/data/local/tmp/wasm-runtime"
HOST_BIN="host/target/aarch64-linux-android/release/wasm-android-host"
WASM_COMPONENT="compose-app.cwasm"

echo "=== Checking ADB connection ==="
adb devices | grep -v "List of"

echo "=== Pushing files to $DEVICE_DIR ==="
adb shell "mkdir -p $DEVICE_DIR"
adb push "$HOST_BIN"       "$DEVICE_DIR/wasm-host"
adb push "$WASM_COMPONENT" "$DEVICE_DIR/compose-app.cwasm"

adb shell "chmod +x $DEVICE_DIR/wasm-host"

echo "=== Checking required .so files on device ==="
# These must exist on the device — they are part of Android
for lib in libEGL.so libGLESv2.so libandroid.so liblog.so; do
    adb shell "ls /system/lib64/$lib" && echo "  ✓ $lib" || echo "  ✗ MISSING: $lib"
done

echo ""
echo "=== To run (requires root): ==="
echo "  adb shell su -c '$DEVICE_DIR/wasm-host $DEVICE_DIR/compose-app.cwasm'"
echo ""
echo "=== To see logs: ==="
echo "  adb logcat -s wasm-runtime:* RustPanic:*"
```

```bash
chmod +x wasm-android-runtime/scripts/deploy.sh
```

### 4. Update `host/src/main.rs` — add WASM loading

Add to `App` struct:
```rust
struct App {
    window:    Option<Arc<Window>>,
    renderer:  Option<canvas_impl::SkiaRenderer>,
    store:     wasmtime::Store<HostState>,
    engine:    wasmtime::Engine,
    bindings:  Option<crate::bindings::SkikoUi>,
    last_cursor: (f32, f32),
}
```

Load the component in `resumed()`, after renderer init:

```rust
fn resumed(&mut self, event_loop: &ActiveEventLoop) {
    // ... existing window + renderer setup ...

    // Load WASM component (AOT on Android, JIT on desktop)
    let wasm_path = std::env::args().nth(1)
        .unwrap_or_else(|| "compose-app.cwasm".to_string());

    let component = if wasm_path.ends_with(".cwasm") {
        // AOT precompiled — safe on Android (no W^X)
        unsafe {
            wasmtime::component::Component::deserialize_file(
                &self.engine, &wasm_path)
        }.expect("deserialize cwasm failed")
    } else {
        // JIT — requires W^X (desktop or rooted Android)
        wasmtime::component::Component::from_file(
            &self.engine, &wasm_path)
        .expect("load wasm failed")
    };

    let mut linker = wasmtime::component::Linker::new(&self.engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker).unwrap();
    crate::bindings::SkikoUi::add_to_linker(&mut linker, |s: &mut HostState| s)
        .unwrap();

    let (bindings, _) = crate::bindings::SkikoUi::instantiate(
        &mut self.store, &component, &linker)
    .expect("instantiate failed");

    self.bindings = Some(bindings);
    log::info!("WASM component loaded successfully");

    self.window.as_ref().map(|w| w.request_redraw());
}
```

In `RedrawRequested`:

```rust
WindowEvent::RedrawRequested => {
    if let Some(b) = &self.bindings {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        b.my_skiko_gfx_renderer()
            .call_render_frame(&mut self.store, nanos)
            .expect("render_frame failed");
        // request next frame for continuous animation
        self.window.as_ref().map(|w| w.request_redraw());
    }
}
```

### 5. Full build + deploy sequence

```bash
cd wasm-android-runtime

# Step 1: Build Skiko wasmWasi
./skiko/gradlew -p skiko :skiko:wasmWasiJar

# Step 2: Build app
./app/gradlew -p app wasmWasiMainClasses

# Step 3: Convert to component
WASM=$(find app -name "*.wasm" -path "*/wasmWasi/*" | head -1)
wasm-tools component new "$WASM" \
  --adapt wasi_snapshot_preview1.reactor.wasm \
  -o compose-app.wasm

# Step 4: AOT compile for Android
./scripts/build-aot.sh compose-app.wasm

# Step 5: Build host for Android
./scripts/build-host-android.sh

# Step 6: Deploy
./scripts/deploy.sh

# Step 7: Run on device
adb shell su -c '/data/local/tmp/wasm-runtime/wasm-host /data/local/tmp/wasm-runtime/compose-app.cwasm'
```

### 6. Watch logs

```bash
# In a separate terminal
adb logcat -s wasm-runtime AndroidRuntime:E | head -50
```

---

## Verify

**Success looks like:**

```
adb logcat output:
  D wasm-runtime: android_main start
  D wasm-runtime: EGL 1.5
  D wasm-runtime: WASM component loaded successfully
  D wasm-runtime: resumed — creating window

Device screen: dark blue background, rotating cyan rectangle, red circle
```

**Test input:**
```bash
# Simulate touch via ADB
adb shell input tap 200 400
# Expected: no crash; if Compose path used, button presses register
```

### ✅ Checkpoint — write after app renders on device

```bash
cat > wasm-android-runtime/.task-state << 'EOF'
TASK=07
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 07 verified OK — app renders on Android device, touch input works, no ART
NOTES=
EOF
```

**🎉 All tasks complete.** The system is working.

---

## Known issues

### `deserialize_file` panics: "Module compiled with concurrency support"

Wasmtime v41–v42 bug. Workaround:
```rust
let mut config = wasmtime::Config::new();
config.wasm_component_model(true);
config.wasm_threads(false);   // ADD THIS
let engine = wasmtime::Engine::new(&config).unwrap();
```

Or compile the WASM with `--wasm-features=-threads`:
```bash
wasmtime compile \
  --target aarch64-linux-android \
  --wasm component-model \
  --wasm-features=-threads \
  -o compose-app.cwasm \
  compose-app.wasm
```

### `libskia.a` not found for Android

skia-safe builds Skia from source. Set:
```bash
export SKIA_NINJA_COMMAND=/path/to/ninja
export SKIA_GN_COMMAND=/path/to/gn
# Or use the pre-built skia approach:
# Add to host/Cargo.toml:
# [features]
# skia-safe = { version = "0.75", features = ["gl", "use-system-jpeg-turbo"] }
# and set SKIA_NINJA_COMMAND in the build environment
```

Alternatively add to `host/Cargo.toml`:
```toml
[features]
default = ["skia-safe/gl"]

[dependencies.skia-safe]
version = "0.75"
features = ["gl", "textlayout", "embed-icudtl"]
```

### App renders but screen stays black

EGL surface may have wrong dimensions. Add debug log in `SkiaRenderer::new()`:
```rust
log::debug!("EGL surface: {}x{}", egl.width, egl.height);
```
If both are 0, the `ANativeWindow` isn't ready yet — ensure GPU init happens
in `Resumed` event, not in `new()`. The `Resumed` event guarantees the window
surface is valid on Android.

### App crashes with `SIGSEGV` in `skia_safe`

Usually the GL context isn't current when Skia tries to draw. Before each
`draw_test_frame()` / `render_frame()` call, ensure EGL context is current:
```rust
// Add to SkiaRenderer
fn make_current(&self) {
    #[cfg(target_os = "android")]
    unsafe { eglMakeCurrent(self.egl.display, self.egl.surface,
                             self.egl.surface, self.egl.context); }
}
```

## Do NOT

- Do not run the AOT-compiled `.cwasm` on desktop — it's compiled for arm64.
  Use the `.wasm` directly on desktop: `cargo run compose-app.wasm`.
- Do not use `Component::from_file()` on unrooted Android — SELinux will
  block the W^X memory allocation and the process will crash silently.
- Do not skip the `wasm-tools component new` step — raw `.wasm` is not a
  component and wasmtime's component API will reject it.
- Do not push the debug build to device if it's too large — use `--release`.
