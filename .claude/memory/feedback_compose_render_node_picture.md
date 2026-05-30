---
name: Compose RenderNode uses WasiDrawable (SkDrawable subclass) for deferred lookup
description: Why our wasi RenderNode wraps an SkDrawable subclass with a swappable inner from finishRecordingAsDrawable — not finishRecordingAsPicture — and the layout assumptions that make the C++ shim work.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
Compose's per-leaf invalidation only works when parent recordings hold a *reference* to the child's draw output, not a snapshot. Skia has two finishing paths and they differ in this respect:

- `SkPictureRecorder::finishRecordingAsPicture()` calls `newDrawableSnapshot()` on the embedded drawable list and stores it as `SkBigPicture::SnapshotArray` of frozen `sk_sp<const SkPicture>`. Subsequent child re-records don't propagate — that was the original bug.
- `SkPictureRecorder::finishRecordingAsDrawable()` keeps the `SkRecord` + the *live* drawable list. Playback invokes each `drawable->draw(canvas)` at replay time. Deferred lookup all the way down. **This is what we use.**

Architecture (all in `host/cpp/wasi_drawable.{h,cpp}` + `host/src/canvas_impl.rs` + `RenderNode.wasi.kt`):

1. **Outer wrapper** = `WasiDrawable`, a C++ subclass of `SkDrawable` with a mutable `sk_sp<SkDrawable> fInner`. Its `onDraw(canvas)` calls `canvas->drawDrawable(fInner.get(), nullptr)`. Refcount-managed via C FFI `wasi_drawable_create/_ref/_unref/_set_inner/_set_bounds`. One outer per `RenderNode`, lifetime = RenderNode lifetime.
2. **Inner** = `SkRecordedDrawable` (skia's return type from `finishRecordingAsDrawable`), refreshed each time the layer re-records. Our `set-drawable-from-recorder` WIT call: finishes the recorder, gets the drawable handle, calls `wasi_drawable_set_inner` (which bumps the SkDrawable's refcount via `sk_ref_sp`), drops the handle. The wrapper keeps the only owning ref.
3. **Parent recordings** capture `canvas->drawDrawable(outer.get())` ops in their own SkRecord. At parent replay, skia invokes `outer->draw(canvas)` → `outer->onDraw` → `canvas->drawDrawable(currentInner)`. Recursion: inner is an SkRecordedDrawable which replays its SkRecord, hitting more drawDrawable ops for grandchildren, etc.

C++ shim binds against skia's `SkDrawable.h` / `SkCanvas.h` / `SkRect.h` headers vendored at the rust-skia 0.93.1-matching commit (`319323662b1685a112f5` → skia submodule sha `37e26f9cc16e89c8f28d386e6748e43475f7e4a9`). Sparse-checkout in `host/vendor/skia-src/`. cc compiles `wasi_drawable.cpp` against those headers; symbols resolve against `libskia.a` pulled in by skia-safe at link time. NDK clang++ at `aarch64-linux-android35-clang++` (versioned API in NDK r23+).

Rust↔skia-safe bridge avoids `pub(crate)` accessors via a layout-trick helper:

```rust
// RCHandle<N> is `pub struct RCHandle<N>(NonNull<N>)` — single-field tuple
// struct over a transparent NonNull, so the first 8 bytes ARE the *mut N.
fn handle_to_native_ptr<T>(handle: *const T) -> *mut c_void {
    unsafe { *(handle as *const *mut c_void) }
}
```

Same trick works for `Drawable` (handle → SkDrawable*) and `Picture` (handle → SkPicture*). For `Canvas` (`pub struct Canvas(UnsafeCell<SkCanvas>)` — repr(transparent) wrapper), the simpler `canvas as *const Canvas as *mut SkCanvas` cast works because UnsafeCell IS transparent.

**Verified 2026-05-12:** Counter increments 0→7 across 7 taps on the Material3 demo with NO force-re-record (the `isDirty` check in `GraphicsLayerOwnerLayer.updateDisplayList` is restored to its upstream behavior). PR-clean, no workaround.

**Known cosmetic limitations** (orthogonal to deferred lookup):
- `RenderNode.drawInto` does NOT apply `alpha`, `clip`, `layerPaint`, `imageFilter`, `maskFilter`, `pathEffect`, `pivot`, `shadowElevation`, `cameraDistance`, etc. These are stored on the data class but unused. Material3's `+` button after first tap looks like a flat dark-purple square instead of an elevated rounded oval — the pressed-state elevation/clip aren't applied. Initial render is correct because Compose synthesizes the rounded shape inside the inner recording.
- Same issue presumably affects shadow rendering for Cards, dropdown menu shadows, dialog dim, etc.

**How to apply:**
- If you add new graphicsLayer fields that need rendering, wire them into `RenderNode.drawInto`'s `canvas.save() / transforms / restore` block.
- If you debug a parent layer "not picking up child changes," verify both: (a) child's `endRecording` is being called (logMessage to confirm), (b) parent uses `setDrawableFromRecorder` not the legacy `setDrawablePicture`. The picture path is gone but it's worth a check during refactors.
- If skia-safe is upgraded, re-confirm the `handle_to_native_ptr` layout trick — Rust doesn't guarantee tuple-struct layout, but rust-skia hasn't changed RCHandle's shape across many versions.
