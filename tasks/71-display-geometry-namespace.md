# Task 71 — Unified display/surface geometry namespace (DESIGN)

**Status:** DESIGN ONLY (filed 2026-05-31, spun out of task 68). No code yet —
capture the model + WIT shape, iterate, then implement.

## Why

Surface/dimension info is scattered and inconsistent across the host↔guest WIT:
- `canvas.surface-width/height` — the foreground app's *logical* area (already
  reduced by chrome AND the keyboard, task 68).
- `keyboard.request-overlay-height(px)` — the IME sizes its overlay in raw px.
- `status.bar-height` / taskbar height — chrome strip sizes, separate verb.
- Overlays (the IME) can't ask for the *screen's* dimensions at all (they only see
  their own overlay surface), so they can't size themselves as a fraction of the
  screen — which forced the host to scale a px request by orientation (task 68's
  `×pw/ph`), the messy bit we want to delete.

These are one family — display, content, keyboard occlusion — and belong in one
namespace, exposed consistently to every guest (fullscreen apps AND overlays).

## Mental model — three nested rectangles (user-space, current orientation)

```
┌─ display ───────────────────────────┐   full panel (e.g. 2880×1440 landscape)
│  ┌─ content ────────────────────┐   │   display − status bar − task bar
│  │  ┌─ safe ───────────────┐    │   │   content − soft keyboard
│  │  │  (focused editor must │    │   │   (= content when keyboard hidden)
│  │  │   stay inside here)   │    │   │
│  │  └──────────────────────┘    │   │
│  │      [ soft keyboard ]       │   │
│  └──────────────────────────────┘   │
│         [ task bar ]                 │
└──────────────────────────────────────┘
```

- **display** — the whole screen. Full-bleed needs (wallpaper, splash).
- **content** — what apps normally lay out to (chrome removed). Persistent;
  equals `display` for immersive apps with no chrome.
- **safe** — `content` minus the soft keyboard. The region a focused editor must
  stay within; transient (changes when the keyboard shows/hides). `safe == content`
  when the keyboard is down.

All orientation-aware: on rotation every rect is recomputed and `renderer.on-resize`
fires (as today).

## Proposed WIT — one `display` (or `surface`) interface

```wit
interface display {
  record size { width: u32, height: u32 }
  // (rect form — x/y too — if/when we need side/notch insets, see Q1)

  display-size:  func() -> size;          // full panel
  content-size:  func() -> size;          // minus chrome (status + task bar)
  safe-size:     func() -> size;          // minus chrome + keyboard
  orientation:   func() -> orientation;   // portrait | landscape (for the IME's %)

  // The geometry side of the keyboard moves here from `keyboard`:
  request-overlay-percent: func(percent: u32);  // IME: 30 / 42; host applies it
                                                //  to content-size().height
}
```
Input routing (`keyboard.send-key-event`, commit-text, …) stays in `keyboard`; only
the *geometry* (overlay sizing + the dims) lives here.

## How it clicks together

- **IME sizes itself resolution-independently:** on its current orientation (read
  from `orientation()` or its own surface width flipping 1440↔2880) it calls
  `request-overlay-percent(landscape ? 42 : 30)`. The host computes
  `percent × content-size().height` and reserves that. **No magic px, no host
  orientation-scaling** — delete task 68's `×pw/ph` (`overlay_rect`) and `×w/h`
  (`recompute_transform`); the host becomes a dumb applier in both orientations.
  - Subtlety: the keyboard % must be of **content** height (chrome removed) but NOT
    of `safe` (which already excludes the keyboard) — otherwise it's circular. The
    host has the content height before applying the keyboard reservation.
- **The keyboard-hides-editor fix (task 68) IS `safe = content − keyboard`.** Today
  the host bakes it into the foreground app's `surface-size()` so a bottom bar rises
  for free. Exposing `safe-size` makes it explicit so an app with a *mid-scroll*
  editor can scroll it into `safe` deliberately (the deferred task-68 follow-up).
- A guest that just wants "lay me out right" keeps using its surface (already =
  `safe`); a guest that needs the real screen asks `display-size`.

## Open questions (decide before implementing)

1. **`safe` as `size` or full `rect`?** Rect (x/y offsets) generalises to
   side/notch/cutout insets; size is simpler for the bottom-only keyboard today.
2. **`request-overlay-percent` (host multiplies) vs `request-overlay-height(px)` +
   app multiplies `content-size` itself?** Percent is cleanest for the IME; px is
   more general for a future non-IME overlay that wants an exact size.
3. **Fold `status.bar-height` / taskbar height in here too**, or leave in `status`?
4. **Naming:** `display` vs `surface` vs `geometry` for the interface; `safe` vs
   `viewport` vs `safe-area`.

## Migration / impact (when implemented)

- Host: add the `display` interface impl over the values `recompute_transform`
  already computes (it knows display, chrome insets, keyboard inset). Link it into
  `SkikoUi` so every guest gets it.
- Task 68 cleanup: drop the px-scaling in `overlay_rect` + `recompute_transform`;
  store a **percent** (or honor px directly), recompute `kb = percent × content_h`.
- IME (`wandr.ime.keyboard`): replace fixed `requestOverlayHeight(864)` with
  per-orientation `request-overlay-percent(30|42)`.
- `surface-width/height` can stay (alias of `safe-size`) for back-compat, or be
  redirected.

## Relationship to other tasks
- Built on task 68 (host-driven keyboard inset) — this generalises its inset model.
- Touches [[project-overlay-orientation]] (task 62) geometry; [[feedback_ime_layout_arbitration]].
- The mid-scroll-editor scroll-into-`safe` is the deferred task-68 follow-up.
