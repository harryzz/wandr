# Task 03 — Skiko wasmWasi Fork

## Goal

The existing patched Skiko repo (from the PoC) is symlinked or referenced
from the project. The `wasmWasiMain` source set files written during the PoC
are confirmed present and the `wasmWasi` Gradle target produces a `.wasm` file.

**This task does NOT clone Skiko from scratch.** You already have a working
patched Skiko from the PoC work. This task wires it into the new project
layout and verifies the build still works.

---

## Prerequisites

### Locate your existing patched Skiko

Find the directory where you did the PoC Skiko work. It should already have:

```bash
# Check these exist in your patched Skiko:
ls <your-skiko-path>/src/wasmWasiMain/kotlin/org/jetbrains/skiko/
# Expected: WitImports.kt  SkiaLayerWasi.kt  WasmMemory.kt  (etc.)

ls <your-skiko-path>/src/wasmWasiMain/kotlin/org/jetbrains/skia/
# Expected: Canvas.wasmWasi.kt  Path.wasmWasi.kt  (etc.)

grep "wasmWasi" <your-skiko-path>/build.gradle.kts
# Expected: wasmWasi { binaries.library() } block present
```

If any of these are missing, the PoC source files need to be added first —
see the file content in steps 6–10 below.

### cm-prototype Kotlin compiler

Must already be published to mavenLocal from the PoC work:

```bash
find ~/.m2/repository/org/jetbrains/kotlin -name "kotlin-stdlib-wasm-wasi-*.jar" | head -3
# Expected: at least one jar found
```

If empty — the compiler was not published. Run:
```bash
# One-time, takes 20-30 min
git clone https://github.com/JetBrains/kotlin \
  -b skuzmich/cm-prototype --depth 1 kotlin-cm-prototype
cd kotlin-cm-prototype
./gradlew publishToMavenLocal \
  -Pkotlin.build.isObsoleteJdkOverrideEnabled=true \
  --parallel -x test
```

---

## Steps

### 1. Link existing patched Skiko into the project

```bash
cd wasm-android-runtime

# Option A: symlink (recommended — changes in skiko reflect immediately)
ln -s /absolute/path/to/your/patched/skiko skiko

# Option B: if skiko is already inside this directory, just confirm:
ls skiko/src/wasmWasiMain/
```

**Tell Claude Code the path:** Replace `/absolute/path/to/your/patched/skiko`
with the actual path to your existing Skiko repo that has the wasmWasi patches.

### 2. Verify `build.gradle.kts` has the wasmWasi target

```bash
grep -A3 "wasmWasi" skiko/build.gradle.kts
# Expected output contains:
#   wasmWasi {
#       binaries.library()
#   }
```

If the block is missing, add it immediately after the `wasmJs { }` block:
```kotlin
wasmWasi {
    binaries.library()
}
```

Verify the `sourceSets` block has `wasmWasiMain`:
```bash
grep "wasmWasiMain" skiko/build.gradle.kts
# Expected: at least one match
```

If missing, add inside the `sourceSets { }` block:
```kotlin
val wasmWasiMain by getting {
    dependsOn(commonMain.get())
    kotlin.srcDir("src/wasmWasiMain/kotlin")
}
```

### 3. Verify the WIT import names match the current WIT file

The WIT file was updated in Task 02. The `@WasmImport` names in
`WitImports.kt` must match exactly. Check:

```bash
# Show what the WIT file defines
grep "func\|interface" wasm-android-runtime/wit/skiko-gfx.wit | head -20

# Show what WitImports.kt imports
grep "@WasmImport" skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/WitImports.kt | head -10
```

Both must use the same interface name (`my:skiko-gfx/canvas`) and function
names (kebab-case, e.g. `surface-width`, `begin-frame`). If the PoC used
different names, update `WitImports.kt` to match `skiko-gfx.wit`.

### 4. Verify the cm-prototype Kotlin version is set

Check what version the PoC already configured:
```bash
grep -E "kotlin.version|kotlin_version" skiko/gradle.properties skiko/gradle/libs.versions.toml 2>/dev/null
# Expected: something like kotlin.version=2.x.x-cm-prototype-SNAPSHOT
```

If the version string is already set and matches what's in `~/.m2`, skip ahead.
If not set or wrong, find the correct version string:
```bash
ls ~/.m2/repository/org/jetbrains/kotlin/kotlin-stdlib-wasm-wasi/
# Shows: 2.x.x-cm-prototype-SNAPSHOT/  (or similar)
```

Set it in `skiko/gradle.properties`:
```properties
kotlin.version=<exact-version-from-above>
```

In `skiko/settings.gradle.kts`, ensure mavenLocal is first:
```kotlin
pluginManagement {
    repositories {
        mavenLocal()
        gradlePluginPortal()
        mavenCentral()
    }
}
dependencyResolutionManagement {
    repositories {
        mavenLocal()
        mavenCentral()
        google()
    }
}
```

### 5. Verify wasmWasiMain source files exist

```bash
# Check which files already exist from the PoC
find skiko/src/wasmWasiMain -name "*.kt" | sort
```

**Expected files** (all should be present from PoC work):
```
skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/WitImports.kt
skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/SkiaLayerWasi.kt
skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/WasmMemory.kt
skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/Canvas.wasmWasi.kt
skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/Path.wasmWasi.kt
```

**If any file is missing**, create it using the content from the
corresponding section below. Only create files that are absent.

Write a checkpoint before starting file creation:

```bash
cat > wasm-android-runtime/.task-state << 'EOF'
TASK=03
STEP=5-file-creation
STATUS=in-progress
LAST_SUCCESS=Task 02 verified OK — WIT parses clean, bindgen! generates Rust types, host compiles
NOTES=Creating missing wasmWasiMain source files
EOF
```

---

### 5a. IF MISSING: `WitImports.kt`

```bash
mkdir -p skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko
```

```kotlin
// skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/WitImports.kt
package org.jetbrains.skiko.wasi

@WasmImport("my:skiko-gfx/canvas", "surface-width")
external fun witSurfaceWidth(): Int

@WasmImport("my:skiko-gfx/canvas", "surface-height")
external fun witSurfaceHeight(): Int

@WasmImport("my:skiko-gfx/canvas", "begin-frame")
external fun witBeginFrame()

@WasmImport("my:skiko-gfx/canvas", "end-frame")
external fun witEndFrame()

@WasmImport("my:skiko-gfx/canvas", "save")
external fun witSave()

@WasmImport("my:skiko-gfx/canvas", "restore")
external fun witRestore()

@WasmImport("my:skiko-gfx/canvas", "translate")
external fun witTranslate(dx: Float, dy: Float)

@WasmImport("my:skiko-gfx/canvas", "scale")
external fun witScale(sx: Float, sy: Float)

@WasmImport("my:skiko-gfx/canvas", "rotate")
external fun witRotate(degrees: Float)

@WasmImport("my:skiko-gfx/canvas", "clip-rect")
external fun witClipRect(x: Float, y: Float, w: Float, h: Float, antiAlias: Boolean)

@WasmImport("my:skiko-gfx/canvas", "clear")
external fun witClear(argb: Int)

@WasmImport("my:skiko-gfx/canvas", "draw-rect")
external fun witDrawRect(x: Float, y: Float, w: Float, h: Float,
                         color: Int, style: Int, strokeWidth: Float,
                         antiAlias: Boolean, alpha: Byte)

@WasmImport("my:skiko-gfx/canvas", "draw-rrect")
external fun witDrawRRect(x: Float, y: Float, w: Float, h: Float,
                          rx: Float, ry: Float,
                          color: Int, style: Int, strokeWidth: Float,
                          antiAlias: Boolean, alpha: Byte)

@WasmImport("my:skiko-gfx/canvas", "draw-oval")
external fun witDrawOval(x: Float, y: Float, w: Float, h: Float,
                         color: Int, style: Int, strokeWidth: Float,
                         antiAlias: Boolean, alpha: Byte)

@WasmImport("my:skiko-gfx/canvas", "draw-line")
external fun witDrawLine(x0: Float, y0: Float, x1: Float, y1: Float,
                         color: Int, style: Int, strokeWidth: Float,
                         antiAlias: Boolean, alpha: Byte)

@WasmImport("my:skiko-gfx/canvas", "draw-path")
external fun witDrawPath(cmdsPtr: Int, cmdsLen: Int,
                         color: Int, style: Int, strokeWidth: Float,
                         antiAlias: Boolean, alpha: Byte)

@WasmImport("my:skiko-gfx/canvas", "create-image")
external fun witCreateImage(width: Int, height: Int, pixelsPtr: Int, pixelsLen: Int): Int

@WasmImport("my:skiko-gfx/canvas", "draw-image")
external fun witDrawImage(id: Int, x: Float, y: Float, alpha: Byte)

@WasmImport("my:skiko-gfx/canvas", "drop-image")
external fun witDropImage(id: Int)

@WasmImport("my:skiko-gfx/canvas", "create-text-blob")
external fun witCreateTextBlob(textPtr: Int, textLen: Int,
                                familyPtr: Int, familyLen: Int,
                                size: Float, weight: Int): Int

@WasmImport("my:skiko-gfx/canvas", "draw-text-blob")
external fun witDrawTextBlob(id: Int, x: Float, y: Float,
                              color: Int, style: Int, strokeWidth: Float,
                              antiAlias: Boolean, alpha: Byte)

@WasmImport("my:skiko-gfx/canvas", "drop-text-blob")
external fun witDropTextBlob(id: Int)
```

---

### 5b. IF MISSING: `Canvas.wasmWasi.kt`

```bash
mkdir -p skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia
```

```kotlin
// skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/Canvas.wasmWasi.kt
package org.jetbrains.skia

import org.jetbrains.skia.impl.NativePointer
import org.jetbrains.skiko.wasi.*

actual class Canvas internal constructor(
    @Suppress("UNUSED_PARAMETER") ptr: NativePointer,
    @Suppress("UNUSED_PARAMETER") managed: Boolean,
    @Suppress("UNUSED_PARAMETER") owner: Any?
) {
    actual fun save(): Int    { witSave(); return 0 }
    actual fun restore()      { witRestore() }
    actual fun translate(dx: Float, dy: Float) = witTranslate(dx, dy)
    actual fun scale(sx: Float, sy: Float)     = witScale(sx, sy)
    actual fun rotate(degrees: Float, px: Float, py: Float) {
        witTranslate(px, py); witRotate(degrees); witTranslate(-px, -py)
    }
    actual fun clipRect(rect: Rect, mode: ClipMode, antiAlias: Boolean) {
        witClipRect(rect.left, rect.top, rect.width, rect.height, antiAlias)
    }
    actual fun clear(color: Int) = witClear(color)
    actual fun drawRect(rect: Rect, paint: Paint) {
        val (c,st,sw,aa,a) = paint.toWit()
        witDrawRect(rect.left, rect.top, rect.width, rect.height, c,st,sw,aa,a)
    }
    actual fun drawOval(bounds: Rect, paint: Paint) {
        val (c,st,sw,aa,a) = paint.toWit()
        witDrawOval(bounds.left, bounds.top, bounds.width, bounds.height, c,st,sw,aa,a)
    }
    actual fun drawLine(x0: Float, y0: Float, x1: Float, y1: Float, paint: Paint) {
        val (c,st,sw,aa,a) = paint.toWit()
        witDrawLine(x0, y0, x1, y1, c,st,sw,aa,a)
    }
    actual fun drawPath(path: Path, paint: Paint) {
        val bytes = path.serializeToBytes()
        val ptr = WasmMemory.copyToLinear(bytes)
        val (c,st,sw,aa,a) = paint.toWit()
        witDrawPath(ptr, bytes.size, c,st,sw,aa,a)
        WasmMemory.free(ptr)
    }
    actual fun drawCircle(cx: Float, cy: Float, r: Float, paint: Paint) {
        drawOval(Rect.makeLTRB(cx-r, cy-r, cx+r, cy+r), paint)
    }
}

private data class PaintWit(val color: Int, val style: Int,
    val strokeWidth: Float, val antiAlias: Boolean, val alpha: Byte)
private fun Paint.toWit() = PaintWit(
    color = this.color,
    style = when (this.mode) {
        PaintMode.FILL -> 0; PaintMode.STROKE -> 1; else -> 2 },
    strokeWidth = this.strokeWidth,
    antiAlias   = this.isAntiAlias,
    alpha       = this.alpha.toByte())
```

---

### 5c. IF MISSING: `Path.wasmWasi.kt`

```kotlin
// skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/Path.wasmWasi.kt
package org.jetbrains.skia

actual class Path {
    private val buf = mutableListOf<Byte>()
    actual fun moveTo(x: Float, y: Float) = emit(0x01, x, y)
    actual fun lineTo(x: Float, y: Float) = emit(0x02, x, y)
    actual fun cubicTo(x1: Float, y1: Float, x2: Float, y2: Float,
                       x3: Float, y3: Float) = emit(0x03, x1,y1,x2,y2,x3,y3)
    actual fun close()  { buf += 0x04.toByte() }
    actual fun reset()  { buf.clear() }
    internal fun serializeToBytes(): ByteArray = buf.toByteArray()
    private fun emit(tag: Byte, vararg fs: Float) {
        buf += tag
        for (f in fs) {
            val b = f.toBits()
            buf += ((b shr 24) and 0xff).toByte()
            buf += ((b shr 16) and 0xff).toByte()
            buf += ((b shr  8) and 0xff).toByte()
            buf += ( b         and 0xff).toByte()
        }
    }
}
```

---

### 5d. IF MISSING: `WasmMemory.kt`

```kotlin
// skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/WasmMemory.kt
package org.jetbrains.skiko.wasi

import kotlin.wasm.unsafe.Pointer
import kotlin.wasm.unsafe.withScopedMemoryAllocator

object WasmMemory {
    fun copyToLinear(bytes: ByteArray): Int {
        val ptr = kotlinx.wasm.unsafe.malloc(bytes.size)
        for (i in bytes.indices) Pointer(ptr.toLong() + i).storeByte(bytes[i])
        return ptr
    }
    fun free(ptr: Int) = kotlinx.wasm.unsafe.free(ptr)
}
```

---

### 5e. IF MISSING: `SkiaLayerWasi.kt`

```kotlin
// skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/SkiaLayerWasi.kt
package org.jetbrains.skiko

import org.jetbrains.skia.Canvas
import org.jetbrains.skiko.wasi.*

internal val wasiCanvas = Canvas(0, false, null)
var currentSkiaLayer: SkiaLayer? = null

actual class SkiaLayer {
    actual var renderDelegate: SkikoRenderDelegate? = null
    internal fun doFrame(nanos: Long) {
        witBeginFrame()
        val w = witSurfaceWidth()
        val h = witSurfaceHeight()
        renderDelegate?.onRender(wasiCanvas, w, h, nanos)
        witEndFrame()
    }
    actual fun needRedraw() {}
    actual fun dispose()    { currentSkiaLayer = null }
}

@WasmExport("render-frame")
fun exportedRenderFrame(nanos: Long)        { currentSkiaLayer?.doFrame(nanos) }

@WasmExport("on-pointer-event")
fun exportedOnPointerEvent(kind: Int, x: Float, y: Float) {
    val t = when(kind) {
        0 -> SkikoPointerEventKind.DOWN
        1 -> SkikoPointerEventKind.UP
        2 -> SkikoPointerEventKind.MOVE
        else -> SkikoPointerEventKind.SCROLL
    }
    currentSkiaLayer?.skikoView?.onPointerEvent(
        SkikoPointerEvent(x, y, t, SkikoMouseButtons.LEFT, emptyList()))
}

@WasmExport("on-key-event")
fun exportedOnKeyEvent(kind: Int, keyCode: Int) {
    val t = if (kind == 0) SkikoKeyboardEventKind.DOWN else SkikoKeyboardEventKind.UP
    currentSkiaLayer?.skikoView?.onKeyboardEvent(SkikoKeyboardEvent(keyCode, t))
}

@WasmExport("on-resize")
fun exportedOnResize(w: Int, h: Int)        { currentSkiaLayer?.doFrame(System.nanoTime()) }
```

---

## Verify

### Compile the wasmWasi target

```bash
cd wasm-android-runtime/skiko
./gradlew :skiko:wasmWasiJar 2>&1 | tail -30
```

**Expected:** `BUILD SUCCESSFUL` — produces a `.wasm` file under:
```
skiko/build/compileSync/wasmWasi/main/productionLibrary/kotlin/skiko.wasm
```

Confirm the file exists:
```bash
ls -lh skiko/build/compileSync/wasmWasi/main/productionLibrary/kotlin/skiko.wasm
# Expected: file exists (may be empty/stub until Compose app is added in Task 04)
```

Check expected exports exist:
```bash
wasm-tools dump \
  skiko/build/compileSync/wasmWasi/main/productionLibrary/kotlin/skiko.wasm \
  | grep -E "export|render-frame|on-pointer"
# Expected: render-frame, on-pointer-event, on-key-event, on-resize appear
```

### ✅ Checkpoint — write this after all checks pass

```bash
cat > wasm-android-runtime/.task-state << 'EOF'
TASK=03
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 03 verified OK — Skiko wasmWasi builds, exports render-frame and input functions
NOTES=
EOF
```

---

## Known issues

### `@WasmImport` / `@WasmExport` not resolved

These annotations are in the cm-prototype Kotlin compiler only.
Verify with:
```bash
find ~/.m2 -name "kotlin-stdlib-wasm-wasi-*.jar" | head -3
# Should find jars from the cm-prototype publish
```
If empty, the publishToMavenLocal step failed — rerun it.

### `expect` class has no actual for `wasmWasi`

Skiko has many `expect` declarations in `commonMain`. You will see errors like:
```
error: Expected class 'Canvas' has no actual for module 'skiko.wasmWasi'
```
Add each missing actual one at a time. Start with `Canvas`, `Paint`, `Rect`,
`Path` — these cover ~90% of what Compose needs. Comment out unresolved
expects with `// TODO` temporarily to unblock compilation.

### `kotlin.wasm.unsafe` package not found

The unsafe memory API package name changed between prototype versions.
Try:
- `kotlin.wasm.unsafe.withScopedMemoryAllocator`
- `kotlinx.wasm.unsafe.malloc`
- Check what's available: `jar tf ~/.m2/...kotlin-stdlib-wasm-wasi-*.jar | grep unsafe`

### `SkikoPointerEvent` constructor signature mismatch

Skiko's input event types may differ between versions. Check actual constructor
signatures in `skiko/src/commonMain/kotlin/org/jetbrains/skiko/SkikoInput.kt`
and adjust the `exportedOnPointerEvent` implementation accordingly.

## Do NOT

- **Do not clone a fresh Skiko.** Use your existing patched repo — cloning
  fresh and re-applying all patches takes days. The symlink approach in Step 1
  is the correct path.
- Do not modify `commonMain` Skiko code — only add/update `wasmWasiMain` actuals.
- Do not add any `import` of browser/JS APIs (`kotlinx.browser`, `org.w3c`) in wasmWasiMain.
- Do not depend on `webMain` source set from `wasmWasiMain`.
- Do not re-run the cm-prototype Kotlin `publishToMavenLocal` if it's already
  done — check `~/.m2` first.
