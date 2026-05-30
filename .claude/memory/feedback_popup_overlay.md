---
name: Compose popup overlay on wasi — setup + animated-graphicsLayer fix
description: Popups (DropdownMenu/AlertDialog/Tooltip/ExposedDropdownMenu) need CanvasLayersComposeScene + a real WindowInfo.containerSize. The "animated graphicsLayer invisible inside a popup" bug is RESOLVED (2026-05-20) — root cause was layer alpha baked into the parent recording, fixed in RenderNode.wasi.kt. No LocalInspectionMode workaround needed anymore.
metadata:
  node_type: memory
  type: feedback
  originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---

**Required setup for popups on wasi:**

1. Use `CanvasLayersComposeScene` (not `PlatformLayersComposeScene`).
2. Inject a `PlatformContext` with a real `WindowInfo.containerSize` in
   pixels. `PlatformContext.Empty()` defaults to `IntSize.Zero`, which
   makes the popup measure to nothing.

**Animated `graphicsLayer` inside a popup — RESOLVED 2026-05-20.**

Symptom (was): Material3 `DropdownMenu` / `AlertDialog` /
`ExposedDropdownMenu` rendered **invisible** — the menu was composed,
laid out and hit-testable (tapping where an item would be selected it),
but painted nothing. Mitigation in use until the fix:
`CompositionLocalProvider(LocalInspectionMode provides true)`, which
makes Material3 `Menu` snap to target instead of animating.

Earlier diagnoses in this memory ("the `graphicsLayer` block is not
re-invoked", "updateTransition path broken") were **wrong**. The actual
root cause, found by host-side `WasiDrawable` tracing:

- `SkiaGraphicsLayer.requiresLayer()` returns true for any `alpha < 1f`,
  so `updateLayerProperties()` sets `RenderNode.layerPaint` to a `Paint`
  carrying that alpha.
- The wasi `RenderNode.drawInto` (`skiko/.../node/RenderNode.wasi.kt`)
  treated `layerPaint != null` as "needs filter layer" and emitted
  `canvas.saveLayer(bounds, paint)` — with `paint.alpha` — **into the
  parent's recording**.
- A popup's root records its content **once**. At the animation's first
  frame alpha ≈ 0, so `saveLayer(alpha=0)` was frozen into the popup
  root's recording. Every later frame replayed that frozen op → the
  layer's content drew into a fully-transparent layer → invisible
  forever. (The child's *live* `WasiDrawable.alpha` animated 0→1
  correctly but was overridden by the frozen parent `saveLayer`.)
- The main composition escaped this only because its parents re-record
  every frame, refreshing the frozen value.

This violated the host-side-transform invariant (see
[[host-side-transforms]]): transform/alpha are LIVE `WasiDrawable`
attrs precisely so parent recordings never need refreshing.

**Fix (in `RenderNode.wasi.kt` `drawInto`, wasi-only):** a recorded
`saveLayer` is emitted ONLY for genuine compositing filters
(`colorFilter` / `imageFilter` / non-`SrcOver` `blendMode`). Plain alpha
is no longer baked — `p.setAlphaf(layerPaint.alphaf)` was removed; alpha
rides solely the live `WasiDrawable` attr applied in
`WasiDrawable::onDraw`. One fix covers DropdownMenu, ExposedDropdownMenu,
AlertDialog enter, Tooltip, and any future animated popup. Also removes
wasteful main-scene parent re-records.

Device-verified 2026-05-20: `DropdownMenu` opens and animates with no
`LocalInspectionMode` workaround.

**How to apply:** for any "animated content invisible inside a
popup/dialog" report on wasi, the suspect is a transform/alpha value
baked into a parent recording rather than applied as a live
`WasiDrawable` attr — diagnose by tracing `save_layer` / clip / matrix
ops landing in the popup root's `PictureRecorder`.
