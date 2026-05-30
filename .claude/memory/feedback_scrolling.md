---
name: Compose scrollable / verticalScroll on wasi — two independent fixes
description: Two separate bugs blocked Modifier.scrollable / verticalScroll on wasi. Both fixed 2026-05-13; touch-drag scrolling now works on device.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
**Symptom:** `Modifier.verticalScroll(rememberScrollState())` on the root Column. Touch-down + drag + touch-up makes `scrollState.value` look correct in a `LaunchedEffect(scrollState.value) { log(...) }` smoke probe (0 → 22 → 76 → ... → 980 during the drag), but the screen content does NOT scroll visually. The Counter card stays at the top; items below the fold never appear.

## Bug A — pointer-event coroutines stalled between events

`WasiFrameDispatcher` (queue-based, flushes once per frame after `scene.render`) only ran resumed continuations on the next frame's `flush()`. That broke Compose's `awaitPointerEventScope { while(true) { val e = awaitPointerEvent(); … } }` pattern used by `scrollable` / `detectDragGestures`: between events, the coroutine never re-ran to register a new awaiter via `awaitPointerEvent()`. Result — DOWN was processed (the initial awaiter saw it), 29 MOVE events fired into a coroutine that was queued-not-run, then UP arrived and the gesture had no drag history to scroll with.

**Fix:** flush the dispatcher AFTER each pointer event, not just per-frame.

```kotlin
// Main.kt - SkikoInputDelegate.onPointerEvent
realScene.sendPointerEvent(eventType = evtType, position = Offset(x, y), type = PointerType.Touch)
wasiFrameDispatcher.flush()  // ← added
```

After this fix alone, `scrollState.value` updated correctly during drag. But content STILL didn't move on screen — exposing bug B.

## Bug B — parent layer's recording stale after child layer moves (SUPERSEDED 2026-05-13)

**Status:** the upstream-mod fix below was REPLACED by a proper architectural fix — host-side live transform attrs on `WasiDrawable`. See `feedback_host_side_transforms.md`. The `invalidateParentLayer?.invoke()` patch in `GraphicsLayerOwnerLayer.move()` is REVERTED; compose-multiplatform-core stays unmodified. Keep the analysis below for context.



On Android with real hardware-accelerated RenderNodes, the parent's recording captures `drawRenderNode(childRenderNode)` as a REFERENCE; the child's translation is a property applied at GPU draw time. Setting `childRenderNode.translationY` doesn't require the parent to re-record.

On our wasi/skia backend, `RenderNode.wasi.kt::drawInto(canvas)` emits explicit `canvas.translate(bounds.left, bounds.top)` BEFORE the `drawDrawable(outerId)` op. The parent's SkPicture-style recording captures BOTH the translate AND the drawDrawable. So `bounds.left/top` is BAKED INTO the recording.

When `Modifier.placeRelativeWithLayer(xOffset, yOffset)` re-runs after `scrollState.value` changes, it calls `coordinator.move(position)` which delegates to `OwnedLayer.move(position)`. `GraphicsLayerOwnerLayer.move()` was setting `graphicsLayer.topLeft = position` (updates child's `bounds.left/top`) and calling `triggerRepaint()` (marks scene dirty for redraw) — but it never invalidated the PARENT layer's recording. So the scene redrew, but the parent's recording still played `translate(0, 0)` + `drawDrawable(child)`, putting the child back at its original position.

**Fix** in `compose-multiplatform-core/compose/ui/ui/src/skikoMain/.../GraphicsLayerOwnerLayer.skiko.kt::move()`:

```kotlin
override fun move(position: IntOffset) {
    layerManager.voteFrameRate(FrameRateCategory.High.value)
    graphicsLayer.topLeft = position
    invalidateParentLayer?.invoke()  // ← added — forces parent to re-record
    triggerRepaint()
}
```

`invalidateParentLayer` is provided at construction by `NodeCoordinator`:
```kotlin
private val invalidateParentLayer: () -> Unit = { wrappedBy?.invalidateLayer() }
```
which walks up to the parent coordinator's layer and marks IT dirty so it re-records.

## How to apply

Both bugs are wasi-specific architectural divergences from Android-Compose's assumptions, NOT upstream regressions. Keep both fixes in tree:

- **Pointer-flush fix** lives in test-app `Main.kt::SkikoInputDelegate`. If you fold the input plumbing into a reusable layer (e.g. a `WasiComposeInputAdapter` helper), keep the post-event `flush()` call inside it.
- **Parent-layer invalidate fix** lives in patched `compose-multiplatform-core` and needs to stay until/unless we rework `RenderNode.wasi.kt` to keep transforms on the outer `WasiDrawable` (live struct read at draw-time, not captured into parent's recording). That refactor would be the more architecturally-correct fix but is much more invasive (host C++ + Rust + skiko + WIT).

## Side-effects to watch

`invalidateParentLayer?.invoke()` on every `move()` can over-invalidate parent recordings in tight animation loops (e.g. a child that's being animated by translation will re-record the parent each frame). For now this is fine — the test app's drag scrolling works at ~60fps. If parent re-records become a perf hotspot later, the proper fix is the WasiDrawable live-transform refactor mentioned above.
