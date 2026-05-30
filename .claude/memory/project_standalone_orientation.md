---
name: project-standalone-orientation
description: How task-33 standalone orientation was fixed — eglQuerySurface lies on taimen Adreno; use ANativeWindow geometry
metadata:
  node_type: memory
  type: project
  originSessionId: 1b4553dd-d7f1-4367-9435-31b88f0c8841
---

Task 33 standalone display orientation — RESOLVED 2026-05-22, device-verified.
The guest UI rendered 90° rotated; final root cause and fix below.

**Key device fact (non-obvious):** on the Pixel 2 XL (taimen) Adreno driver,
`eglQuerySurface(EGL_WIDTH/EGL_HEIGHT)` **lies** — it returns the *transposed*
size (`2880×1440`) for a buffer that is really `1440×2880` portrait.
`ANativeWindow_getWidth/getHeight` returns the true geometry. The renderer
had built a `2880×1440` Skia GL surface (+ `glViewport`) over the real
`1440×2880` framebuffer → content rendered rotated/clipped.

**The fix (host-only):** `wart-host/src/egl.rs` `EglContext::new` now takes
the GL buffer geometry from `ANativeWindow_getWidth/getHeight`, preferring it
over the `eglQuerySurface` report. With correct dims `from_native_window`
sees `physical == intended == 1440×2880`, `base_matrix` is identity, the
guest renders 1:1 upright/full-screen. Applies to the NativeActivity path
too (same lie there). Input needs no transform (logical == input-window
frame). The shim (`cpp/sf_surface.cpp`) creates the surface portrait
`1440×2880` with a BLASTBufferQueue attached directly to `g_control`.

**Rejected — do not retry:**
- A host-side `base_matrix` rotation: a `WART_ORIENT 0..7` device sweep
  (full dihedral group) confirmed *no* rotation/mirror matrix yields upright
  portrait — the buffer was never actually transposed, so any transposing
  matrix just rotates/mirrors the result. It was a buffer-size bug.
- `NATIVE_WINDOW_TRANSFORM_HINT`: queried via the shim's
  `sf_query_transform_hint()` — reads 0 on taimen, uninformative (the
  transpose is an `eglQuerySurface` quirk, not a real layer transform).
- `setBuffersTransform` / `setDisplayProjection(ROTATION_90)` — earlier Step
  2 dead ends (per-buffer transform not honoured as composition rotation;
  display projection is a global change that rotates the launcher too).

**Inert override left in place:** `WART_ORIENT=<0..7>` (host base-matrix,
`FLIP_H=1 FLIP_V=2 ROT_90=4` dihedral bitmask) + `WART_SF_HINT=<0..7>`
(shim transform-hint pin) — escape hatches for a panel that genuinely needs
a rotation; default (unset) = identity, the correct path.

See `tasks/33-boot-model-bringup.md` Step 3. Related:
[[project-boot-model-libgui-build]], [[project-standalone-input]].
