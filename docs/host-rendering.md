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

- **Resources (post-Phase-C):** every host-resident object (shaders,
  images, pictures, typefaces, paragraphs, recordings, scene layers) is a
  wasmtime `ResourceTable` resource on the wasi:canvas@0.0.2 contract —
  the legacy u32-id maps are gone.

## Architecture: how the layers connect

```
Kotlin wandr-app (wasmWasiMain)
  └─ calls: org.jetbrains.skia.Canvas / Paint / Path / Shader / ...
       └─ WasiCanvas.kt routes to → wasi:canvas@0.0.2 bindings
            (generated/wasicanvas/, JetBrains Kotlin wit-bindgen fork)
            └─ WIT: proposals/wasi-canvas/wit (types/draw/layout/scene/embedding)
                 └─ Rust host: wasi_canvas_002_impl.rs (resources in
                    HostState.table; backing types in wasi_canvas_impl.rs)
                      └─ calls: skia_safe::Canvas / Paint / Path / Shader / ...
```

**One-way data flow for a draw call:**
1. Kotlin builds a `Paint`, sets color/blendMode/shader
2. `Paint.wasiPaint()` maps it to the wasi:canvas `paint` record
   (carrying an `option<borrow<shader>>` resource handle)
3. `canvas.draw-rect(rect, paint)` crosses the boundary on the frame
   canvas (or the innermost guest-explicit recording — see
   WasiCanvasBackend's target stack)
4. The host resolves the canvas resource and calls skia

**Text path:** Compose paragraphs go through `wasi:canvas/layout`
(0.0.2 setter-form paragraph-builder; host shapes via skparagraph).
The old text-blob verbs are gone — `drawString`/`drawTextBlob` build a
host paragraph per run (drawTextRun in WasiCanvasBackend.kt).

**Retained scenes:** RenderNode = a `wasi:canvas/scene` layer — content
recorded once via `graphics.start-recording` → `layer.set-content`
(consumes the recording LIVE; pictures would snapshot nested layers);
per-frame motion is `set-transform`/`set-alpha`/clips only. Host side:
the WasiDrawable C++ shim (canvas_impl.rs FFI + cpp/wasi_drawable.cpp).

## Skiko wasmWasiMain — file reference

| File | Purpose |
|------|---------|
| `generated/wasicanvas/` | wasi:canvas@0.0.2 bindings (Kotlin wit-bindgen fork; regen recipe in `wit-canvas/world.wit`) |
| `generated/uishell/` | wandr:ui-shell imports + shell-events/frame-pacing/input-handlers exports (`wit-shell/world.wit`) |
| `org/jetbrains/skia/SkiaTypes.wasi.kt` | Canvas (offscreen = new-offscreen resource), Paint, Rect, Path, Shader, Image, TextBlob value types |
| `org/jetbrains/skia/paragraph/` | Paragraph + ParagraphBuilder over wasi:canvas/layout |
| `org/jetbrains/skiko/WasiCanvas.kt` | The singleton main canvas — routes to WasiCanvasBackend.target (frame / innermost recording), guest CTM tracking for set/reset-matrix |
| `org/jetbrains/skiko/wasi/WasiCanvasBackend.kt` | frame bracket, recording stack, paint/type mappers, drawTextRun |
| `org/jetbrains/skiko/node/RenderNode.wasi.kt` | RenderNode over scene layers |
| `org/jetbrains/skiko/wasi/ShellImpls.kt` | export impls (shell-events, frame-pacing, pointer/key/frame handlers) |
| `org/jetbrains/skiko/SkiaLayerWasi.kt` | SkiaLayer — doFrame reads size off the acquired buffer |
