# Task 09 — Path + PathBuilder

> **Status: ✅ complete (verified 2026-05-15).** Implementation shipped as part of the working end-to-end Compose-on-WASM PoC. WIT entries, Rust host impl, and Kotlin wasmWasi stubs all in place. This file is kept as historical reference for the architectural decisions made during implementation.

## Goal

`Canvas.drawPath(path, paint)` and `Canvas.clipPath(path)` work.
`Path` and `PathBuilder` classes are available in the wasmWasiMain source set
with enough API coverage for Compose's typical use: moveTo, lineTo, cubicTo,
quadTo, arcTo, addRect, addRRect, addOval, close.

Done looks like: the test-app can draw a star polygon, a rounded-corner path,
and clip a region to an arbitrary path shape.

---

## Architecture decision: SVG path string serialization

Kotlin builds an SVG path string (`M x y L x1 y1 C x1 y1 x2 y2 x3 y3 Z ...`).
The string is passed as UTF-8 bytes across the WIT boundary.
Rust calls `skia_safe::Path::from_svg(svg_string)`.

Pros: no custom binary format, easy to debug (log the string), Skia already
has the parser. The WIT `draw-path` function already takes `list<u8>` — only
the semantic interpretation changes (bytes are now UTF-8 SVG, not binary).

---

## Steps

### 1. Verify WIT already has `draw-path` and `clip-path`

Task 08 added `clip-path` to the WIT file. Confirm both are present:

```bash
grep "draw-path\|clip-path" /home/harry/wasm-android-runtime/wit/skiko-gfx.wit
```

Expected:
```
    draw-path: func(path-data: list<u8>, paint: paint-attrs);
    clip-path: func(path-data: list<u8>, anti-alias: bool);
```

If missing, add them (see task 08 WIT template).

### 2. Implement `Path` class in `SkiaTypes.wasi.kt`

Add the full `Path` class after the existing type definitions:

```kotlin
class Path {
    private val sb = StringBuilder()
    private var _fillMode: PathFillMode = PathFillMode.WINDING

    var fillMode: PathFillMode
        get() = _fillMode
        set(v) { _fillMode = v }

    fun moveTo(x: Float, y: Float): Path    = apply { sb.append("M $x $y ") }
    fun rMoveTo(dx: Float, dy: Float): Path = apply { sb.append("m $dx $dy ") }
    fun lineTo(x: Float, y: Float): Path    = apply { sb.append("L $x $y ") }
    fun rLineTo(dx: Float, dy: Float): Path = apply { sb.append("l $dx $dy ") }
    fun quadTo(x1: Float, y1: Float, x2: Float, y2: Float): Path =
        apply { sb.append("Q $x1 $y1 $x2 $y2 ") }
    fun rQuadTo(dx1: Float, dy1: Float, dx2: Float, dy2: Float): Path =
        apply { sb.append("q $dx1 $dy1 $dx2 $dy2 ") }
    fun cubicTo(x1: Float, y1: Float, x2: Float, y2: Float, x3: Float, y3: Float): Path =
        apply { sb.append("C $x1 $y1 $x2 $y2 $x3 $y3 ") }
    fun rCubicTo(dx1: Float, dy1: Float, dx2: Float, dy2: Float, dx3: Float, dy3: Float): Path =
        apply { sb.append("c $dx1 $dy1 $dx2 $dy2 $dx3 $dy3 ") }
    fun close(): Path = apply { sb.append("Z ") }

    /** SVG arc command: A rx ry x-rotation large-arc-flag sweep-flag x y */
    fun arcTo(rx: Float, ry: Float, xAxisRotate: Float,
              largeArc: Boolean, sweep: Boolean,
              x: Float, y: Float): Path =
        apply {
            val la = if (largeArc) 1 else 0
            val sw = if (sweep) 1 else 0
            sb.append("A $rx $ry $xAxisRotate $la $sw $x $y ")
        }

    /** Add a closed rectangle contour. */
    fun addRect(r: Rect, dir: PathDirection = PathDirection.CLOCKWISE): Path = apply {
        if (dir == PathDirection.CLOCKWISE) {
            moveTo(r.left, r.top)
            lineTo(r.right, r.top)
            lineTo(r.right, r.bottom)
            lineTo(r.left, r.bottom)
        } else {
            moveTo(r.left, r.top)
            lineTo(r.left, r.bottom)
            lineTo(r.right, r.bottom)
            lineTo(r.right, r.top)
        }
        close()
    }

    /** Add a closed oval contour (approximated with 4 cubic bezier arcs). */
    fun addOval(r: Rect, dir: PathDirection = PathDirection.CLOCKWISE): Path = apply {
        val cx = r.left + r.width / 2f
        val cy = r.top  + r.height / 2f
        val rx = r.width / 2f
        val ry = r.height / 2f
        // Cubic bezier constant for approximating a circle arc
        val k = 0.5522847498f
        val kx = rx * k
        val ky = ry * k
        if (dir == PathDirection.CLOCKWISE) {
            moveTo(cx + rx, cy)
            cubicTo(cx + rx, cy - ky, cx + kx, cy - ry, cx, cy - ry)
            cubicTo(cx - kx, cy - ry, cx - rx, cy - ky, cx - rx, cy)
            cubicTo(cx - rx, cy + ky, cx - kx, cy + ry, cx, cy + ry)
            cubicTo(cx + kx, cy + ry, cx + rx, cy + ky, cx + rx, cy)
        } else {
            moveTo(cx + rx, cy)
            cubicTo(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry)
            cubicTo(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy)
            cubicTo(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry)
            cubicTo(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy)
        }
        close()
    }

    /** Add a rounded-rectangle contour (approximated with cubic bezier arcs). */
    fun addRRect(r: RRect, dir: PathDirection = PathDirection.CLOCKWISE): Path = apply {
        val rx = if (r.radii.isNotEmpty()) r.radii[0] else 0f
        val ry = if (r.radii.size >= 2) r.radii[1] else rx
        if (rx == 0f && ry == 0f) return addRect(
            Rect.makeLTRB(r.left, r.top, r.right, r.bottom), dir)
        val k = 0.5522847498f
        val kx = rx * k
        val ky = ry * k
        // top-left → clockwise
        moveTo(r.left + rx, r.top)
        lineTo(r.right - rx, r.top)
        cubicTo(r.right - rx + kx, r.top, r.right, r.top + ry - ky, r.right, r.top + ry)
        lineTo(r.right, r.bottom - ry)
        cubicTo(r.right, r.bottom - ry + ky, r.right - rx + kx, r.bottom, r.right - rx, r.bottom)
        lineTo(r.left + rx, r.bottom)
        cubicTo(r.left + rx - kx, r.bottom, r.left, r.bottom - ry + ky, r.left, r.bottom - ry)
        lineTo(r.left, r.top + ry)
        cubicTo(r.left, r.top + ry - ky, r.left + rx - kx, r.top, r.left + rx, r.top)
        close()
    }

    fun addPath(src: Path): Path = apply { sb.append(src.sb) }

    fun reset(): Path = apply { sb.clear() }

    /** Returns the serialized SVG path string bytes for WIT transport. */
    internal fun toSvgBytes(): ByteArray = sb.toString().trimEnd().encodeToByteArray()

    companion object {
        fun makeFromSVGString(svg: String): Path = Path().also { it.sb.append(svg) }
    }
}

enum class PathDirection   { CLOCKWISE, COUNTER_CLOCKWISE }
enum class PathFillMode    { WINDING, EVEN_ODD }
```

### 3. Implement `PathBuilder` in `SkiaTypes.wasi.kt`

```kotlin
class PathBuilder {
    private val path = Path()

    fun setFillType(fillType: PathFillMode): PathBuilder = apply { path.fillMode = fillType }
    fun moveTo(x: Float, y: Float): PathBuilder           = apply { path.moveTo(x, y) }
    fun moveTo(p: Point): PathBuilder                     = moveTo(p.x, p.y)
    fun lineTo(x: Float, y: Float): PathBuilder           = apply { path.lineTo(x, y) }
    fun lineTo(p: Point): PathBuilder                     = lineTo(p.x, p.y)
    fun quadTo(x1: Float, y1: Float, x2: Float, y2: Float): PathBuilder =
        apply { path.quadTo(x1, y1, x2, y2) }
    fun cubicTo(x1: Float, y1: Float, x2: Float, y2: Float, x3: Float, y3: Float): PathBuilder =
        apply { path.cubicTo(x1, y1, x2, y2, x3, y3) }
    fun arcTo(oval: Rect, startAngle: Float, sweepAngle: Float, forceMoveTo: Boolean): PathBuilder =
        apply { path.addArcTo(oval, startAngle, sweepAngle, forceMoveTo) }
    fun addRect(rect: Rect, dir: PathDirection = PathDirection.CLOCKWISE): PathBuilder =
        apply { path.addRect(rect, dir) }
    fun addOval(oval: Rect, dir: PathDirection = PathDirection.CLOCKWISE): PathBuilder =
        apply { path.addOval(oval, dir) }
    fun addRRect(rrect: RRect, dir: PathDirection = PathDirection.CLOCKWISE): PathBuilder =
        apply { path.addRRect(rrect, dir) }
    fun closePath(): PathBuilder = apply { path.close() }

    fun snapshot(): Path = Path().also { it.addPath(path) }
    fun detach(): Path { val p = path; return p }
    fun build(): Path = snapshot()
}
```

Also add a `addArcTo` helper to `Path` (used by PathBuilder above):

```kotlin
// Inside Path class:
fun addArcTo(oval: Rect, startAngle: Float, sweepAngle: Float, forceMoveTo: Boolean): Path = apply {
    // Decompose arc into cubic bezier segments using the oval approach.
    // Skia uses a cubic approximation; here we emit an SVG arc which Skia parses exactly.
    val endAngle = startAngle + sweepAngle
    val cx = oval.left + oval.width / 2f
    val cy = oval.top  + oval.height / 2f
    val rx = oval.width / 2f
    val ry = oval.height / 2f
    val startRad = startAngle * (PI.toFloat() / 180f)
    val endRad   = endAngle   * (PI.toFloat() / 180f)
    val x1 = cx + rx * kotlin.math.cos(startRad)
    val y1 = cy + ry * kotlin.math.sin(startRad)
    val x2 = cx + rx * kotlin.math.cos(endRad)
    val y2 = cy + ry * kotlin.math.sin(endRad)
    val largeArc = kotlin.math.abs(sweepAngle) > 180f
    val sweep    = sweepAngle > 0f
    if (forceMoveTo || sb.isEmpty()) moveTo(x1, y1) else lineTo(x1, y1)
    arcTo(rx, ry, 0f, largeArc, sweep, x2, y2)
}
```

Add `import kotlin.math.PI` and `import kotlin.math.cos`/`sin` at the top of the file if not already present.

### 4. Update `WasiCanvas.kt` — add `drawPath` and `clipPath` overrides

```kotlin
fun drawPath(path: Path, paint: Paint): Canvas {
    WitCanvas.Import.drawPath(path.toSvgBytes(), paint.witAttrs())
    return this
}

override fun clipPath(p: Path, mode: ClipMode, antiAlias: Boolean): Canvas {
    WitCanvas.Import.clipPath(p.toSvgBytes(), antiAlias)
    return this
}
```

### 5. Update `canvas_impl.rs` — implement `draw_path` and `clip_path`

Replace the existing `draw_path` stub and implement `clip_path`:

```rust
fn draw_path(&mut self, path_data: Vec<u8>, paint: PaintAttrs) {
    if let Ok(svg) = std::str::from_utf8(&path_data) {
        if let Some(path) = skia_safe::Path::from_svg(svg) {
            self.renderer.canvas().draw_path(&path, &make_paint(&paint));
        } else {
            log::warn!("draw_path: failed to parse SVG: {}", &svg[..svg.len().min(60)]);
        }
    }
}

fn clip_path(&mut self, path_data: Vec<u8>, anti_alias: bool) {
    if let Ok(svg) = std::str::from_utf8(&path_data) {
        if let Some(path) = skia_safe::Path::from_svg(svg) {
            self.renderer.canvas().clip_path(&path, None, anti_alias);
        }
    }
}
```

### 6. Add `Point` type to `SkiaTypes.wasi.kt` if not present

```kotlin
data class Point(val x: Float, val y: Float) {
    companion object {
        val ZERO = Point(0f, 0f)
    }
}
```

### 7. Add path test to `Main.kt`

```kotlin
// ── Section: Path (task 09) ───────────────────────────────────────────────
val t9Top = t8Top + sp(100f)
canvas.drawString("task 09: drawPath / clipPath",
    margin, t9Top, Font(size = sp(11f)),
    Paint().apply { color = 0xFF94A3B8.toInt() })

// Star polygon via Path
val starPath = Path()
val cx09 = margin + sp(36f)
val cy09 = t9Top + sp(50f)
val outer = sp(28f)
val inner = sp(12f)
for (i in 0..9) {
    val angle = -PI.toFloat() / 2f + i * PI.toFloat() / 5f
    val r = if (i % 2 == 0) outer else inner
    val x = cx09 + r * kotlin.math.cos(angle)
    val y = cy09 + r * kotlin.math.sin(angle)
    if (i == 0) starPath.moveTo(x, y) else starPath.lineTo(x, y)
}
starPath.close()
canvas.drawPath(starPath, Paint().apply {
    color = 0xFFFFD700.toInt(); isAntiAlias = true
})

// clipPath to oval
val clipPath = Path()
clipPath.addOval(Rect.makeXYWH(margin + sp(80f), t9Top + sp(18f), sp(70f), sp(56f)))
canvas.save()
canvas.clipPath(clipPath, ClipMode.INTERSECT, true)
canvas.drawPaint(Paint().apply { color = 0xFF0F3460.toInt() })
canvas.drawString("clipped text", margin + sp(82f), t9Top + sp(50f),
    Font(size = sp(11f)), Paint().apply { color = 0xFF00D4FF.toInt() })
canvas.restore()

// RRect path via PathBuilder
val pb = PathBuilder()
pb.addRRect(RRect.makeXYWH(margin + sp(160f), t9Top + sp(18f), sp(80f), sp(56f), sp(16f)))
canvas.drawPath(pb.build(), Paint().apply {
    color = 0xFFE94560.toInt(); mode = PaintMode.STROKE
    strokeWidth = sp(3f); isAntiAlias = true
})
```

### 8. Build and test

```bash
cd /home/harry/skiko
./gradlew :skiko:wasmWasiJar --console=plain --no-daemon 2>&1 | tail -5
./gradlew :test-app:compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon 2>&1 | tail -10
```

Then run the full build pipeline (see CLAUDE.md) and push to device.

---

## Verify

```bash
adb shell am force-stop com.example.wasmruntime
adb logcat -c
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
sleep 6
adb logcat -d | grep -E "(draw_path|clip_path|parse SVG|render_frame #[0-4]|fatal)"
```

Expected:
- No `failed to parse SVG` warnings
- `render_frame #0: ... ok=true`
- Device shows star polygon, oval clip, stroked RRect path

### ✅ Checkpoint

```bash
cat > .task-state << 'EOF'
TASK=09
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 09 verified OK — drawPath/clipPath work with SVG serialization
NOTES=
EOF
```

---

## Known issues

### `Path::from_svg` returns `None` for empty string

Guard on the Kotlin side: if `sb.isEmpty()`, don't call `drawPath`.
Add a `val isEmpty: Boolean get() = sb.isEmpty()` property to `Path`.

### SVG arc approximation precision

The `addArcTo` helper converts degrees→radians→endpoint coordinates. Floating
point errors can make arcs slightly off. If arc accuracy matters, use the cubic
bezier decomposition approach instead (see Skia's SkPathBuilder source).

### `from_svg` and relative commands

Skia's SVG parser supports both uppercase (absolute) and lowercase (relative)
SVG path commands. The Path class emits lowercase `m`, `l`, `c`, `q` for
relative variants — this is valid SVG and Skia handles it correctly.

### Performance: many small paths

Each `drawPath` call encodes the entire SVG string as UTF-8 and passes it
across the WIT boundary. For paths called every frame (e.g., in animations),
this is fine for paths under ~1KB. For very large paths (100+ segments),
consider caching the path as a WIT resource (future optimization).

---

## Do NOT

- Store the SVG string in the host and re-use across frames — the Kotlin side
  owns the path lifetime, there's no caching on the host side.
- Implement `PathMeasure` — it requires native Skia and is not needed for Compose MVP.
- Implement `Path.op()` (boolean ops) — deferred to a later task.
