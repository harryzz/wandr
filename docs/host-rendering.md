# Host rendering & architecture notes

_How the host renders (EGL/skia decisions), the Kotlin→WIT→host data flow,
and the skiko wasmWasiMain file map. Read for canvas_impl / skiko / rendering work._

## Key rendering decisions

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

## Architecture: how the layers connect

```
Kotlin wart-app (wasmWasiMain)
  └─ calls: org.jetbrains.skia.Canvas / Paint / Path / Shader / ...
       └─ WasiCanvas.kt delegates to → WIT imports (generated/SkikoUi.kt)
            └─ WIT interface: wit/skiko-gfx.wit
                 └─ Rust host: runtime/wart-host/src/canvas_impl.rs implements WIT trait
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

## Skiko wasmWasiMain — file reference

| File | Purpose |
|------|---------|
| `generated/SkikoUi.kt` | WIT-generated public API — `Canvas.Import.*` calls |
| `generated/InternalSkikoUi.kt` | Low-level `@WasmImport` external function declarations |
| `org/jetbrains/skia/SkiaTypes.wasi.kt` | Canvas, Paint, Rect, RRect, Font, Typeface, TextBlob, Path stubs |
| `org/jetbrains/skiko/WasiCanvas.kt` | Concrete Canvas implementation — delegates to WIT imports |
| `org/jetbrains/skiko/SkiaLayerWasi.kt` | SkiaLayer stub — beginFrame/endFrame, renderDelegate |
| `org/jetbrains/skiko/wasi/RendererImpl.kt` | WIT renderer export — renderFrame, onPointerEvent, onKeyEvent, onResize |
