# Task 11 — Gradient Shaders

> **Status: ✅ complete (verified 2026-05-15).** Implementation shipped as part of the working end-to-end Compose-on-WASM PoC. WIT entries, Rust host impl, and Kotlin wasmWasi stubs all in place. This file is kept as historical reference for the architectural decisions made during implementation.

## Goal

`Shader.makeLinearGradient` and `Shader.makeRadialGradient` work.
`Paint.shader` can be set and the gradient is applied when drawing.
`FilterTileMode` (clamp/repeat/mirror/decal) is respected.

Done looks like: the test-app renders a linear-gradient filled rectangle and
a radial-gradient filled circle, both with multiple color stops.

---

## Architecture

Shaders are host-side resources identified by a `u32` handle. The WIT functions
`create-linear-gradient` / `create-radial-gradient` return handles.
`paint-attrs` carries a `shader-id: u32` (0 = no shader).
`drop-shader` frees the host resource.

The WIT declarations were already added in task 08. This task implements the
Kotlin and Rust sides.

---

## Steps

### 1. Verify WIT already has shader functions (added in task 08)

```bash
grep "gradient\|shader" /home/harry/wasm-android-runtime/wit/skiko-gfx.wit
```

Expected output includes:
```
    create-linear-gradient: func(...)  -> u32;
    create-radial-gradient: func(...) -> u32;
    drop-shader: func(id: u32);
```

If missing, add them from the task 08 WIT template before continuing.

### 2. Extend `paint-attrs` in WIT to carry shader ID

The `paint-attrs` record needs a `shader-id` field. Update the record in
`wit/skiko-gfx.wit`:

```wit
    record paint-attrs {
        color:        u32,
        style:        paint-style,
        stroke-width: f32,
        stroke-miter: f32,
        stroke-cap:   stroke-cap,
        stroke-join:  stroke-join,
        anti-alias:   bool,
        alpha:        u8,
        blend-mode:   blend-mode,
        shader-id:    u32,    // NEW — 0 means no shader
    }
```

Sync to skiko repo:

```bash
cp /home/harry/wasm-android-runtime/wit/skiko-gfx.wit \
   /home/harry/skiko/skiko/wit/skiko-gfx.wit
```

### 3. Update Kotlin `PaintAttrs` record in `SkikoUi.kt`

Add `shaderId: UInt = 0u` as the last field of the `PaintAttrs` data class.
Update the `@WasmImport` declarations in `InternalSkikoUi.kt` to add the
`shader-id` parameter.

### 4. Add `FilterTileMode` enum and `Shader` class to `SkiaTypes.wasi.kt`

```kotlin
enum class FilterTileMode { CLAMP, REPEAT, MIRROR, DECAL }

class Shader internal constructor(internal val id: UInt) {
    fun close() {
        if (id != 0u) WitCanvas.Import.dropShader(id)
    }

    companion object {
        fun makeLinearGradient(
            p0: Point, p1: Point,
            colors: IntArray,
            stops: FloatArray? = null,
            tileMode: FilterTileMode = FilterTileMode.CLAMP,
        ): Shader {
            val colorsU = colors.map { it.toUInt() }
            val stopsF  = stops?.toList() ?: List(colors.size) { i -> i.toFloat() / (colors.size - 1).coerceAtLeast(1) }
            val tm = tileMode.ordinal.toByte()
            val id = WitCanvas.Import.createLinearGradient(
                p0.x, p0.y, p1.x, p1.y, colorsU, stopsF, tm)
            return Shader(id)
        }

        fun makeRadialGradient(
            center: Point,
            radius: Float,
            colors: IntArray,
            stops: FloatArray? = null,
            tileMode: FilterTileMode = FilterTileMode.CLAMP,
        ): Shader {
            val colorsU = colors.map { it.toUInt() }
            val stopsF  = stops?.toList() ?: List(colors.size) { i -> i.toFloat() / (colors.size - 1).coerceAtLeast(1) }
            val tm = tileMode.ordinal.toByte()
            val id = WitCanvas.Import.createRadialGradient(
                center.x, center.y, radius, colorsU, stopsF, tm)
            return Shader(id)
        }

        /** Solid-color shader — use for tinting images (task 13). */
        fun makeColor(color: Int): Shader {
            // Linear gradient from one color to itself is effectively solid.
            val c = listOf(color.toUInt(), color.toUInt())
            val s = listOf(0f, 1f)
            val id = WitCanvas.Import.createLinearGradient(
                0f, 0f, 1f, 0f, c, s, FilterTileMode.CLAMP.ordinal.toByte())
            return Shader(id)
        }
    }
}
```

### 5. Update `Paint` class in `SkiaTypes.wasi.kt`

```kotlin
class Paint {
    var color:       Int            = 0xFF000000.toInt()
    var mode:        PaintMode      = PaintMode.FILL
    var strokeWidth: Float          = 0f
    var strokeMiter: Float          = 4f
    var strokeCap:   PaintStrokeCap  = PaintStrokeCap.BUTT
    var strokeJoin:  PaintStrokeJoin = PaintStrokeJoin.MITER
    var isAntiAlias: Boolean        = false
    var alpha:       Int            = 255
    var blendMode:   BlendMode      = BlendMode.SRC_OVER
    var shader:      Shader?        = null   // NEW

    fun apply(block: Paint.() -> Unit): Paint { block(); return this }
}
```

### 6. Update `WasiCanvas.witAttrs()` to include shader ID

```kotlin
private fun Paint.witAttrs(): WitCanvas.PaintAttrs {
    // ... existing fields ...
    return WitCanvas.PaintAttrs(
        color       = color.toUInt(),
        style       = styleVal,
        strokeWidth = strokeWidth,
        strokeMiter = strokeMiter,
        strokeCap   = capVal,
        strokeJoin  = joinVal,
        antiAlias   = isAntiAlias,
        alpha       = alpha.toUByte(),
        blendMode   = blendVal,
        shaderId    = shader?.id ?: 0u,   // NEW
    )
}
```

### 7. Update `canvas_impl.rs` — implement shader creation and application

Add shader storage to `SkiaRenderer`:

```rust
pub struct SkiaRenderer {
    // ... existing fields ...
    shader_cache: HashMap<u32, skia_safe::Shader>,
    next_shader_id: u32,
}
```

Initialize in `SkiaRenderer::new`:
```rust
shader_cache: HashMap::new(),
next_shader_id: 1,
```

Implement `create_linear_gradient` and `create_radial_gradient` (replacing the stubs from task 08):

```rust
fn create_linear_gradient(&mut self,
    x0: f32, y0: f32, x1: f32, y1: f32,
    colors: Vec<u32>, stops: Vec<f32>, tile_mode: u8,
) -> u32 {
    use skia_safe::{Point, gradient_shader, TileMode};
    let pts  = [Point::new(x0, y0), Point::new(x1, y1)];
    let cols: Vec<skia_safe::Color> = colors.iter()
        .map(|&c| skia_safe::Color::from_argb(
            (c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8))
        .collect();
    let stops_opt: Option<&[f32]> = if stops.is_empty() { None } else { Some(&stops) };
    let mode = tile_mode_from_u8(tile_mode);
    let shader = gradient_shader::linear(&pts, cols.as_slice(), stops_opt, mode, None, None);
    if let Some(s) = shader {
        let id = self.renderer.next_shader_id;
        self.renderer.next_shader_id += 1;
        self.renderer.shader_cache.insert(id, s);
        id
    } else {
        log::warn!("create_linear_gradient: gradient_shader::linear returned None");
        0
    }
}

fn create_radial_gradient(&mut self,
    cx: f32, cy: f32, radius: f32,
    colors: Vec<u32>, stops: Vec<f32>, tile_mode: u8,
) -> u32 {
    use skia_safe::{Point, gradient_shader, TileMode};
    let center = Point::new(cx, cy);
    let cols: Vec<skia_safe::Color> = colors.iter()
        .map(|&c| skia_safe::Color::from_argb(
            (c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8))
        .collect();
    let stops_opt: Option<&[f32]> = if stops.is_empty() { None } else { Some(&stops) };
    let mode = tile_mode_from_u8(tile_mode);
    let shader = gradient_shader::radial(center, radius, cols.as_slice(), stops_opt, mode, None, None);
    if let Some(s) = shader {
        let id = self.renderer.next_shader_id;
        self.renderer.next_shader_id += 1;
        self.renderer.shader_cache.insert(id, s);
        id
    } else {
        0
    }
}

fn drop_shader(&mut self, id: u32) {
    self.renderer.shader_cache.remove(&id);
}
```

Add helper function:

```rust
fn tile_mode_from_u8(v: u8) -> skia_safe::TileMode {
    match v {
        1 => skia_safe::TileMode::Repeat,
        2 => skia_safe::TileMode::Mirror,
        3 => skia_safe::TileMode::Decal,
        _ => skia_safe::TileMode::Clamp,
    }
}
```

Update `make_paint` to apply shader from the cache:

```rust
fn make_paint_with_renderer(attrs: &PaintAttrs, renderer: &SkiaRenderer) -> skia_safe::Paint {
    let mut p = make_paint(attrs);  // existing function for basic fields
    if attrs.shader_id != 0 {
        if let Some(shader) = renderer.shader_cache.get(&attrs.shader_id) {
            p.set_shader(Some(shader.clone()));
        }
    }
    p
}
```

Update all `draw_*` functions that call `make_paint` to use
`make_paint_with_renderer(attrs, &self.renderer)` instead.

> Because `make_paint_with_renderer` borrows `self.renderer` immutably while
> `self.renderer.canvas()` borrows it mutably, you may need to extract the
> paint first, then call the canvas method. Example pattern:
> ```rust
> fn draw_rect(&mut self, x: f32, y: f32, w: f32, h: f32, paint: PaintAttrs) {
>     let p = make_paint_with_renderer(&paint, &self.renderer);
>     let r = skia_safe::Rect::from_xywh(x, y, w, h);
>     self.renderer.canvas().draw_rect(r, &p);
> }
> ```

### 8. Add test to `Main.kt`

```kotlin
// ── Section: Gradient shaders (task 11) ──────────────────────────────────
val t11Top = t10Top + sp(60f)
canvas.drawString("task 11: linear + radial gradient",
    margin, t11Top, Font(size = sp(11f)),
    Paint().apply { color = 0xFF94A3B8.toInt() })

// Linear gradient rect
val linearShader = Shader.makeLinearGradient(
    Point(margin, t11Top + sp(18f)),
    Point(margin + sp(140f), t11Top + sp(18f)),
    intArrayOf(0xFF00D4FF.toInt(), 0xFFE94560.toInt(), 0xFF533483.toInt()),
    floatArrayOf(0f, 0.5f, 1f),
)
canvas.drawRect(
    Rect.makeXYWH(margin, t11Top + sp(18f), sp(140f), sp(40f)),
    Paint().apply { shader = linearShader; isAntiAlias = true }
)
linearShader.close()

// Radial gradient oval
val cx11 = margin + sp(200f)
val cy11 = t11Top + sp(38f)
val radialShader = Shader.makeRadialGradient(
    Point(cx11, cy11), sp(36f),
    intArrayOf(0xFFFFFFFF.toInt(), 0xFF00D4FF.toInt(), 0x00000000.toInt()),
)
canvas.drawOval(
    Rect.makeXYWH(cx11 - sp(36f), cy11 - sp(24f), sp(72f), sp(48f)),
    Paint().apply { shader = radialShader; isAntiAlias = true }
)
radialShader.close()
```

### 9. Build and test

```bash
cd /home/harry/skiko
./gradlew :skiko:wasmWasiJar --console=plain --no-daemon 2>&1 | tail -5
./gradlew :test-app:compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon 2>&1 | tail -10
```

Then run the full pipeline and push.

---

## Verify

```bash
adb shell am force-stop com.example.wasmruntime
adb logcat -c
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
sleep 6
adb logcat -d | grep -E "(gradient|shader|render_frame #[0-4]|fatal)"
```

Expected:
- No `gradient_shader::linear returned None` warnings
- `render_frame #0: ... ok=true`
- Device shows horizontal rainbow gradient rect and radial white-to-cyan oval

### ✅ Checkpoint

```bash
cat > .task-state << 'EOF'
TASK=11
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 11 verified OK — linear and radial gradients render on device
NOTES=
EOF
```

---

## Known issues

### `gradient_shader::linear` returns `None`

This happens when `colors.len() < 2`. Guard on the Kotlin side in `makeLinearGradient`:
if only one color is provided, duplicate it.

### Shader not applied to text blobs

The CPU rasterize path for text uses a separate surface. To apply a shader to
text, the shader would need to be applied to the CPU surface before blitting —
complex and rarely needed. Skip for now; text uses `color` only.

### Borrow checker: `make_paint_with_renderer` + `canvas()`

The Rust borrow checker may reject `self.renderer.canvas()` after
`&self.renderer` is borrowed. Fix by cloning the paint before calling canvas:

```rust
let p = {
    let mut p = make_paint(attrs);
    if attrs.shader_id != 0 {
        if let Some(s) = self.renderer.shader_cache.get(&attrs.shader_id) {
            p.set_shader(Some(s.clone()));
        }
    }
    p
};
self.renderer.canvas().draw_rect(r, &p);
```

---

## Do NOT

- Implement `Shader.makeBlend` or `Shader.makeFractalNoise` — not needed for Compose MVP.
- Keep shader resources alive across frames without calling `close()` — the
  `HashMap` will grow unbounded if shaders are created but never dropped.
- Use shader IDs > `u32::MAX / 2` — the host doesn't prevent integer overflow
  in `next_shader_id` but it's not a real concern for typical usage.
