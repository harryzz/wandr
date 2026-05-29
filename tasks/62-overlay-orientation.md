# Task 62 — Orientation for overlay surfaces (IME / status bar / taskbar)

🔲 **Scoped 2026-05-29, not started.** Spun out of task 61 device testing — the
user observed the soft keyboard does not rotate with the device.

## The bug

Task 43 added runtime screen-orientation handling, but **gated it to the
fullscreen app only**:

```
runtime/wart-host/src/standalone.rs:352
    // Overlay (IME) surfaces don't auto-rotate (v1).
    let orient_sensor = if enable_rotation && WART_ORIENT unset { … } else { None };
```

`enable_rotation` is `true` only for `OverlayMode::None` (the fullscreen
launcher/app). The IME (`war.ime.keyboard`), status bar (`war.statusbar`), and
taskbar (`war.taskbar`) run as separate overlay-mode `wart-host` processes with
`enable_rotation = false`. So when the device rotates:

- the **fullscreen app** rotates (task 43: `canvas_impl::set_orientation` content
  pre-rotation via the dihedral `base_matrix` + swapped logical dims + re-issued
  `on_resize`; the physical SF buffer is never touched, input is inverse-mapped);
- the **overlays stay portrait** — in landscape the keyboard is still drawn for a
  portrait bottom strip, in the wrong place, and its touch mapping is portrait
  (taps misalign).

## Why it's not a one-line `enable_rotation = true`

Task 43's fullscreen fix works because the app owns a **fullscreen** buffer:
pre-rotating content fills the rotated screen. Overlays have **fixed anchored
geometry** created in the panel's native (portrait) space:

```
standalone.rs:111  create_overlay(SHIM_SO, x=0, y, w=0 (=full width), h)
                    IME:    y=-1 (bottom-anchored), h=INITIAL_OVERLAY_PX (1200)
                    status: y=0  (top),             h=status_bar_height_px()
                    taskbar:y=-1 (bottom-anchored), h=taskbar_height_px()
```

In landscape the *physical* bottom edge maps to a *side* of the portrait panel
buffer. Merely pre-rotating the keyboard content into the existing bottom-strip
buffer yields a sideways keyboard squished into a thin strip — wrong. A correct
landscape keyboard needs the **overlay geometry itself** to change: a wide-short
strip along the physical bottom = (in portrait-buffer space) a tall-narrow strip
down the left/right edge spanning the full panel height, width = keyboard height.

So the fix touches three layers that task 43's fullscreen path did not:
1. **Overlay surface geometry** must be recomputed per orientation (the
   `create_overlay` rect flips between portrait/landscape), OR the overlay becomes
   a fullscreen transparent surface that draws the keyboard in the correct rotated
   sub-rect (no shim change, but the guest must own placement + the empty area
   must pass touches through — passthrough on a fullscreen overlay is its own
   problem).
2. **The IME guest relayout** — the keyboard must lay out for the landscape width
   (longer dimension), not the portrait width. The guest needs the post-rotation
   logical dims via `on_resize`.
3. **Input inverse-transform** for the overlay (as task 43 did for the app), and
   the overlay must subscribe to the orientation sensor (enable it for overlays).

## Options

- **A — geometry-flip in the shim (proper).** On rotation, recreate/resize the
  overlay with rotated anchor+dims via `libsf_surface.so`, re-issue `on_resize`
  with landscape dims, inverse-map input. Needs a libsf_surface rebuild on a-03
  (see [[project-boot-model-libgui-build]]). Cleanest result; most work.
- **B — fullscreen transparent IME overlay.** The IME owns a fullscreen surface,
  draws the keyboard in the correct rotated sub-rect, reuses task 43's content
  pre-rotation wholesale (`enable_rotation = true` just works), and passes through
  touches outside the keyboard rect. Avoids shim geometry work but needs reliable
  touch-passthrough on the overlay + careful z-order vs the fg app.
- **C — orientation lock (stopgap).** Keep overlays portrait; when a
  non-`None`-orientation is active, the IME/bars hide or the system stays
  portrait-locked. Cheap, not a real fix.

## Recommendation

Option **A** for the IME, status bar, and taskbar together (they share the
overlay path), since the geometry-generic `create_overlay(x,y,w,h)` shim from
task 55 already parameterizes geometry — adding a `set_overlay_geometry` / resize
entry point is the natural extension. Reuse task 43's `device_orientation_handle`
+ `poll_device_rotation` + `set_orientation` machinery (already in `sensors_impl`
/ `canvas_impl`), lifting the `enable_rotation` gate for overlays once the
geometry flip is in place.

## Reuse / precedent

- `tasks/43-screen-orientation.md` — the fullscreen orientation machinery to
  extend (`set_orientation`, `dihedral_transform`, `device_rotation_to_orient`,
  pointer inverse-transform).
- `standalone.rs` overlay branch (`create_overlay`), `sf_surface.rs` /
  `cpp/sf_surface.cpp` (the geometry-generic shim), `keyboard_host_impl.rs`
  (`request_overlay_height` — the existing dynamic-resize precedent for the IME
  overlay).
- [[feedback_no_art_layer_dependencies]] — read rotation from the Device
  Orientation HAL sensor, not WMS.
