# Task 43 — screen orientation handling in standalone mode

> **Status:** ✅ device-verified 2026-05-29. Auto-follow works both
> landscape directions, full-screen, taps land correctly.
>
> **The "needs a-03" premise was wrong.** The task was scoped assuming
> visible rotation requires a SurfaceFlinger buffer-transform shim
> (`Surface::setTransform`, needing the a-03 build host). It does not:
> the host-side `WANDR_ORIENT` machinery in `canvas_impl.rs` already does
> **content pre-rotation** into a fixed portrait buffer via a Skia
> `base_matrix` (the full 8-orientation dihedral group). Runtime rotation
> = make that dynamic. No a-03, no SF buffer transform, no EGL resize.
>
> **Rotation source (ART-free, per [[feedback_no_art_layer_dependencies]]):**
> the native **Device Orientation HAL sensor**
> (`android.sensor.device_orientation`, type 27, on-change) reports
> screen rotation 0/1/2/3 directly — the SAME sensor WMS's
> `WindowOrientationListener` reads, consumed here via the rsbinder
> sensorservice path. No accel→rotation fusion math, no `system_server`.
>
> **What landed:**
> - `canvas_impl.rs` — factored the orient→matrix math into
>   `dihedral_transform()`; added `current_orient` + `set_orientation()`
>   (recomputes `base_matrix` + swapped logical dims live; physical GL
>   buffer untouched).
> - `sensors_impl.rs` — `find_handle_by_type(27)` + cross-platform host
>   wrappers `device_orientation_handle()` / `enable_sensor()` /
>   `poll_device_rotation()`.
> - `standalone.rs` — enable the sensor once, poll per frame (on-change →
>   cheap), apply via `set_orientation` + re-issue `on_resize`;
>   inverse-transform pointer coords by `base_matrix.invert()` so taps
>   land when rotated; `device_rotation_to_orient` mapping
>   (0→0, 1→4, 2→3, 3→7 — handedness device-confirmed correct);
>   gated to the fullscreen app (not IME overlay); `WANDR_ORIENT` env
>   forces a fixed orient (disables auto-follow).
> - **wandr-app** (`Main.kt` + `RealComposeApp.kt`) — the render delegate
>   was discarding the per-frame `w/h` from `doFrame`, so the
>   `CanvasLayersComposeScene` stayed sized at the startup (portrait)
>   geometry and `base_matrix` rotated portrait content into the
>   landscape buffer → only a corner visible. Fix: the delegate now sets
>   `realScene.size` + the popup `containerSize` (`MutableSceneWindowInfo`,
>   exposed via `realSceneWindowInfo`) on a size change. **This was the
>   actual half-render bug** — pre-existing, latent (orient 0 never
>   exercised it). No skiko rebuild needed; `doFrame` already fed fresh
>   `surfaceWidth/Height` every frame.
>
> **Orientation lock** stays out of v1 (see "Out of scope"). When wanted
> it's a declarative `package.toml` field (e.g. `orientation = "auto" |
> "portrait" | "landscape"`) the host reads to gate `set_orientation` —
> NOT a runtime WIT export (host owns the rotation, so an app can't lock
> it purely app-side; but it's a static manifest declaration, not a verb).

## Original scope (kept for history; the a-03 framing below is superseded)

## Why this matters

wandr-host's standalone path acquires a SurfaceFlinger surface at fixed
1440×2880 portrait dimensions. SurfaceFlinger doesn't auto-rotate
surfaces that aren't owned by an Activity, so wandr-app stays in
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
| 1. Sensor / settings poll | `wandr-host/src/standalone.rs` | Every ~500ms read `user_rotation` (manual lock) AND `dumpsys window` (auto-rotated). Cheap; only act on change. |
| 2. Buffer transform | `cpp/sf_surface.{cpp,h}` + new soong build | Add `sf_set_rotation(rotation)` that calls `nativeWindow->perform(window, NATIVE_WINDOW_SET_BUFFERS_TRANSFORM, transform)` and/or `SurfaceControl::setTransform`. **Requires AOSP a-03 tree** for the libgui rebuild. |
| 3. EGL / Skia resize | `wandr-host/src/canvas_impl.rs` | Re-create the EGL surface (or resize) at swapped dimensions; rebuild Skia `gr_context` surface. Inheritance of caches (text blobs, paragraphs) — pattern in lib.rs warm-resume. |
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
