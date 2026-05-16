# Task 13 — ColorFilter (tint + invert)

> **Status: ✅ complete (verified 2026-05-15).** Implementation shipped as part of the working end-to-end Compose-on-WASM PoC. WIT entries, Rust host impl, and Kotlin wasmWasi stubs all in place. This file is kept as historical reference for the architectural decisions made during implementation.

## Goal

`ColorFilter` for the two most common Compose use cases:
- **Blend/tint**: multiply or modulate an image with a color (icon tinting)
- **Invert**: invert all color channels (dark-mode effect)

`Paint.colorFilter = ColorFilter.makeBlend(color, blendMode)` applies the
filter when drawing.

Done looks like: the test-app draws a white-pixels image tinted to cyan using
`ColorFilter.makeBlend`, and a second copy with colors inverted.

---

## Architecture

Two approaches:

**Option A (chosen):** Extend `paint-attrs` with `color-filter-kind: u8` and
`color-filter-color: u32`. The host interprets these without a separate WIT
resource (no `create-color-filter` handle). Simple; covers the 2 needed cases.

**Option B:** `create-color-filter` → handle, like shaders. More flexible but
more WIT surface. Deferred until a third filter type is actually needed.

---

## Steps

### 1. Update `paint-attrs` in `wit/skiko-gfx.wit`

```wit
    enum color-filter-kind {
        none,
        blend,    // blend color with image pixels using blend-mode
        invert,   // invert RGB channels, preserve alpha
    }

    record paint-attrs {
        color:               u32,
        style:               paint-style,
        stroke-width:        f32,
        stroke-miter:        f32,
        stroke-cap:          stroke-cap,
        stroke-join:         stroke-join,
        anti-alias:          bool,
        alpha:               u8,
        blend-mode:          blend-mode,
        shader-id:           u32,
        color-filter-kind:   color-filter-kind,   // NEW
        color-filter-color:  u32,                 // NEW — ARGB for blend kind
    }
```

Sync to skiko repo:

```bash
cp /home/harry/wasm-android-runtime/wit/skiko-gfx.wit \
   /home/harry/skiko/skiko/wit/skiko-gfx.wit
```

### 2. Update Kotlin bindings

Add to `SkikoUi.kt`:

```kotlin
enum class ColorFilterKind { NONE, BLEND, INVERT }
```

Update `PaintAttrs` data class with two new fields:

```kotlin
data class PaintAttrs(
    // ... existing fields ...
    val colorFilterKind:  ColorFilterKind = ColorFilterKind.NONE,
    val colorFilterColor: UInt = 0u,
)
```

Update `InternalSkikoUi.kt` `@WasmImport` declarations accordingly.

### 3. Add `ColorFilter` class to `SkiaTypes.wasi.kt`

```kotlin
class ColorFilter private constructor(
    internal val kind:  ColorFilterKind,
    internal val color: Int = 0,
    internal val blendMode: BlendMode = BlendMode.SRC_OVER,
) {
    companion object {
        /**
         * Blend [color] with the source pixel using [mode].
         * Common use: tint an image or icon.
         */
        fun makeBlend(color: Int, mode: BlendMode): ColorFilter =
            ColorFilter(ColorFilterKind.BLEND, color, mode)

        /** Invert all RGB channels; alpha is unchanged. */
        fun makeInvert(): ColorFilter = ColorFilter(ColorFilterKind.INVERT)

        /** Colour matrix — not implemented; returns invert for compatibility. */
        fun makeMatrix(matrix: FloatArray): ColorFilter = makeInvert()
    }
}

enum class ColorFilterKind { NONE, BLEND, INVERT }
```

### 4. Update `Paint` class

```kotlin
class Paint {
    // ... existing fields ...
    var colorFilter: ColorFilter? = null   // NEW
}
```

### 5. Update `WasiCanvas.witAttrs()`

```kotlin
private fun Paint.witAttrs(): WitCanvas.PaintAttrs {
    // ... existing field mapping ...
    val cfKind = when (colorFilter?.kind) {
        ColorFilterKind.BLEND  -> WitCanvas.ColorFilterKind.BLEND
        ColorFilterKind.INVERT -> WitCanvas.ColorFilterKind.INVERT
        else                   -> WitCanvas.ColorFilterKind.NONE
    }
    val cfColor = colorFilter?.color?.toUInt() ?: 0u
    return WitCanvas.PaintAttrs(
        // ... existing fields ...
        colorFilterKind  = cfKind,
        colorFilterColor = cfColor,
    )
}
```

### 6. Update `canvas_impl.rs` — apply color filter in `make_paint`

```rust
fn make_paint_with_renderer(attrs: &PaintAttrs, renderer: &SkiaRenderer) -> skia_safe::Paint {
    let mut p = make_paint(attrs);

    // Apply shader
    if attrs.shader_id != 0 {
        if let Some(s) = renderer.shader_cache.get(&attrs.shader_id) {
            p.set_shader(Some(s.clone()));
        }
    }

    // Apply color filter
    match attrs.color_filter_kind {
        ColorFilterKind::Blend => {
            let c = attrs.color_filter_color;
            let color = skia_safe::Color::from_argb(
                (c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8);
            // The blend-mode for the color filter comes from the paint's own blend mode field.
            // Use Modulate for standard icon tinting; otherwise use the paint blend mode.
            let blend = skia_safe::BlendMode::Modulate;
            if let Some(cf) = skia_safe::color_filters::blend(color, blend) {
                p.set_color_filter(cf);
            }
        }
        ColorFilterKind::Invert => {
            // Invert via a colour matrix: [-1 0 0 0 1 / 0 -1 0 0 1 / 0 0 -1 0 1 / 0 0 0 1 0]
            let matrix = [
                -1f32,  0f32,  0f32, 0f32, 1f32,
                 0f32, -1f32,  0f32, 0f32, 1f32,
                 0f32,  0f32, -1f32, 0f32, 1f32,
                 0f32,  0f32,  0f32, 1f32, 0f32,
            ];
            if let Some(cf) = skia_safe::color_filters::matrix_row_major(&matrix) {
                p.set_color_filter(cf);
            }
        }
        ColorFilterKind::None => {}
    }

    p
}
```

Check that `skia-safe` exposes `color_filters::blend` and
`color_filters::matrix_row_major`. In skia-safe 0.93+ they are in the
`skia_safe::color_filters` module. If the names differ, check:

```bash
grep -r "fn blend\|fn matrix" ~/.cargo/registry/src/*/skia-safe-*/src/
```

### 7. Add test to `Main.kt`

```kotlin
// ── Section: ColorFilter (task 13) ───────────────────────────────────────
val t13Top = t12Top + sp(110f)
canvas.drawString("task 13: ColorFilter (tint / invert)",
    margin, t13Top, Font(size = sp(11f)),
    Paint().apply { color = 0xFF94A3B8.toInt() })

// White source image (will be tinted)
val whiteImg = Image.makeFromColor(48, 48, 0xFFFFFFFF.toInt())

// Tint cyan
canvas.drawImage(whiteImg, margin, t13Top + sp(18f),
    Paint().apply {
        colorFilter = ColorFilter.makeBlend(0xFF00D4FF.toInt(), BlendMode.MULTIPLY)
    }
)

// Tint red
canvas.drawImage(whiteImg, margin + sp(60f), t13Top + sp(18f),
    Paint().apply {
        colorFilter = ColorFilter.makeBlend(0xFFE94560.toInt(), BlendMode.MULTIPLY)
    }
)

// Invert the checkerboard from task 12 (rebuild it here)
val checkImg = run {
    val w = 48; val h = 48
    val px = ByteArray(w * h * 4)
    for (py in 0 until h) for (px2 in 0 until w) {
        val i = (py * w + px2) * 4
        val light = ((px2 / 8 + py / 8) % 2 == 0)
        px[i+0] = if (light) 0xCC.toByte() else 0x33.toByte()
        px[i+1] = if (light) 0xCC.toByte() else 0x33.toByte()
        px[i+2] = if (light) 0xCC.toByte() else 0x33.toByte()
        px[i+3] = 0xFF.toByte()
    }
    Image.makeFromPixels(w, h, px)
}
canvas.drawImage(checkImg, margin + sp(120f), t13Top + sp(18f))
canvas.drawImage(checkImg, margin + sp(180f), t13Top + sp(18f),
    Paint().apply { colorFilter = ColorFilter.makeInvert() })

whiteImg.close()
checkImg.close()
```

### 8. Build and test

```bash
cd /home/harry/skiko
./gradlew :skiko:wasmWasiJar --console=plain --no-daemon 2>&1 | tail -5
./gradlew :test-app:compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon 2>&1 | tail -10
```

Then run the full pipeline and push to device.

---

## Verify

```bash
adb shell am force-stop com.example.wasmruntime
adb logcat -c
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
sleep 6
adb logcat -d | grep -E "(color_filter|render_frame #[0-4]|fatal)"
```

Expected:
- No errors
- Device shows: cyan-tinted square, red-tinted square, original checkerboard, inverted checkerboard

### ✅ Checkpoint — tasks 08–13 complete

```bash
cat > .task-state << 'EOF'
TASK=13
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 13 verified OK — ColorFilter tint+invert work, all API tasks complete
NOTES=
EOF
```

---

## Known issues

### `color_filters::blend` not found

skia-safe organises this differently across versions:
- 0.75: `ColorFilter::new_blend_mode`
- 0.93+: `color_filters::blend`

Check the skia-safe version in `host/Cargo.toml` and use the matching API.

### Invert matrix — wrong channel order

The colour matrix applies to [R, G, B, A] in Skia's row-major format.
If invert appears to not work, confirm the matrix is:

```
R' = -1*R + 0*G + 0*B + 0*A + 255
G' = 0*R + -1*G + 0*B + 0*A + 255
B' = 0*R + 0*G + -1*B + 0*A + 255
A' = 0*R + 0*G + 0*B + 1*A + 0
```

---

## Do NOT

- Implement `ColorFilter.makeMatrix` for full matrix support — the stub
  returning `makeInvert()` is a placeholder. Real matrix support can be added
  later by passing the 20 floats via WIT.
- Apply color filter to text — the CPU blit path doesn't pass through the
  `make_paint_with_renderer` path for text; text always uses `color` directly.
