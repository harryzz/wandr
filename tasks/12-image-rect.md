# Task 12 — Image Rect Drawing + Image Kotlin Class

> **Status: ✅ complete (verified 2026-05-15).** Implementation shipped as part of the working end-to-end Compose-on-WASM PoC. WIT entries, Rust host impl, and Kotlin wasmWasi stubs all in place. This file is kept as historical reference for the architectural decisions made during implementation.

## Goal

`Image` is a first-class Kotlin type in wasmWasiMain.
`Canvas.drawImageRect(image, src, dst, paint)` works.
`Canvas.drawImage(image, left, top, paint)` works with a Paint argument.

Done looks like: the test-app uploads a small RGBA8888 pixel buffer as an
image, draws it at full size, then draws a cropped subregion into a different
destination rectangle.

---

## Architecture

The WIT `create-image` / `draw-image` / `drop-image` functions already exist.
`draw-image-rect` was added in task 08's WIT update.
This task adds the Kotlin `Image` wrapper class and updates `WasiCanvas` to use it.

---

## Steps

### 1. Verify WIT has `draw-image-rect` (added in task 08)

```bash
grep "draw-image-rect" /home/harry/wasm-android-runtime/wit/skiko-gfx.wit
```

If missing, add to the canvas interface:

```wit
    draw-image-rect: func(image-id: u32,
                          src-x: f32, src-y: f32, src-w: f32, src-h: f32,
                          dst-x: f32, dst-y: f32, dst-w: f32, dst-h: f32,
                          paint: paint-attrs);
```

Sync to skiko repo if changed.

### 2. Add `Image` class to `SkiaTypes.wasi.kt`

```kotlin
/**
 * Host-managed image resource. Create via [Image.makeFromPixels].
 * Must be closed when no longer needed to free the host-side texture.
 */
class Image private constructor(
    internal val id: UInt,
    val width:  Int,
    val height: Int,
) : AutoCloseable {

    override fun close() {
        if (id != 0u) WitCanvas.Import.dropImage(id)
    }

    companion object {
        /**
         * Upload raw RGBA8888 pixel data to the host.
         * [pixels] must be exactly [width] * [height] * 4 bytes.
         */
        fun makeFromPixels(width: Int, height: Int, pixels: ByteArray): Image {
            require(pixels.size == width * height * 4) {
                "pixels must be width*height*4 bytes, got ${pixels.size}"
            }
            val id = WitCanvas.Import.createImage(
                width.toUInt(), height.toUInt(), pixels.toList())
            return Image(id, width, height)
        }

        /** Convenience: create a solid-colour image for testing. */
        fun makeFromColor(width: Int, height: Int, argb: Int): Image {
            val a = (argb shr 24) and 0xFF
            val r = (argb shr 16) and 0xFF
            val g = (argb shr  8) and 0xFF
            val b =  argb         and 0xFF
            val pixels = ByteArray(width * height * 4) { i ->
                when (i % 4) {
                    0 -> r.toByte()
                    1 -> g.toByte()
                    2 -> b.toByte()
                    else -> a.toByte()
                }
            }
            return makeFromPixels(width, height, pixels)
        }
    }
}
```

### 3. Update `WasiCanvas.kt` — add image overloads

```kotlin
fun drawImage(image: Image, left: Float, top: Float): Canvas {
    WitCanvas.Import.drawImage(image.id, left, top, 255u)
    return this
}

fun drawImage(image: Image, left: Float, top: Float, paint: Paint?): Canvas {
    val alpha = paint?.alpha?.toUByte() ?: 255u
    // If paint has a shader or blend mode, use draw-image-rect with a full-image src
    if (paint != null && (paint.blendMode != BlendMode.SRC_OVER || paint.shader != null)) {
        WitCanvas.Import.drawImageRect(
            image.id,
            0f, 0f, image.width.toFloat(), image.height.toFloat(),
            left, top, image.width.toFloat(), image.height.toFloat(),
            paint.witAttrs()
        )
    } else {
        WitCanvas.Import.drawImage(image.id, left, top, alpha)
    }
    return this
}

fun drawImageRect(image: Image, dst: Rect): Canvas {
    WitCanvas.Import.drawImageRect(
        image.id,
        0f, 0f, image.width.toFloat(), image.height.toFloat(),
        dst.left, dst.top, dst.width, dst.height,
        Paint().witAttrs()
    )
    return this
}

fun drawImageRect(image: Image, dst: Rect, paint: Paint?): Canvas {
    WitCanvas.Import.drawImageRect(
        image.id,
        0f, 0f, image.width.toFloat(), image.height.toFloat(),
        dst.left, dst.top, dst.width, dst.height,
        paint?.witAttrs() ?: Paint().witAttrs()
    )
    return this
}

fun drawImageRect(image: Image, src: Rect, dst: Rect, paint: Paint? = null): Canvas {
    WitCanvas.Import.drawImageRect(
        image.id,
        src.left, src.top, src.width, src.height,
        dst.left, dst.top, dst.width, dst.height,
        paint?.witAttrs() ?: Paint().witAttrs()
    )
    return this
}
```

### 4. Update `canvas_impl.rs` — implement `draw_image_rect`

The stub was added in task 08. Replace it with the full implementation if the
stub is still a no-op:

```rust
fn draw_image_rect(&mut self, image_id: u32,
                   src_x: f32, src_y: f32, src_w: f32, src_h: f32,
                   dst_x: f32, dst_y: f32, dst_w: f32, dst_h: f32,
                   paint: PaintAttrs) {
    use skia_safe::{Rect, canvas::SrcRectConstraint};
    if let Some(img) = self.renderer.image_cache.get(&image_id) {
        let src = Rect::from_xywh(src_x, src_y, src_w, src_h);
        let dst = Rect::from_xywh(dst_x, dst_y, dst_w, dst_h);
        let p   = make_paint_with_renderer(&paint, &self.renderer);
        self.renderer.canvas().draw_image_rect(
            img.as_ref(),
            Some((&src, SrcRectConstraint::Fast)),
            dst,
            &p,
        );
    } else {
        log::warn!("draw_image_rect: unknown image id {}", image_id);
    }
}
```

Verify that `image_cache: HashMap<u32, skia_safe::Image>` exists in
`SkiaRenderer` and is populated by `create_image`.

### 5. Add test to `Main.kt`

```kotlin
// ── Section: Image (task 12) ──────────────────────────────────────────────
val t12Top = t11Top + sp(80f)
canvas.drawString("task 12: drawImage / drawImageRect",
    margin, t12Top, Font(size = sp(11f)),
    Paint().apply { color = 0xFF94A3B8.toInt() })

// Create a simple 4×4 checkerboard RGBA image
val imgW = 64; val imgH = 64
val pixels = ByteArray(imgW * imgH * 4)
for (py in 0 until imgH) {
    for (px in 0 until imgW) {
        val i = (py * imgW + px) * 4
        val light = ((px / 8 + py / 8) % 2 == 0)
        pixels[i + 0] = if (light) 0xFF.toByte() else 0x33.toByte() // R
        pixels[i + 1] = if (light) 0xFF.toByte() else 0x99.toByte() // G
        pixels[i + 2] = if (light) 0xFF.toByte() else 0xFF.toByte() // B
        pixels[i + 3] = 0xFF.toByte()                                // A
    }
}
val img = Image.makeFromPixels(imgW, imgH, pixels)

// Draw full image
canvas.drawImage(img, margin, t12Top + sp(18f))

// Draw just the top-left quadrant into a larger rect
canvas.drawImageRect(
    img,
    src = Rect.makeXYWH(0f, 0f, imgW / 2f, imgH / 2f),
    dst = Rect.makeXYWH(margin + sp(80f), t12Top + sp(18f), sp(80f), sp(80f)),
)

// Draw with semi-transparent paint
canvas.drawImage(img, margin + sp(180f), t12Top + sp(18f),
    Paint().apply { alpha = 128 })

img.close()
```

### 6. Build and test

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
adb logcat -d | grep -E "(draw_image|create_image|unknown image|render_frame #[0-4]|fatal)"
```

Expected:
- No `unknown image id` warnings
- `render_frame #0: ... ok=true`
- Device shows: checkerboard at normal size, top-left quadrant zoomed into a larger rect, semi-transparent version

### ✅ Checkpoint

```bash
cat > .task-state << 'EOF'
TASK=12
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 12 verified OK — drawImage/drawImageRect render correctly
NOTES=
EOF
```

---

## Known issues

### `draw_image_rect` — image not in cache

If `image_cache` stores `skia_safe::Image` as `Image` (not `Arc<Image>`),
cloning may be needed. Use `img.clone()` or wrap in `Arc`.

### Pixel format: RGBA vs BGRA

Skia's `Image::from_raster_data` expects RGBA8888 by default on most
platforms, but some Android builds expect BGRA. If the checkerboard appears
with swapped red/blue channels, swap R and B in the pixel array construction.

### Alpha pre-multiplication

Skia expects premultiplied alpha in `ImageInfo::new_n32_premul`. When
creating the image, either pre-multiply the pixels yourself or use
`ColorAlphaType::Unpremul` in the `ImageInfo`. The `create_image` host
implementation should specify the correct alpha type.

In `canvas_impl.rs`, check that `create_image` uses:
```rust
let info = skia_safe::ImageInfo::new(
    (width as i32, height as i32),
    skia_safe::ColorType::RGBA8888,
    skia_safe::AlphaType::Unpremul,
    None,
);
```

---

## Do NOT

- Implement `Image.makeFromEncoded` (PNG/JPEG decode) — that requires a codec
  and is not needed for Compose MVP. Pass raw RGBA pixels.
- Store images per-frame — create them once and reuse across frames.
  Call `close()` only when done with the image entirely.
