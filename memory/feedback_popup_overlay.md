---
name: Compose popup overlay on wasi — three changes + isolated `graphicsLayer { State }` block-not-re-invoked bug
description: DropdownMenu/AlertDialog/Tooltip/anything popup-bearing needs (1) CanvasLayersComposeScene, (2) WindowInfo with real containerSize, (3) LocalInspectionMode workaround for the Material3 alpha animation. Root cause of (3): in popup (layer) compositions, the `Modifier.graphicsLayer { … }` block IS NOT RE-INVOKED when its read State<Float> changes — even though the composable recomposes per state change. Bug is in placement / layer-block dispatch, not in animation or state tracking.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
**Required setup for popups on wasi** (verified 2026-05-13):

1. **Use `CanvasLayersComposeScene`** (not `PlatformLayersComposeScene`).
2. **Inject a `PlatformContext` with a real `WindowInfo.containerSize`** in pixels. `PlatformContext.Empty()`'s default is `IntSize.Zero`, which makes the popup measure to nothing.
3. **`Modifier.graphicsLayer { State<Float> }` block re-invocation IS NOW FIXED** for pure `Popup` widgets — confirmed by test-app diagnostic (a `Popup` containing `Box.graphicsLayer { alpha = animateFloatAsState(...) }` animates correctly from 0→1; `gfx-block` re-invokes per frame with progressing `a`). One of the architectural fixes this session resolved it. Most likely candidates: host-side `WasiDrawable` transforms (task #51), `WasiFrameDispatcher.Delay` impl (task #52), post-pointer-event `flush()`.
4. **Material3 `DropdownMenu` / `AlertDialog` / `ExposedDropdownMenu` STILL need the `LocalInspectionMode provides true` workaround**. These use `MutableTransitionState` / `updateTransition` for their expand-collapse animation (a different code path from `animateFloatAsState`). Items don't appear without the workaround. Same workaround pattern as before.

## The underlying bug, precisely

The minimal reproducer:
```kotlin
Popup(...) {
    var target by remember { mutableStateOf(0f) }
    LaunchedEffect(Unit) { target = 1f }
    val a by animateFloatAsState(target, tween(1500))
    SideEffect { logMessage("compose a=$a target=$target") }
    Box(
        modifier = Modifier
            .size(200.dp, 80.dp)
            .graphicsLayer {
                logMessage("block a=$a")
                this.alpha = a
            }
            .background(Color.Red)
    ) { … }
}
```

Logs:
```
block:   #1..15 all a=0.0  (burst at initial composition)
compose: a=0.0, 0.0, 0.00004, 0.0005, 0.0016, 0.003, 0.005, 0.008, …   (every recompose)
```

- The popup composable RECOMPOSES every frame; `SideEffect`-captured `a` reflects the animation progressing 0→1.
- The `graphicsLayer { … }` block runs ~15 times during initial composition (all reading `a=0.0`) and then **never again**.
- Result: layer applies alpha=0 forever → popup invisible despite the animated state advancing.

So the chain is broken at: "State read inside the layer block → snapshot observer → layer invalidation → block re-invocation". The animation IS running; the composition IS recomposing; the State<Float> IS updating; the layer block IS NOT being re-invoked.

## What's NOT broken (confirmed via test-app)
- Animation pipeline: `animateFloatAsState` + the underlying Animatable coroutine run correctly in layer compositions (`compose` log progresses).
- State observation in composables: SideEffect / DisposableEffect see the new alpha each frame.
- Layer compose/draw: composition commits, popup is laid out, hit-testable; plain `Box(Modifier.size().background(…))` and `Box(Modifier.size().graphicsLayer{alpha=1.0f}.background(…))` both render correctly.
- saveLayer rendering: `graphicsLayer { alpha = 0.99f }` (fixed value forcing the saveLayer path) renders correctly.

## What IS broken
- `Modifier.graphicsLayer { … }` block, when reading a State<Float> that animates inside a **layer** composition (popup/dialog/tooltip), is invoked only at initial composition and **never re-invoked** when the State updates.
- Same modifier in the **main** composition works correctly (Counter, RadioButton, Switch, etc. all animate).

## Suspected location of the bug
The "state read inside layer block → re-invoke block" pipeline lives in:
- `Modifier.placeWithLayer(…, layerBlock = layerBlock)` and its measure/place implementation
- `GraphicsLayerOwnerLayer.skiko.kt`'s `updateLayerProperties` / `invalidate`
- The `SnapshotStateObserver` that subscribes the layer block to state reads

The diff is between mainOwner and AttachedComposeSceneLayer.owner — they use the same `RootNodeOwner` class. The bug is likely in how the layer's owner's snapshot observer is wired up, or in how state-write notifications reach the layer's `updateLayerProperties` path.

## What the snapshot-observer wiring actually looks like (verified 2026-05-13)

`CanvasLayersComposeSceneImpl` and `AttachedComposeSceneLayer.owner` both construct `RootNodeOwner` passing the SAME `snapshotInvalidationTracker` (from `BaseComposeScene`). So the deferred-CommandList and its flush via `sendAndPerformSnapshotChanges()` is **shared** between main and layer owners.

But each `RootNodeOwner` creates its own `OwnerSnapshotObserver` via `snapshotInvalidationTracker.snapshotObserver()` — wrapping a per-owner `SnapshotStateObserver`. Each owner's `init` calls `snapshotObserver.startObserving()` which registers an apply-observer with the global `Snapshot`.

For a `NodeCoordinator` belonging to the layer's owner, `snapshotObserver` resolves to `layoutNode.requireOwner().snapshotObserver` → the LAYER's observer. So:
- `NodeCoordinator.updateLayerParameters()` calls `snapshotObserver.observeReads(this, onCommitAffectingLayerParams) { layerBlock.invoke(scope) }`
- The layer's `SnapshotStateObserver` records reads under target=coordinator
- State writes globally fire all started apply-observers, including the layer's
- `onCommitAffectingLayerParams(coordinator)` → `coordinator.updateLayerParameters()` → block re-invoked
- Result flows through `layer.updateLayerProperties(scope)` → `graphicsLayer.alpha = scope.alpha` → `renderNode.alpha = value`

The wiring **looks** symmetric between main and layer owners. The bug must be a subtler divergence (maybe `node.isAttached` flips false on layer recompose; maybe `layer == null` early-exit fires for popup coordinators; maybe `scopeMaps` in the layer's SnapshotStateObserver isn't subscribed to the right state objects).

## Both BLOCK and PARAM forms fail in popup (verified 2026-05-13)

```kotlin
// Both fail:
.graphicsLayer { this.alpha = a }      // BLOCK form
.graphicsLayer(alpha = a)              // PARAM form
```

This rules out "only the snapshot-observer-only path is broken" — even PARAM form's `SimpleGraphicsLayerModifier.update(node)` → `node.invalidateLayerBlock()` → `updateLayerBlock(layerBlock, forceUpdateLayerParameters=true)` doesn't take effect for popup-owned coordinators.

Both forms eventually call `LayoutModifierNode.updateLayerBlock(layerBlock)`:
```kotlin
fun LayoutModifierNode.updateLayerBlock(layerBlock: (GraphicsLayerScope.() -> Unit)?) {
    if (!node.isAttached) return                          // ← may early-exit
    requireCoordinator(Nodes.Layout)
        .wrapped
        ?.updateLayerBlock(layerBlock, forceUpdateLayerParameters = true)
}
```
And inside `NodeCoordinator.updateLayerBlock`:
```kotlin
if (layoutNode.isAttached && layerBlock != null) {        // ← may early-exit
    this.layerBlock = layerBlock
    if (layer == null) { … } else if (updateParameters) {
        updateLayerParameters()
    }
}
```

**Two early-exit guards to instrument**: `node.isAttached` and `layoutNode.isAttached`. If either is false for popup coordinators during recompose, the invalidate is silently dropped. This is the next concrete diagnostic — add a single `logMessage(if (!node.isAttached) "INVAL_DROP_DET" else "INVAL_OK")` at the top of `LayoutModifierNode.updateLayerBlock` and rebuild compose-ui-wasi.

**Next investigation step**: instrument `LayoutModifierNode.updateLayerBlock` early-exits and `NodeCoordinator.updateLayerBlock` early-exits with `logMessage()` to identify which guard is firing. Then walk back from that guard to the popup's recompose path to see why.

## Things that did NOT fix the bug (left in tree as improvements anyway)
- `identityHashCode` was-broken fix (was needed for Checkbox's main-composition Transition; doesn't fix layer's).
- `WasiFrameDispatcher` replacing `Dispatchers.Unconfined` (upstream's own TODO).
- Caching `nanoTime` in `compose-ui-wasi/UiActuals.wasi.kt::currentTimeMillis()` (avoids the `currentNanoTime` realloc-allocator poisoning).

## Workaround in test app
```kotlin
CompositionLocalProvider(LocalInspectionMode provides true) {
    DropdownMenu(…) { … }
}
```
This makes Material3 Menu use `expandedState.targetState`-conditional snap-to-target instead of the animated value. Visual is correct, no animation.

Plain `Popup` (no Material3 Surface, no alpha animation) needs none of this — works fine.
