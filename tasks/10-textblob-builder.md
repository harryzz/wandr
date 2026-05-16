# Task 10 — TextBlobBuilder (multi-run styled text)

> **Status: ✅ complete (verified 2026-05-15).** Implementation shipped as part of the working end-to-end Compose-on-WASM PoC. WIT entries, Rust host impl, and Kotlin wasmWasi stubs all in place. This file is kept as historical reference for the architectural decisions made during implementation.

## Goal

`TextBlobBuilder` lets callers build a blob from multiple text runs, each with
its own font. `drawTextLine` works. Common Compose text patterns like mixing
bold and normal weight in one line are possible without `Paragraph`.

Done looks like: the test-app renders a single text line that contains both
normal and bold segments drawn from a `TextBlobBuilder`, and a `TextLine`
equivalent rendered via `drawTextLine`.

---

## Architecture

The existing `createTextBlob` WIT function creates a single-run blob.
Multi-run blobs require a builder protocol:

```
beginTextBlob()                      // start accumulating runs
addTextRun(text, family, size, weight, italic, x, y)  // add one run
...
endTextBlob() → u32                  // finalise, return blob ID
```

The host builds a `skia_safe::TextBlobBuilder` and appends each run.
`endTextBlob` calls `.make()` and stores the result.

The existing `drawTextBlob` / `dropTextBlob` functions are reused unchanged.

---

## Steps

### 1. Add WIT functions

In `wit/skiko-gfx.wit`, inside the `canvas` interface, add after the existing
text blob functions:

```wit
    // --- multi-run text blob builder ---
    begin-text-blob: func();
    add-text-run:    func(text: list<u8>, font-family: list<u8>,
                          size: f32, weight: u32, italic: bool,
                          x: f32, y: f32);
    end-text-blob:   func() -> u32;
```

Sync to skiko repo:

```bash
cp /home/harry/wasm-android-runtime/wit/skiko-gfx.wit \
   /home/harry/skiko/skiko/wit/skiko-gfx.wit
```

### 2. Update Kotlin bindings (`generated/SkikoUi.kt` and `InternalSkikoUi.kt`)

Add to the Canvas interface and Import companion:

```kotlin
fun beginTextBlob()
fun addTextRun(text: String, fontFamily: String,
               size: Float, weight: UInt, italic: Boolean,
               x: Float, y: Float)
fun endTextBlob(): UInt
```

Add the corresponding `@WasmImport` declarations in `InternalSkikoUi.kt`,
following the exact naming convention already used.

### 3. Add `TextBlobBuilder` to `SkiaTypes.wasi.kt`

The existing `TextBlob` class stores a single run. `TextBlobBuilder` accumulates
multiple runs and finalises via `build()`:

```kotlin
class TextBlobBuilder {
    data class Run(
        val text:       String,
        val fontFamily: String,
        val size:       Float,
        val weight:     Int,
        val italic:     Boolean,
        val x:          Float,
        val y:          Float,
    )

    private val runs = mutableListOf<Run>()

    fun appendRun(font: Font, text: String, x: Float, y: Float): TextBlobBuilder = apply {
        val family = font.filePath.takeIf { it.isNotEmpty() } ?: font.familyName
        runs += Run(text, family, font.size, font.weight, font.italic, x, y)
    }

    /**
     * Sends all runs to the host and returns a single-blob ID.
     * The caller owns the ID and must call dropTextBlob when done.
     */
    fun build(): TextBlob {
        WitCanvas.Import.beginTextBlob()
        for (r in runs) {
            WitCanvas.Import.addTextRun(
                r.text, r.fontFamily, r.size, r.weight.toUInt(), r.italic, r.x, r.y)
        }
        val id = WitCanvas.Import.endTextBlob()
        // Return a "handle" TextBlob — text field null signals it's host-managed
        return TextBlob(_hostId = id)
    }
}
```

Update `TextBlob` to carry an optional host-managed ID for blobs built via
`TextBlobBuilder` (so `WasiCanvas.drawTextBlob` can reuse the same draw path):

```kotlin
class TextBlob(
    val text:       String?  = null,
    val fontFamily: String   = "",
    val filePath:   String   = "",
    val size:       Float    = 14f,
    val weight:     Int      = 400,
    val italic:     Boolean  = false,
    internal val _hostId: UInt = 0u,   // non-zero = host-managed multi-run blob
) {
    companion object {
        fun makeFromString(text: String, font: Font?): TextBlob = TextBlob(
            text       = text,
            fontFamily = font?.familyName ?: "",
            filePath   = font?.filePath   ?: "",
            size       = font?.size       ?: 14f,
            weight     = font?.weight     ?: 400,
            italic     = font?.italic     ?: false,
        )
    }
}
```

### 4. Update `WasiCanvas.drawTextBlob`

Handle both single-run blobs (original path) and host-managed blobs:

```kotlin
override fun drawTextBlob(blob: TextBlob, x: Float, y: Float, paint: Paint): Canvas {
    if (blob._hostId != 0u) {
        // Host-managed multi-run blob — ID already valid
        WitCanvas.Import.drawTextBlob(blob._hostId, x, y, paint.witAttrs())
        // Note: caller owns the lifecycle; do NOT drop here
        return this
    }
    // Single-run blob — original path
    val text = blob.text ?: return this
    val family = blob.filePath.takeIf { it.isNotEmpty() } ?: blob.fontFamily
    val blobId = WitCanvas.Import.createTextBlob(
        text, family, blob.size, blob.weight.toUInt(), blob.italic)
    WitCanvas.Import.drawTextBlob(blobId, x, y, paint.witAttrs())
    WitCanvas.Import.dropTextBlob(blobId)
    return this
}
```

### 5. Add `TextLine` as a thin `TextBlob` wrapper

`TextLine` in commonMain is backed by native harfbuzz shaping. For WASM, a
simplified version that renders a pre-built string at a fixed y-baseline is
sufficient:

```kotlin
class TextLine private constructor(
    internal val blob: TextBlob,
    internal val _width: Float,
) {
    val width: Float get() = _width

    companion object {
        fun make(text: String, font: Font): TextLine =
            TextLine(TextBlob.makeFromString(text, font), text.length * font.size * 0.6f)
    }
}
```

Add `drawTextLine` to `WasiCanvas`:

```kotlin
fun drawTextLine(line: TextLine, x: Float, y: Float, paint: Paint): Canvas =
    drawTextBlob(line.blob, x, y, paint)
```

### 6. Update `canvas_impl.rs` — implement builder WIT functions

Add a `text_blob_builder` accumulation field to `SkiaRenderer`:

```rust
pub struct SkiaRenderer {
    // ... existing fields ...
    text_blob_runs: Vec<TextBlobRun>,  // accumulated runs for the active builder
}

struct TextBlobRun {
    text:    String,
    family:  String,
    size:    f32,
    weight:  u32,
    italic:  bool,
    x:       f32,
    y:       f32,
}
```

Implement the three new WIT functions in `impl Host for HostState`:

```rust
fn begin_text_blob(&mut self) {
    self.renderer.text_blob_runs.clear();
}

fn add_text_run(&mut self, text: Vec<u8>, font_family: Vec<u8>,
                size: f32, weight: u32, italic: bool,
                x: f32, y: f32) {
    let text   = String::from_utf8_lossy(&text).into_owned();
    let family = String::from_utf8_lossy(&font_family).into_owned();
    self.renderer.text_blob_runs.push(TextBlobRun { text, family, size, weight, italic, x, y });
}

fn end_text_blob(&mut self) -> u32 {
    // Build each run as a separate single-run blob, rendered at its (x,y).
    // Store the list of (blob, x, y) as a "multi-run blob" resource.
    let runs = std::mem::take(&mut self.renderer.text_blob_runs);
    let id = self.renderer.next_blob_id;
    self.renderer.next_blob_id += 1;

    // Convert each run to a skia TextBlob and store
    let blobs: Vec<(skia_safe::TextBlob, f32, f32)> = runs.iter().filter_map(|r| {
        let tf = self.renderer.get_typeface(&r.family, r.weight >= 700, r.italic);
        let font = skia_safe::Font::from_typeface_with_params(
            tf, r.size, 1.0, 0.0);
        skia_safe::TextBlob::from_str(&r.text, &font)
            .map(|b| (b, r.x, r.y))
    }).collect();

    self.renderer.multi_blob_cache.insert(id, blobs);
    id
}
```

Update `draw_text_blob` to handle multi-run blobs stored in `multi_blob_cache`:

```rust
fn draw_text_blob(&mut self, id: u32, x: f32, y: f32, paint: PaintAttrs) {
    // Try multi-run cache first
    if let Some(blobs) = self.renderer.multi_blob_cache.get(&id) {
        let p = make_paint(&paint);
        for (blob, bx, by) in blobs {
            // Use CPU blit path for consistent font rendering
            self.renderer.draw_text_blob_cpu(blob, x + bx, y + by, &p);
        }
        return;
    }
    // Fall through to single-run blob cache (existing path)
    if let Some(blob) = self.renderer.blob_cache.get(&id) {
        let p = make_paint(&paint);
        self.renderer.draw_text_blob_cpu(blob, x, y, &p);
    }
}
```

Add `multi_blob_cache: HashMap<u32, Vec<(skia_safe::TextBlob, f32, f32)>>` to
`SkiaRenderer` and initialize it in `SkiaRenderer::new`.

Update `drop_text_blob` to also remove from `multi_blob_cache`:

```rust
fn drop_text_blob(&mut self, id: u32) {
    self.renderer.blob_cache.remove(&id);
    self.renderer.multi_blob_cache.remove(&id);
}
```

### 7. Add test to `Main.kt`

```kotlin
// ── Section: TextBlobBuilder (task 10) ────────────────────────────────────
val t10Top = t9Top + sp(110f)
canvas.drawString("task 10: TextBlobBuilder multi-run",
    margin, t10Top, Font(size = sp(11f)),
    Paint().apply { color = 0xFF94A3B8.toInt() })

val baselineY = t10Top + sp(30f)
val builder = TextBlobBuilder()
builder.appendRun(Font(size = sp(16f), weight = 400), "Hello ", margin, baselineY)
builder.appendRun(Font(size = sp(16f), weight = 700), "bold ", margin + sp(48f), baselineY)
builder.appendRun(Font(size = sp(16f), italic = true), "italic ", margin + sp(92f), baselineY)
builder.appendRun(
    Font(Typeface.makeFromFile("/system/fonts/DroidSansMono.ttf"), sp(14f)),
    "mono", margin + sp(144f), baselineY
)
val multiBlob = builder.build()
canvas.drawTextBlob(multiBlob, 0f, 0f,
    Paint().apply { color = 0xFFE2E8F0.toInt(); isAntiAlias = true })
// drop manually since host-managed
WitCanvas.Import.dropTextBlob(multiBlob._hostId)
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
adb logcat -d | grep -E "(render_frame #[0-4]|begin_text|add_text|end_text|fatal)"
```

Expected:
- No fatal errors
- Device shows "Hello **bold** *italic* mono" all on one line in the task 10 section

### ✅ Checkpoint

```bash
cat > .task-state << 'EOF'
TASK=10
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 10 verified OK — TextBlobBuilder multi-run text renders correctly
NOTES=
EOF
```

---

## Known issues

### x offsets for multi-run text require manual measurement

`appendRun` takes explicit `x` positions. Kotlin has no font metrics to measure
advance widths at this stage. Work around by estimating: `advance ≈ text.length * size * 0.6`.
Task 11+ can add font metrics queries if precise measurement is needed.

### `TextBlobBuilder` `build()` calls WIT immediately

Unlike commonMain where the builder accumulates native pointers, this builder
calls WIT during `build()`. Do not call `build()` inside a performance-critical
per-frame path without caching the resulting `TextBlob`.

### Multi-run blob `x` parameter in `drawTextBlob`

When drawing a multi-run blob from `TextBlobBuilder`, the `x, y` passed to
`drawTextBlob` are applied as an **offset** on top of the per-run positions
stored in the blob. If you want absolute positioning, pass `x=0, y=0`.

---

## Do NOT

- Implement full harfbuzz text shaping — that's `Paragraph` (task 14, deferred).
- Expose `_hostId` as public API — it's an implementation detail.
- Call `dropTextBlob` inside `drawTextBlob` for host-managed blobs — the caller owns them.
