# Task 08 — Paint Completeness + Missing Primitives

> **Status: ✅ complete (verified 2026-05-15).** Implementation shipped as part of the working end-to-end Compose-on-WASM PoC. WIT entries, Rust host impl, and Kotlin wasmWasi stubs all in place. This file is kept as historical reference for the architectural decisions made during implementation.

## Goal

Extend the WIT interface and Kotlin/Rust implementations so that Paint has
full stroke and blend properties, and Canvas gains the primitives that Compose
uses most: `drawArc`, `drawPaint`, `drawDRRect`, `clipRRect`, `skew`, and
`concat` (3×3 matrix). No path or shader support yet — those are tasks 09/11.

Done looks like: the test-app can draw arcs, donuts, skewed shapes, and
semi-transparent layered fills using all blend modes, and clip to rounded
rectangles.

---

## Steps

### 1. Update `wit/skiko-gfx.wit`

Add new enums and extend `paint-attrs`. Add new canvas functions.

```wit
package my:skiko-gfx@0.1.0;

interface canvas {
    // ── paint enums ───────────────────────────────────────────────────────
    enum paint-style { fill, stroke, fill-and-stroke }

    enum blend-mode {
        src-over,   // default — normal alpha compositing
        src,        // replace destination
        dst-in,     // cut out by source alpha
        dst-out,
        src-atop,
        dst-atop,
        xor,
        multiply,
        screen,
        overlay,
        darken,
        lighten,
        color-dodge,
        color-burn,
        hard-light,
        soft-light,
        difference,
        exclusion,
        clear,
    }

    enum stroke-cap  { butt, round, square }
    enum stroke-join { miter, round, bevel }

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
    }

    // ── surface dimensions ────────────────────────────────────────────────
    surface-width:  func() -> u32;
    surface-height: func() -> u32;

    begin-frame: func();
    end-frame:   func();

    // ── transform stack ───────────────────────────────────────────────────
    save:             func();
    save-layer:       func(x: f32, y: f32, w: f32, h: f32, has-bounds: bool, alpha: u8);
    restore:          func();
    translate:        func(dx: f32, dy: f32);
    scale:            func(sx: f32, sy: f32);
    rotate:           func(degrees: f32);
    skew:             func(sx: f32, sy: f32);
    /// 3×3 matrix in row-major order: [a b c / d e f / g h i]
    concat:           func(a: f32, b: f32, c: f32,
                           d: f32, e: f32, f: f32,
                           g: f32, h: f32, i: f32);
    reset-matrix:     func();

    // ── clip ─────────────────────────────────────────────────────────────
    clip-rect:  func(x: f32, y: f32, w: f32, h: f32, anti-alias: bool);
    clip-rrect: func(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32, anti-alias: bool);

    // ── drawing ───────────────────────────────────────────────────────────
    clear:       func(argb: u32);
    draw-paint:  func(paint: paint-attrs);
    draw-rect:   func(x: f32, y: f32, w: f32, h: f32, paint: paint-attrs);
    draw-rrect:  func(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32, paint: paint-attrs);
    /// Donut shape: draw rrect with a rrect hole.
    draw-drrect: func(ox: f32, oy: f32, ow: f32, oh: f32, orx: f32, ory: f32,
                      ix: f32, iy: f32, iw: f32, ih: f32, irx: f32, iry: f32,
                      paint: paint-attrs);
    draw-oval:   func(x: f32, y: f32, w: f32, h: f32, paint: paint-attrs);
    draw-line:   func(x0: f32, y0: f32, x1: f32, y1: f32, paint: paint-attrs);
    draw-arc:    func(x: f32, y: f32, w: f32, h: f32,
                      start-angle: f32, sweep-angle: f32,
                      include-center: bool, paint: paint-attrs);

    /// Serialised path as UTF-8 SVG path string (M/L/C/Q/A/Z).
    draw-path: func(path-data: list<u8>, paint: paint-attrs);

    // ── text blobs ────────────────────────────────────────────────────────
    create-text-blob: func(text: list<u8>, font-family: list<u8>, size: f32, weight: u32, italic: bool) -> u32;
    draw-text-blob:   func(id: u32, x: f32, y: f32, paint: paint-attrs);
    drop-text-blob:   func(id: u32);

    // ── images ────────────────────────────────────────────────────────────
    create-image:     func(width: u32, height: u32, pixels: list<u8>) -> u32;
    draw-image:       func(image-id: u32, x: f32, y: f32, alpha: u8);
    draw-image-rect:  func(image-id: u32,
                           src-x: f32, src-y: f32, src-w: f32, src-h: f32,
                           dst-x: f32, dst-y: f32, dst-w: f32, dst-h: f32,
                           paint: paint-attrs);
    drop-image:       func(image-id: u32);

    // ── shaders (task 11) ─────────────────────────────────────────────────
    create-linear-gradient: func(x0: f32, y0: f32, x1: f32, y1: f32,
                                  colors: list<u32>, stops: list<f32>,
                                  tile-mode: u8) -> u32;
    create-radial-gradient: func(cx: f32, cy: f32, radius: f32,
                                  colors: list<u32>, stops: list<f32>,
                                  tile-mode: u8) -> u32;
    drop-shader: func(id: u32);

    // ── clip-path (task 09) ───────────────────────────────────────────────
    clip-path: func(path-data: list<u8>, anti-alias: bool);
}

interface renderer {
    enum pointer-kind { down, up, move, scroll }
    enum key-kind     { down, up }

    render-frame:     func(nanos: u64);
    on-pointer-event: func(kind: pointer-kind, x: f32, y: f32);
    on-key-event:     func(kind: key-kind, key-code: u32);
    on-resize:        func(w: u32, h: u32);
}

world skiko-ui {
    import canvas;
    export renderer;
}
```

> **Note:** The shader and clip-path functions are declared here so the WIT file
> is complete. Their Kotlin/Rust implementations land in tasks 09 and 11.
> Adding them now avoids a second bindgen regeneration later.

### 2. Sync WIT to skiko repo

```bash
cp /home/harry/wasm-android-runtime/wit/skiko-gfx.wit \
   /home/harry/skiko/skiko/wit/skiko-gfx.wit
```

### 3. Update Kotlin bindings — `generated/SkikoUi.kt`

Add new WIT functions to the `Canvas` interface and its `Import` companion.
Each new function follows the existing pattern exactly.

New functions to add to the interface (and companion override):

```kotlin
// in interface Canvas:
fun skew(sx: Float, sy: Float)
fun concat(a: Float, b: Float, c: Float,
           d: Float, e: Float, f: Float,
           g: Float, h: Float, i: Float)
fun resetMatrix()
fun clipRrect(x: Float, y: Float, w: Float, h: Float, rx: Float, ry: Float, antiAlias: Boolean)
fun drawPaint(paint: PaintAttrs)
fun drawDrrect(ox: Float, oy: Float, ow: Float, oh: Float, orx: Float, ory: Float,
               ix: Float, iy: Float, iw: Float, ih: Float, irx: Float, iry: Float,
               paint: PaintAttrs)
fun drawArc(x: Float, y: Float, w: Float, h: Float,
            startAngle: Float, sweepAngle: Float,
            includeCenter: Boolean, paint: PaintAttrs)
fun drawImageRect(imageId: UInt,
                  srcX: Float, srcY: Float, srcW: Float, srcH: Float,
                  dstX: Float, dstY: Float, dstW: Float, dstH: Float,
                  paint: PaintAttrs)
// shader functions (stubs for now — task 11 implements them):
fun createLinearGradient(x0: Float, y0: Float, x1: Float, y1: Float,
                          colors: List<UInt>, stops: List<Float>, tileMode: Byte): UInt
fun createRadialGradient(cx: Float, cy: Float, radius: Float,
                          colors: List<UInt>, stops: List<Float>, tileMode: Byte): UInt
fun dropShader(id: UInt)
// clip-path (stub — task 09):
fun clipPath(pathData: List<Byte>, antiAlias: Boolean)
```

Also update `PaintAttrs` record class to add the new fields:

```kotlin
data class PaintAttrs(
    val color: UInt,
    val style: PaintStyle,
    val strokeWidth: Float,
    val strokeMiter: Float,       // NEW
    val strokeCap: StrokeCap,     // NEW
    val strokeJoin: StrokeJoin,   // NEW
    val antiAlias: Boolean,
    val alpha: UByte,
    val blendMode: BlendMode,     // NEW
)
```

Add new enum classes to `SkikoUi.kt`:

```kotlin
enum class BlendMode {
    SRC_OVER, SRC, DST_IN, DST_OUT, SRC_ATOP, DST_ATOP,
    XOR, MULTIPLY, SCREEN, OVERLAY, DARKEN, LIGHTEN,
    COLOR_DODGE, COLOR_BURN, HARD_LIGHT, SOFT_LIGHT,
    DIFFERENCE, EXCLUSION, CLEAR;
}

enum class StrokeCap  { BUTT, ROUND, SQUARE }
enum class StrokeJoin { MITER, ROUND, BEVEL }
```

Update `InternalSkikoUi.kt`: add `@WasmImport` external declarations for each
new function, following the exact naming convention already used in the file.

### 4. Update `SkiaTypes.wasi.kt` — Paint class and new Kotlin enums

```kotlin
// Add these enums (at file level, alongside existing PaintMode/ClipMode):
enum class BlendMode {
    SRC_OVER, SRC, DST_IN, DST_OUT, SRC_ATOP, DST_ATOP,
    XOR, MULTIPLY, SCREEN, OVERLAY, DARKEN, LIGHTEN,
    COLOR_DODGE, COLOR_BURN, HARD_LIGHT, SOFT_LIGHT,
    DIFFERENCE, EXCLUSION, CLEAR
}
enum class PaintStrokeCap  { BUTT, ROUND, SQUARE }
enum class PaintStrokeJoin { MITER, ROUND, BEVEL }

// Update Paint class:
class Paint {
    var color:       Int            = 0xFF000000.toInt()
    var mode:        PaintMode      = PaintMode.FILL
    var strokeWidth: Float          = 0f
    var strokeMiter: Float          = 4f      // Skia default
    var strokeCap:   PaintStrokeCap  = PaintStrokeCap.BUTT
    var strokeJoin:  PaintStrokeJoin = PaintStrokeJoin.MITER
    var isAntiAlias: Boolean        = false
    var alpha:       Int            = 255
    var blendMode:   BlendMode      = BlendMode.SRC_OVER

    fun apply(block: Paint.() -> Unit): Paint { block(); return this }
}
```

### 5. Update `WasiCanvas.kt`

Replace `witAttrs()` to include new fields:

```kotlin
private fun Paint.witAttrs(): WitCanvas.PaintAttrs {
    val styleVal = when (mode) {
        PaintMode.FILL            -> WitCanvas.PaintStyle.FILL
        PaintMode.STROKE          -> WitCanvas.PaintStyle.STROKE
        PaintMode.STROKE_AND_FILL -> WitCanvas.PaintStyle.FILL_AND_STROKE
    }
    val capVal = when (strokeCap) {
        PaintStrokeCap.BUTT   -> WitCanvas.StrokeCap.BUTT
        PaintStrokeCap.ROUND  -> WitCanvas.StrokeCap.ROUND
        PaintStrokeCap.SQUARE -> WitCanvas.StrokeCap.SQUARE
    }
    val joinVal = when (strokeJoin) {
        PaintStrokeJoin.MITER -> WitCanvas.StrokeJoin.MITER
        PaintStrokeJoin.ROUND -> WitCanvas.StrokeJoin.ROUND
        PaintStrokeJoin.BEVEL -> WitCanvas.StrokeJoin.BEVEL
    }
    val blendVal = when (blendMode) {
        BlendMode.SRC_OVER   -> WitCanvas.BlendMode.SRC_OVER
        BlendMode.SRC        -> WitCanvas.BlendMode.SRC
        BlendMode.DST_IN     -> WitCanvas.BlendMode.DST_IN
        BlendMode.MULTIPLY   -> WitCanvas.BlendMode.MULTIPLY
        BlendMode.SCREEN     -> WitCanvas.BlendMode.SCREEN
        BlendMode.CLEAR      -> WitCanvas.BlendMode.CLEAR
        // ... map all cases
        else -> WitCanvas.BlendMode.SRC_OVER
    }
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
    )
}
```

Add new Canvas method overrides:

```kotlin
override fun skew(sx: Float, sy: Float): Canvas {
    WitCanvas.Import.skew(sx, sy); return this
}

fun concat(matrix: Matrix33): Canvas {
    val m = matrix.mat
    WitCanvas.Import.concat(m[0],m[1],m[2], m[3],m[4],m[5], m[6],m[7],m[8])
    return this
}

fun resetMatrix(): Canvas {
    WitCanvas.Import.resetMatrix(); return this
}

override fun clipRRect(r: RRect, mode: ClipMode, antiAlias: Boolean): Canvas {
    val rx = if (r.radii.isNotEmpty()) r.radii[0] else 0f
    val ry = if (r.radii.size >= 2) r.radii[1] else rx
    WitCanvas.Import.clipRrect(r.left, r.top, r.width, r.height, rx, ry, antiAlias)
    return this
}

fun drawPaint(paint: Paint): Canvas {
    WitCanvas.Import.drawPaint(paint.witAttrs()); return this
}

fun drawDRRect(outer: RRect, inner: RRect, paint: Paint): Canvas {
    val orx = if (outer.radii.isNotEmpty()) outer.radii[0] else 0f
    val ory = if (outer.radii.size >= 2) outer.radii[1] else orx
    val irx = if (inner.radii.isNotEmpty()) inner.radii[0] else 0f
    val iry = if (inner.radii.size >= 2) inner.radii[1] else irx
    WitCanvas.Import.drawDrrect(
        outer.left, outer.top, outer.width, outer.height, orx, ory,
        inner.left, inner.top, inner.width, inner.height, irx, iry,
        paint.witAttrs()
    )
    return this
}

fun drawArc(oval: Rect, startAngle: Float, sweepAngle: Float,
            includeCenter: Boolean, paint: Paint): Canvas {
    WitCanvas.Import.drawArc(
        oval.left, oval.top, oval.width, oval.height,
        startAngle, sweepAngle, includeCenter, paint.witAttrs()
    )
    return this
}
```

### 6. Update `host/src/canvas_impl.rs` — Rust WIT trait implementation

Add/update `make_paint` to handle new fields:

```rust
fn make_paint(attrs: &PaintAttrs) -> skia_safe::Paint {
    let mut p = skia_safe::Paint::default();
    p.set_argb(
        ((attrs.color >> 24) & 0xFF) as u8,
        ((attrs.color >> 16) & 0xFF) as u8,
        ((attrs.color >>  8) & 0xFF) as u8,
        ( attrs.color        & 0xFF) as u8,
    );
    p.set_style(match attrs.style {
        PaintStyle::Fill         => skia_safe::PaintStyle::Fill,
        PaintStyle::Stroke       => skia_safe::PaintStyle::Stroke,
        PaintStyle::FillAndStroke => skia_safe::PaintStyle::StrokeAndFill,
    });
    p.set_stroke_width(attrs.stroke_width);
    p.set_stroke_miter(attrs.stroke_miter);
    p.set_stroke_cap(match attrs.stroke_cap {
        StrokeCap::Butt   => skia_safe::PaintCap::Butt,
        StrokeCap::Round  => skia_safe::PaintCap::Round,
        StrokeCap::Square => skia_safe::PaintCap::Square,
    });
    p.set_stroke_join(match attrs.stroke_join {
        StrokeJoin::Miter => skia_safe::PaintJoin::Miter,
        StrokeJoin::Round => skia_safe::PaintJoin::Round,
        StrokeJoin::Bevel => skia_safe::PaintJoin::Bevel,
    });
    p.set_anti_alias(attrs.anti_alias);
    p.set_alpha(attrs.alpha);
    p.set_blend_mode(match attrs.blend_mode {
        BlendMode::SrcOver  => skia_safe::BlendMode::SrcOver,
        BlendMode::Src      => skia_safe::BlendMode::Src,
        BlendMode::DstIn    => skia_safe::BlendMode::DstIn,
        BlendMode::DstOut   => skia_safe::BlendMode::DstOut,
        BlendMode::SrcAtop  => skia_safe::BlendMode::SrcATop,
        BlendMode::DstAtop  => skia_safe::BlendMode::DstATop,
        BlendMode::Xor      => skia_safe::BlendMode::Xor,
        BlendMode::Multiply => skia_safe::BlendMode::Multiply,
        BlendMode::Screen   => skia_safe::BlendMode::Screen,
        BlendMode::Overlay  => skia_safe::BlendMode::Overlay,
        BlendMode::Darken   => skia_safe::BlendMode::Darken,
        BlendMode::Lighten  => skia_safe::BlendMode::Lighten,
        BlendMode::ColorDodge => skia_safe::BlendMode::ColorDodge,
        BlendMode::ColorBurn  => skia_safe::BlendMode::ColorBurn,
        BlendMode::HardLight  => skia_safe::BlendMode::HardLight,
        BlendMode::SoftLight  => skia_safe::BlendMode::SoftLight,
        BlendMode::Difference => skia_safe::BlendMode::Difference,
        BlendMode::Exclusion  => skia_safe::BlendMode::Exclusion,
        BlendMode::Clear      => skia_safe::BlendMode::Clear,
    });
    p
}
```

Add new WIT trait implementations in `impl Host for HostState`:

```rust
fn skew(&mut self, sx: f32, sy: f32) {
    self.renderer.canvas().skew((sx, sy));
}

fn concat(&mut self, a: f32, b: f32, c: f32,
                     d: f32, e: f32, f: f32,
                     g: f32, h: f32, i: f32) {
    use skia_safe::Matrix;
    let m = Matrix::new_all(a, b, c, d, e, f, g, h, i);
    self.renderer.canvas().concat(&m);
}

fn reset_matrix(&mut self) {
    self.renderer.canvas().reset_matrix();
}

fn clip_rrect(&mut self, x: f32, y: f32, w: f32, h: f32,
              rx: f32, ry: f32, anti_alias: bool) {
    use skia_safe::{RRect, Rect};
    let rr = RRect::new_rect_xy(
        Rect::from_xywh(x, y, w, h), rx, ry);
    self.renderer.canvas().clip_rrect(rr, None, anti_alias);
}

fn draw_paint(&mut self, paint: PaintAttrs) {
    self.renderer.canvas().draw_paint(&make_paint(&paint));
}

fn draw_drrect(&mut self, ox: f32, oy: f32, ow: f32, oh: f32, orx: f32, ory: f32,
               ix: f32, iy: f32, iw: f32, ih: f32, irx: f32, iry: f32,
               paint: PaintAttrs) {
    use skia_safe::{RRect, Rect};
    let outer = RRect::new_rect_xy(Rect::from_xywh(ox, oy, ow, oh), orx, ory);
    let inner = RRect::new_rect_xy(Rect::from_xywh(ix, iy, iw, ih), irx, iry);
    self.renderer.canvas().draw_drrect(&outer, &inner, &make_paint(&paint));
}

fn draw_arc(&mut self, x: f32, y: f32, w: f32, h: f32,
            start_angle: f32, sweep_angle: f32,
            include_center: bool, paint: PaintAttrs) {
    use skia_safe::Rect;
    self.renderer.canvas().draw_arc(
        Rect::from_xywh(x, y, w, h),
        start_angle, sweep_angle,
        include_center,
        &make_paint(&paint),
    );
}

fn draw_image_rect(&mut self, image_id: u32,
                   src_x: f32, src_y: f32, src_w: f32, src_h: f32,
                   dst_x: f32, dst_y: f32, dst_w: f32, dst_h: f32,
                   paint: PaintAttrs) {
    use skia_safe::Rect;
    if let Some(img) = self.image_cache.get(&image_id) {
        let src = Rect::from_xywh(src_x, src_y, src_w, src_h);
        let dst = Rect::from_xywh(dst_x, dst_y, dst_w, dst_h);
        let p   = make_paint(&paint);
        self.renderer.canvas().draw_image_rect(
            img, Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)), dst, &p);
    }
}

// Shader stubs (implemented in task 11 — return 0 for now):
fn create_linear_gradient(&mut self, _x0: f32, _y0: f32, _x1: f32, _y1: f32,
                           _colors: Vec<u32>, _stops: Vec<f32>, _tile_mode: u8) -> u32 { 0 }
fn create_radial_gradient(&mut self, _cx: f32, _cy: f32, _radius: f32,
                           _colors: Vec<u32>, _stops: Vec<f32>, _tile_mode: u8) -> u32 { 0 }
fn drop_shader(&mut self, _id: u32) {}

// clip-path stub (implemented in task 09):
fn clip_path(&mut self, _path_data: Vec<u8>, _anti_alias: bool) {}
```

### 7. Add test to `test-app/src/wasmWasiMain/kotlin/Main.kt`

Add a section at the bottom that exercises the new features:

```kotlin
// ── Section: new primitives (task 08) ────────────────────────────────────
val t8Top = origTop + sp(100f)
canvas.drawString("task 08: arc / drrect / blendMode / clipRRect",
    margin, t8Top, Font(size = sp(11f)),
    Paint().apply { color = 0xFF94A3B8.toInt() })

// Arc (progress-indicator style)
canvas.drawArc(
    Rect.makeXYWH(margin, t8Top + sp(16f), sp(48f), sp(48f)),
    -90f, 270f, false,
    Paint().apply {
        color = 0xFF00D4FF.toInt(); mode = PaintMode.STROKE
        strokeWidth = sp(4f); strokeCap = PaintStrokeCap.ROUND; isAntiAlias = true
    }
)

// DRRect (donut shape)
canvas.drawDRRect(
    RRect.makeXYWH(margin + sp(60f), t8Top + sp(16f), sp(48f), sp(48f), sp(8f)),
    RRect.makeXYWH(margin + sp(68f), t8Top + sp(24f), sp(32f), sp(32f), sp(4f)),
    Paint().apply { color = 0xFFE94560.toInt(); isAntiAlias = true }
)

// ClipRRect
canvas.save()
canvas.clipRRect(RRect.makeXYWH(margin + sp(120f), t8Top + sp(16f), sp(80f), sp(48f), sp(16f)),
    ClipMode.INTERSECT, true)
canvas.drawPaint(Paint().apply { color = 0xFF533483.toInt() })
canvas.drawString("clipped", margin + sp(124f), t8Top + sp(44f),
    Font(size = sp(11f)), Paint().apply { color = 0xFFFFFFFF.toInt() })
canvas.restore()

// BlendMode: multiply overlay
canvas.save()
canvas.translate(margin + sp(210f), t8Top + sp(16f))
canvas.drawRect(Rect.makeXYWH(0f, 0f, sp(40f), sp(48f)),
    Paint().apply { color = 0xFF00D4FF.toInt(); isAntiAlias = true })
canvas.drawRect(Rect.makeXYWH(sp(10f), sp(8f), sp(40f), sp(48f)),
    Paint().apply { color = 0xFFFF6B6B.toInt(); blendMode = BlendMode.MULTIPLY; isAntiAlias = true })
canvas.restore()
```

### 8. Build and test

```bash
# Build Skiko jar
cd /home/harry/skiko
./gradlew :skiko:wasmWasiJar --console=plain --no-daemon 2>&1 | tail -5

# Build test-app
./gradlew :test-app:compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon 2>&1 | tail -10
```

If Gradle build fails, use the **gradle-triage** agent.

Then run the full build pipeline from CLAUDE.md and push to device.

---

## Verify

```bash
adb shell am force-stop com.example.wasmruntime
adb logcat -c
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
sleep 6
adb logcat -d | grep -E "(render_frame #[0-4]|fatal|error)"
```

Expected:
- `render_frame #0: ... ok=true` (first frame, any time <2s)
- No `fatal` or `error` lines related to canvas calls
- Device shows arc, donut, clipped region, blend mode section

### ✅ Checkpoint

```bash
cat > .task-state << 'EOF'
TASK=08
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 08 verified OK — arc/drrect/blendMode/clipRRect render on device
NOTES=
EOF
```

---

## Known issues

### `concat` matrix argument order

skia-safe `Matrix::new_all` takes `(scale_x, skew_x, trans_x, skew_y, scale_y, trans_y, persp_0, persp_1, persp_2)` — NOT row-major. Map accordingly or use `Matrix::from_affine`.

### WIT enum ordinal mismatch

The WIT bindgen assigns integer values to enum variants in declaration order.
If you add variants to `blend-mode` in a different order than the Rust match,
the values will be mismatched. Keep the Kotlin and Rust enum variant order
identical to the WIT declaration order.

### `clip_rrect` — `RRect::new_rect_xy` unavailable

Use `RRect::new_rect_radii` instead:
```rust
let radii = [skia_safe::Point::new(rx, ry); 4];
let rr = RRect::new_rect_radii(Rect::from_xywh(x, y, w, h), &radii);
```

### `draw_arc` sweep angle clamped

Skia's `draw_arc` with `sweep_angle > 360` draws nothing. Clamp in Rust if needed.

---

## Do NOT

- Add Shader implementation in this task — that's task 11. The stub returning 0 is correct for now.
- Change the WIT `paint-attrs` field order — it affects the canonical ABI
  parameter order in the generated `@WasmImport` declarations.
