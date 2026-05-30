---
name: Host-side WasiDrawable transforms — Android-style live transform attrs
description: Compose layer position/translation/scale/rotation/clip/alpha/shadow live on the host's C++ WasiDrawable struct, NOT in parent recordings. Matches Android's hardware RenderNode model. Setting a transform doesn't invalidate any parent.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---

**Background.** Android's hardware RenderNode stores `translationY`, `scaleX`, etc. as LIVE properties on the C++ RenderNode struct. The parent's display list captures `drawRenderNode(childHandle)` as a reference; child's transform is applied by the GPU at replay time. Setting `child.translationY = 100` updates the live struct and the next frame draws the child translated — without invalidating the parent.

Our wasi/skia backend now follows the same architecture. The `WasiDrawable` C++ class (`host/cpp/wasi_drawable.{h,cpp}`) holds:

- `fLayerX/fLayerY` — parent-space position (from Compose's `bounds.left/top`)
- `fTranslationX/Y` — Compose's `graphicsLayer.translationX/Y` post-translate
- `fScaleX/Y`, `fRotationZ`, `fPivotX/Y` — T(pivot)·S·R·T(-pivot)
- `fAlpha` — composite alpha; applied via saveLayer if < 1
- clip: `fClipKind` (None/Rect/RRect) + `fClipRect`/`fClipRRect` + `fClipAA`
- `fShadowElevation` — coarse drop-shadow

`WasiDrawable::onDraw(canvas)` applies them in order (1. layer pos → 2. translation → 3. scale/rotate around pivot → 4. shadow → 5. clip → 6. alpha saveLayer → 7. drawDrawable(inner) → restore). Setting any attr is a non-recordable host-state mutation — never captured into a recording.

## WIT surface

In `wit/skiko-gfx.wit` interface `canvas`:

```wit
set-drawable-transform: func(
    drawable-id: u32,
    layer-x: f32, layer-y: f32,
    translation-x: f32, translation-y: f32,
    scale-x: f32, scale-y: f32,
    rotation-z: f32,
    pivot-x: f32, pivot-y: f32,
    alpha: f32,
);
set-drawable-clip-rect:  func(drawable-id: u32, l: f32, t: f32, r: f32, b: f32, antialias: bool);
set-drawable-clip-rrect: func(drawable-id: u32, l: f32, t: f32, r: f32, b: f32, radii: list<f32>, antialias: bool);
clear-drawable-clip:     func(drawable-id: u32);
set-drawable-shadow-elevation: func(drawable-id: u32, elevation: f32);
```

The 8-float `radii` array follows `SkRRect::setRectRadii` order: UL.x, UL.y, UR.x, UR.y, LR.x, LR.y, LL.x, LL.y. Skiko's `RRect.makeComplexLTRB` matches.

## Kotlin side (`skiko/.../node/RenderNode.wasi.kt`)

Each transform-affecting property has a custom setter that re-pushes the COMPLETE transform record on change. `drawInto` shrunk to a single `Canvas.Import.drawDrawable(drawableId)` call (plus, for the rare `clipPath` outline or filter-heavy `layerPaint`/`imageFilter`/`colorFilter`/`blendMode` cases, a parent-captured `canvas.clipPath` / `canvas.saveLayer` wrapper — those rarely animate and aren't in the scroll/animation hot path).

## What this unlocks

1. **No more parent-re-record on child move.** `Modifier.verticalScroll`'s drag updates the child layer's `bounds.topLeft` → `setDrawableTransform` → host-state updated → next replay draws at new pos. Parent's recording is untouched.
2. **The previous `invalidateParentLayer?.invoke()` patch in `GraphicsLayerOwnerLayer.move()` is removed** — no longer needed. compose-multiplatform-core stays unmodified (we only fork via inclusion).
3. **Animated `translationY`, `scaleX`, `alpha`, etc. don't need parent re-record.** Layer-block re-invocation (the popup `graphicsLayer { State<Float> }` issue from task #53) still needs the layer block to read the State — but the *propagation* from scope.translationY → graphicsLayer.translationY → renderNode.translationY → wasiDrawable now takes the host-side fast path.

## What's NOT yet on the host

- `colorFilter`, `imageFilter`, `blendMode` (set via Compose's `layerPaint`) — still go through parent-captured `canvas.saveLayer`. Add later if hot-path requires.
- `clipPath` — still uses parent-captured `canvas.clipPath`. SVG/path serialization to host is doable but not a common case.
- `rotationX`, `rotationY`, `cameraDistance` — currently no-op fields on RenderNode (no 3D perspective transform). Add when needed.

## Files changed

- `wit/skiko-gfx.wit` + sync `skiko/wit/skiko-gfx.wit` — 5 new WIT functions.
- `host/cpp/wasi_drawable.{h,cpp}` — extended struct + `onDraw` rewrite + 5 new `extern "C"` setters.
- `host/src/canvas_impl.rs` — FFI mod extended + 5 new WIT method impls.
- `skiko/.../generated/InternalSkikoUi.kt` + `generated/SkikoUi.kt` — 5 new `@WasmImport` decls + 5 new `override fun` impls + 5 abstract method signatures.
- `skiko/.../node/RenderNode.wasi.kt` — full rewrite per above.
- `compose-multiplatform-core/.../GraphicsLayerOwnerLayer.skiko.kt` — reverted to upstream (no `invalidateParentLayer` in `move()`).

## Verification (2026-05-13)

Test app on device:
- Scroll up/down via swipe: content tracks finger smoothly, scroll position correct top↔bottom (Counter at top, Primary/Secondary buttons at bottom of List).
- Scroll back up + Counter +/- still works (3 taps → "3").
- Checkbox, RadioGroup, TextField placeholder, DropdownMenu trigger, Progress bars, Slider, Switch, Palette, Buttons all still render correctly at their new positions.

## How to apply

This is the architectural baseline going forward. When adding new layer-state properties (e.g. `rotationX` if 3D perspective is needed, or `colorFilter` if filters become hot-path), follow the same recipe:
1. Add WIT setter (non-recordable, takes `drawable-id` + value).
2. Add `extern "C"` wrapper + WasiDrawable field + apply in `onDraw`.
3. Add Rust FFI decl + WIT method impl.
4. Add generated Kotlin binding (3 places: `InternalSkikoUi.kt` `@WasmImport`, `SkikoUi.kt` `override fun`, abstract `fun` in interface).
5. Add Kotlin setter on `RenderNode` that pushes to host on change.

Do NOT add new transforms via `canvas.translate(...)` in `drawInto` — that bakes them into parent recordings and re-creates the original bug.
