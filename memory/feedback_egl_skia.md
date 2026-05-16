---
name: Android EGL context lifecycle and SkiaRenderer drop order
description: EGL make_current must be called before every render; SkiaRenderer field order determines drop order
type: feedback
originSessionId: ca7f3a70-2c6e-4c65-baae-454dc44933b5
---
**Call `egl.make_current()` at the start of every render frame (before any canvas op), not just before flush.**

On Android, the EGL context can become unbound between `resumed()` and the first `RedrawRequested`. Skia's `surface.canvas()` on a GPU-backed surface may internally trigger GL state queries, hitting "no current context" before `flush_and_submit()` is reached.

**Why:** `eglMakeCurrent` binds per-thread; anything between creation and first draw can unbind it. Android's event loop may dispatch internal callbacks that affect EGL state.

**How to apply:** Add `self.egl.make_current()` at the top of `draw_test_frame` (and `flush_and_swap`) — belt-and-suspenders.

---

**`SkiaRenderer` fields must be declared with `egl` LAST so it drops after `gr_context` and `surface`.**

Rust drops struct fields in declaration order. If `egl` is declared first, `EglContext::drop()` calls `eglMakeCurrent(NO_CONTEXT)` before `GrDirectContext::drop()` flushes its GL pipeline — triggering "no current context" during Skia cleanup.

**Why:** Skia's `GrDirectContext` destructor makes GL calls. The EGL context must still be bound when this happens.

**How to apply:** In `SkiaRenderer`, order fields: `gr_context`, `surface`, other fields, then `egl` last (with `// dropped last` comment).

---

**In test mode (no WASM component), store the renderer in `App.test_renderer`, not as a local.**

In `resumed()`, the `renderer` variable must be assigned to `self.test_renderer = Some(renderer)` in the `else` branch. If it's left as a local, it drops at end of the `if` block, destroying the EGL context immediately — before any frames are drawn.

**Why:** `renderer_mut()` previously only looked through `self.store`, so the test-mode renderer was never accessible from `RedrawRequested`.

**How to apply:** `App` has `test_renderer: Option<SkiaRenderer>`; `renderer_mut()` checks `self.store` first, falls back to `self.test_renderer`; `suspended()` clears both.
