---
name: Warm-resume preserves wasm state across Android background/foreground
description: How the host keeps the wasmtime Store + Compose composition alive when winit::suspended/resumed fires, by swapping just the renderer's GPU parts and inheriting CPU-side caches.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
The previous implementation dropped `bindings/store/renderer` on `winit::suspended` and rebuilt them on `winit::resumed`. That meant every backgrounding (HOME press, notification pulldown, screen-rotation) destroyed wasm-side state — `appMain` re-ran, Compose composition was recreated, scheduler timers reset, lifecycle observers vanished.

Working pattern (verified 2026-05-12):

1. **`EglContext::drop` must NOT call `eglTerminate`.** The display is `EGL_DEFAULT_DISPLAY` (a process singleton). `eglTerminate` from one drop would invalidate any sibling context (during warm-resume swap, a fresh and a stale context co-exist briefly). `eglDestroySurface` + `eglDestroyContext` are sufficient.

2. **Two resume paths in `ApplicationHandler::resumed`:**
   - *Cold:* store+bindings are None → full `Store::new` + `SkikoUi::instantiate` (existing flow).
   - *Warm:* store+bindings already alive → build a fresh `SkiaRenderer` for the new NativeWindow, transfer the **CPU-side caches** from the live renderer into it via `SkiaRenderer::inherit_caches_from(&mut old)`, then `std::mem::replace` the renderer field on `Store::data_mut()` and drop the stale one. Dispatch `Resumed` via `set_lifecycle`.

3. **What to inherit (CPU-side, picture-replay-safe):**
   `text_blobs`, `multi_blob_cache`, `text_blob_runs`, `shader_cache`, `recorders`, `pictures`, `recording_stack`, `typeface_cache`, `para_builders`, `paragraphs`, `font_collection`, plus all `next_*_id` counters so the next host-minted ID doesn't collide with one already in the inherited tables.

4. **What NOT to inherit (GPU-resident):**
   `images`, `text_image_cache`/`text_image_keys`. Their underlying skia textures were owned by the dying `gr_context` and become invalid. Compose re-rasterizes/re-uploads them on first replay against the new context.

5. **`suspended` becomes:** dispatch `Stopped` via the lifecycle bridge (so observers see ON_STOP), set `self.window = None` to let android-activity release the NativeWindow, KEEP `store/bindings`. The store-resident renderer's EGL surface is now invalid; nothing accesses it until the next `resumed`.

Why inheriting pictures works visually: `skia_safe::Picture` is a CPU-side recording of draw ops. Replaying it against a different gr_context creates fresh GPU textures lazily during replay. Compose's RenderNode shim (`/home/harry/skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/node/RenderNode.wasi.kt`) calls `canvas.drawPicture(picture)` with the picture ID — host's `draw_picture` looks it up in the inherited `pictures` hashmap and the replay just works.

**How to apply:**
- The full Material3 demo's UI returns intact after HOME → return cycle. Counter/Slider values survive because composition is preserved.
- For now `Destroyed` events still aren't emitted (winit drops them); on real app termination, the store gets cleaned up by process exit. See task #42 for granular state emission.
- If you add new caches to `SkiaRenderer`, decide per-cache whether they're CPU-replayable (inherit) or GPU-resident (drop). Add to `inherit_caches_from`.
