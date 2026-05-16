---
name: RenderNode.drawInto applies clip/alpha/pivot/layerPaint/shadow
description: How the wasi RenderNode shim's drawInto now applies graphics-layer attributes — clip outline (rect/rrect/path), alpha modulation via saveLayer, pivot-aware transforms, layerPaint compositing, and a coarse shadow.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
Up until task #45, `RenderNode.drawInto` only emitted `drawDrawable(id)` after applying bounds and rough transforms. The renderNode fields `clip` / `setClipRect|RRect|Path` / `alpha` / `layerPaint` / `imageFilter` / `pivot` / `shadowElevation` / `setLightingInfo` were stored but never consumed — `setClipRect/RRect/Path` were *no-op stubs*. The Material3 "+" button rendered as a flat dark square after first tap because its rounded clip was thrown away.

Current behaviour (in `/home/harry/skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/node/RenderNode.wasi.kt::drawInto`), in order:

1. **bounds translate** — `canvas.translate(bounds.left, bounds.top)` so the inner drawable (recorded in local 0..w × 0..h coords) lands at the layer's position in parent space.
2. **post-translation** — `canvas.translate(translationX, translationY)`.
3. **pivot-aware scale/rotate** — wrapped in `T(pivot) · S · R · T(-pivot)` when any of scaleX/scaleY/rotationZ differs from identity. Skipped entirely if no transform is needed to avoid two extra matrix mults per draw.
4. **coarse shadow** — if `shadowElevation > 0 && clipShape != None`, drop a flat translucent black copy of the outline at `(0, elevation·0.8)` with alpha capped at 64/255. Hard edges, no blur — proper SkShadowUtils-style raytraced shadow would need blur shaders / MaskFilter plumbed through WIT. Visible only when the underlying surface is lighter than ~25% gray, so it disappears under Material3's dark-theme black background.
5. **clip** — `canvas.clipRect / clipRRect / clipPath` from the last-set `clipShape` (a sealed class: `None | Rectangle | Rounded | PathShape`). Only applied if `clip == true`. The clip data is stored at set-time by `setClipRect|RRect|Path`, which used to be no-ops.
6. **saveLayer for alpha/layerPaint** — only if `alpha < 1f`, `layerPaint != null`, or `imageFilter != null`. Builds a fresh `Paint` that copies layerPaint's `alphaf`/`imageFilter`/`blendMode`/`colorFilter` so the caller's Paint isn't mutated, then multiplies our own `alpha` on top of `layerPaint.alphaf`.
7. **emit `drawDrawable(id)`** — the deferred-replay outer wrapper (see `feedback_compose_render_node_picture.md`).
8. **restore** — pop saveLayer if pushed, then the outer save.

Known limits / deferred work:
- **Shadow:** no blur, hard edge, single drop direction. Proper M3 shadows would need either (a) MaskFilter blur exposed via WIT paint-attrs, or (b) a host-side `shadow_impl` that wraps `SkShadowUtils::DrawShadow`.
- **3D rotation:** `rotationX/Y` + `cameraDistance` ignored. Skia can do this via camera matrix but Compose seldom uses it.
- **maskFilter / pathEffect:** Stored but ignored. Compose uses them rarely; not worth plumbing unless a demo needs it.
- **Ambient/spot shadow colors:** Stored via `setLightingInfo` but the coarse shadow always paints solid translucent black.

**How to apply:**
- If a Compose UI shows incorrect clipping (content outside rounded shape), check `clipShape` is being stored — Compose calls `setClipRect/RRect/Path` BEFORE the first draw, so a unit test of the shim would catch dropped methods.
- If alpha isn't modulating, verify `alpha < 1f` is reaching `drawInto` (Material3's pressed-state alpha is e.g. 0.38f; check via logging once if uncertain).
- Don't try to combine `layerPaint` mutations with our own alpha by mutating `layerPaint` directly — Compose reuses the same Paint instance across frames and mutation corrupts state.
