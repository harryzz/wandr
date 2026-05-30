---
name: Compose interactivity broken at child-OwnedLayer drawLayer step
description: Input + recomposition + layout + modifier update + layer invalidation ALL work on wasmWasi, but only the root OwnedLayer's drawLayer is ever invoked — child OwnedLayers' picture stays at records=1 and renders stale.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
State of Compose-on-wasmWasi interactivity as of 2026-05-12:

What works:
- Pointer event chain end-to-end: winit Touch → host `dispatch_pointer_v2` (both v1 and v2) → `SkikoInputDelegate.onPointerEvent` → `realScene.sendPointerEvent` → `inputHandler.onPointerEvent` → hit-testing → gesture-detector callbacks fire (e.g. `detectTapGestures { onPress = { ... }, onTap = { ... } }`)
- `mutableStateOf` writes inside gesture callbacks
- Recomposition (LaunchedEffect re-fires with new state value)
- Measure/layout (`onGloballyPositioned` reports new sizes after state changes)
- Modifier reconciliation (`BackgroundElement.update(node)` fires with the new color)
- OwnedLayer invalidation (multiple `GraphicsLayerOwnerLayer.invalidate()` instances fire many times per tap — 60+ on root, 30+ on others)

What is broken:
- Only the ROOT GraphicsLayer's `draw` is ever invoked. Child OwnedLayers exist and are invalidated, but their `drawLayer` is never called, so `updateDisplayList` never re-records, and their `RenderNode.endRecording` count stays at 1 (initial composition).
- The root layer's record draws embed `drawPicture(child.id)` calls (Skia's nested-Picture semantics). When the root replays, those nested picture refs point at the STALE child pictures from records=1. So the screen shows initial state forever.
- A direct `canvas.drawRect(...)` call from inside our renderDelegate AFTER `realScene.render()` DOES paint each frame (confirmed via a moving magenta marker), proving EGL swap + Skia GPU pipeline are fine.

The most likely place for the bug: `NodeCoordinator.draw` (commonMain) does `val layer = layer; if (layer != null) layer.drawLayer(...) else drawContainedDrawModifiers(...)`. For sub-Composables, the `layer` field must be getting null/wrong despite OwnedLayers being created, OR the OwnedLayer→NodeCoordinator link isn't established on wasi. Investigate where the OwnedLayer is supposed to be attached to its NodeCoordinator and verify it happens on our build.

The Slider crash (PathBuilder.reset/rewind recursion) is separately FIXED — that was Kotlin source-level infinite recursion, not related to this.

**How to apply:**
- When a Compose UI doesn't visibly update on wasi, do NOT assume input or state propagation is broken — measure it. The break is at the per-OwnedLayer drawLayer invocation step.
- Quick repro: add a Box with `Modifier.background(color).pointerInput { detectTapGestures { onPress = { count++ } } }` where `color` depends on `count`. The text and tap counts in logs will work; visually the color will not change.
- Next diagnostic step: instrument `NodeCoordinator.draw` in commonMain to log whether `layer` is null when state-affected subtrees are drawn.

**Why:** Saves hours of re-diagnosing the same issue. The instrumented logging trail used (BackgroundElement.update, GLOL.invalidate@hash, RenderNode endRecording count, GraphicsLayer.draw count) is the canonical way to confirm where the break is.
