# Task 43 — screen orientation handling in standalone mode

> **Status:** 🔲 scoped 2026-05-26, not started. Deferred mid-session
> because the libgui shim change requires the AOSP a-03 build host
> (`project-boot-model-libgui-build`), not the regular dev machine.

## Why this matters

wart-host's standalone path acquires a SurfaceFlinger surface at fixed
1440×2880 portrait dimensions. SurfaceFlinger doesn't auto-rotate
surfaces that aren't owned by an Activity, so wart-app stays in
portrait even when the device is physically landscape. Material
widgets, lists, and the markdown card all assume portrait reading.

A correct rotation handler would:
1. Detect device rotation (sensors or `settings get system user_rotation` poll).
2. Re-rotate the SurfaceFlinger buffer transform via `Surface::setTransform`
   (libgui call, needs the shim).
3. Swap the EGL backing buffer / Skia surface dimensions.
4. Tell Compose via `call_on_resize(new_width, new_height)`.
5. Preserve Skia/Compose state across the dimension change (warm-resume style).

## What needs to land

| Step | Where | Notes |
|---|---|---|
| 1. Sensor / settings poll | `wart-host/src/standalone.rs` | Every ~500ms read `user_rotation` (manual lock) AND `dumpsys window` (auto-rotated). Cheap; only act on change. |
| 2. Buffer transform | `cpp/sf_surface.{cpp,h}` + new soong build | Add `sf_set_rotation(rotation)` that calls `nativeWindow->perform(window, NATIVE_WINDOW_SET_BUFFERS_TRANSFORM, transform)` and/or `SurfaceControl::setTransform`. **Requires AOSP a-03 tree** for the libgui rebuild. |
| 3. EGL / Skia resize | `wart-host/src/canvas_impl.rs` | Re-create the EGL surface (or resize) at swapped dimensions; rebuild Skia `gr_context` surface. Inheritance of caches (text blobs, paragraphs) — pattern in lib.rs warm-resume. |
| 4. Compose on-resize | already exists | Call `skiko.my_skiko_gfx_renderer().call_on_resize(w, h)`; Compose re-lays out. |
| 5. State preservation | similar to lib.rs warm-resume | Take the existing warm-resume code's "inherit_caches_from" mechanism, generalize for the runtime-dimension-change case. |

## Why deferred this session

- Step 2 alone needs the AOSP a-03 build host (per memory
  `project-boot-model-libgui-build`); the regular dev machine can't
  build the soong target.
- Step 5 (warm-resume in standalone) is non-trivial — current warm-
  resume only fires on Android activity onResume; the standalone path
  has no equivalent trigger.
- Realistic effort 3-5 hours of focused work on the right machine.

## Out of scope for v1

- Per-app orientation locks (some apps want portrait-only; that's a
  manifest-level declaration to add later).
- Smooth rotation animation (just swap; OS handles the rotation overlay).
- Multi-display support (only the primary display, for now).

## Recommended order if/when picked up

1. Start on the regular dev machine: add the **rotation polling** (step 1) + **Compose on_resize call** (step 4). With a portrait surface still, this proves the detection + signal path.
2. Move to a-03 tree for the **libgui shim work** (step 2). Test that the buffer transform actually rotates the displayed image.
3. Wire the **EGL/Skia resize** (step 3) — most risk here.
4. Stress-test rotation with **state preservation** (step 5) — markdown card mid-scroll, text-field with focus, etc.

## Related

- [[project-boot-model-libgui-build]] — shim must build in-tree on a-03.
- [[feedback_warm_resume]] — the cache-inheritance pattern to reuse.
- [[project-standalone-orientation]] — earlier orientation fix that's
  about the INITIAL boot rotation (different bug; this task is about
  runtime orientation CHANGES).
- `tasks/33-boot-model-bringup.md` — the standalone-mode foundation
  this rides on.
