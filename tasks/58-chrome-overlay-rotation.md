# Task 58 — chrome overlays rotate with the device (status bar + taskbar)

> **Status:** 🔲 scoped 2026-05-29, not started. Spun out of task 56 +
> the content-insets fix. Fullscreen apps now rotate (task 43) and are
> correctly inset away from the chrome in any orientation (the host
> shrinks the logical frame + translates content into the chrome gap),
> but the **chrome overlays themselves don't rotate** — they're fixed
> physical top/bottom strips, so in landscape the status bar and taskbar
> sit along the side edges with sideways text.

## Problem

The status bar (`wandr.statusbar`, `OverlayMode::Top`) and taskbar
(`wandr.taskbar`, `OverlayMode::BottomBar`) are created as fixed
SurfaceFlinger overlay strips on the **physical** panel edges:

- status bar: physical `[0,0][PW,SB]` (top).
- taskbar: physical `[0,PH-TB][PW,PH]` (bottom).

Their renderers have orientation auto-follow **disabled** (`standalone.rs`
gates `enable_rotation` to fullscreen `None` mode — overlays pass it
`false`), and their surfaces are fixed-geometry. So when the device
rotates:

- The fullscreen app rotates its content (task 43) + is inset away from
  the physical top/bottom strips (the content-insets fix) — **correct**.
- The status bar / taskbar strips stay on the physical top/bottom, which
  in landscape are the **left/right side edges** of the held device, and
  their text/icons render horizontally → **sideways** relative to the
  landscape view. Their geometry (a 132/150-px-tall horizontal strip) is
  also wrong for a side edge.

User-visible: rotate to landscape → the clock/battery and the Back/Home/
Recents icons are rotated 90° and pinned to a side, not the top/bottom of
the landscape view.

## What "correct" looks like

In landscape the status bar should be a strip along the **visual top** of
the landscape view (a horizontal band across the long edge) with upright
text; the taskbar along the **visual bottom**. I.e. the chrome rotates
*with* the device, exactly like the fullscreen app already does.

## Approach (the overlay surfaces must follow orientation)

Each overlay process needs to do, on an orientation change, what the
fullscreen path does — but for a strip:

1. **Observe rotation.** Overlays currently disable the Device Orientation
   sensor poll. Enable a poll (or have the arbiter broadcast orientation
   to overlays — cleaner, one sensor reader). The arbiter already has a
   signal bus to the per-host sockets (task 47/54) — an
   `orientation <code>` push to each overlay's control socket is a natural
   fit and avoids N sensor readers.
2. **Recompute the strip geometry per orientation.** In portrait the
   status bar is `(0,0,PW,SB)`; in landscape it should be along the new
   visual-top edge. Because SurfaceFlinger composites onto the fixed
   physical panel, this means either (a) resize/move the overlay surface
   to the correct physical edge for that orientation (e.g. landscape
   status bar = a `(0,0,SB,PH)` *vertical* physical strip on the physical
   left, since physical-left = visual-top after a 90° hold) **and**
   rotate its content, or (b) keep the physical strip but rotate only the
   content (leaves the strip on the wrong edge — not enough). Option (a)
   is the real fix.
3. **Rotate the content.** Reuse `SkiaRenderer::set_orientation` (already
   exists) so the strip's text/icons draw upright. The overlay guest
   re-lays-out via `on_resize` with the swapped strip dims.
4. **Keep the content-insets in sync.** The fullscreen app's insets are
   currently `top=SB, bottom=TB` on the physical frame (task 56). When the
   chrome moves to the side edges in landscape, the app's inset must move
   correspondingly. Today the host derives insets from `WANDR_INSET_TOP/
   BOTTOM` env (physical top/bottom) and the dihedral maps them to the
   right logical edge *assuming the chrome stays on the physical top/
   bottom*. Once the chrome moves to physical side edges in landscape,
   the inset computation in `recompute_transform` must inset the matching
   physical edge instead. This couples task 58 to the task-56 inset math —
   do them together.

## Key files

- `runtime/wandr-host/src/standalone.rs` — overlay launch (`OverlayMode`),
  `enable_rotation` gating, the sensor poll loop.
- `runtime/wandr-host/src/sf_surface.rs` + `cpp/sf_surface.cpp` — overlay
  surface geometry (`sf_create_overlay_surface(x,y,w,h)`); a landscape
  strip needs different geometry, possibly a surface
  resize/reposition on rotation (new shim entry or recreate). **Building
  the shim needs the a-03 AOSP host** (see [[project-boot-model-libgui-build]]).
- `runtime/wandr-host/src/canvas_impl.rs` — `recompute_transform` inset
  math (must track which physical edge the chrome is on per orientation).
- `runtime/wandr-arbiter/src/main.rs` — (option 1) broadcast orientation to
  overlay control sockets.
- `apps/system/wandr.statusbar/`, `apps/system/wandr.taskbar/` — guests
  re-lay-out on `on_resize` (already handle resize; verify the strip-major
  axis swap looks right).

## Open questions

1. **Sensor source:** per-overlay sensor poll (simple, N readers) vs
   arbiter broadcasts one orientation to all overlays (cleaner, 1 reader,
   but new socket message + ordering vs the fullscreen app's own poll).
2. **Surface reposition:** can `libsf_surface.so` move/resize an existing
   overlay `SurfaceControl` cheaply on rotation, or must it recreate the
   surface (flicker)? Needs an a-03 shim experiment.
3. **Does the panel ever physically rotate** (SF buffer transform), or is
   everything content-pre-rotation as today? (Task 43 established it's all
   content pre-rotation — no EGL resize — so the overlays follow that
   model: fixed physical buffer, rotated content, but the strip must move
   to the correct physical edge.)
4. **Is landscape chrome even wanted** for the v1 product, or is hiding
   the chrome in landscape (immersive) acceptable short-term? Hiding is a
   trivial alternative (demote both overlays on landscape, restore on
   portrait) that sidesteps the surface-reposition work.

## Cheap interim option

If full landscape chrome is not worth the a-03 surface-reposition work
yet: **hide the chrome overlays in landscape** (the arbiter demotes
`wandr.statusbar` + `wandr.taskbar` on a landscape orientation event and
restores them on portrait), and have the host drop the app insets to 0
in landscape (immersive). One orientation signal + two demote/promote
calls + an inset toggle — no shim rebuild. Document as the v1 behavior
until a device/product actually needs landscape chrome.

## Related

- `tasks/56-taskbar.md` — the taskbar + the content-insets fix this spins
  out of.
- `tasks/55-status-bar.md` — the status bar overlay.
- `tasks/43-screen-orientation.md` — the fullscreen-app rotation model
  (content pre-rotation, no EGL resize) this must follow.
- [[project-standalone-orientation]], [[feedback_no_art_layer_dependencies]],
  [[project-boot-model-libgui-build]] (a-03 for the shim).
