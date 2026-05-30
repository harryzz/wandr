# Task 68 — Host-driven soft-keyboard inset

**Status:** OPEN (filed 2026-05-31, spun out of task 67 Phase 2 polish).

## Problem

When the soft keyboard (war.ime.keyboard overlay) is shown over a foreground
app, it occludes the bottom of the app's surface, but the app is **not resized
and not told the keyboard's height**. So a bottom bar (e.g. signal-ui's message
composer, a Compose text field) sits *behind* the keyboard while typing.

The wrong fix (rejected): have each app reserve a hard-coded fraction of the
surface for the keyboard. The overlay is `INITIAL_OVERLAY_PX = 1200` physical px
today (`runtime/wart-host/src/standalone.rs`) **and is resizable at runtime** via
`my:skiko-gfx/keyboard.request-overlay-height` — so any app that hard-codes a
height/fraction would need rebuilding whenever the keyboard size changes. The
height must come from the host, not the app.

Second gap: the **⌄ Hide key** (`KeyAction.Hide`,
`apps/system/war.ime.keyboard/.../ImeKeyboard.kt`) hides the IME's *own* overlay
but sends **no key/detach to the foreground app** (verified: no `on-key` /
`detach-editor` in logcat when pressed). So the app's editor stays focused and any
keyboard-driven layout change lingers after the keyboard is gone.

## Correct design (app-agnostic)

The **host** reduces the foreground app's **bottom inset** by the live overlay
height when the keyboard shows, and restores it on hide. That fires the guest's
existing `renderer.on-resize`, so **any** guest (Compose, dioxus-canvas, raw
canvas) re-lays-out its bottom content above the keyboard automatically — zero
hardcoding in apps; a resized keyboard is purely a host concern.

Pieces:
1. **Foreground-app host** — on `editor-attach` (already routed:
   `ime-host: forwarded attach-editor`, `ime_host_impl`) add the overlay height to
   the renderer's bottom inset (`canvas_impl::set_insets(top, bottom + kb)` →
   `on_resize`); on `editor-detach`, restore. Reuse the existing inset machinery
   (`standalone.rs` seeds `renderer.set_insets`).
2. **Arbiter** — owns the IME overlay + `request-overlay-height`, so it knows the
   *live* height. Push it to the foreground app's host at attach time (and on
   resize) instead of the host assuming `INITIAL_OVERLAY_PX`. So shrinking the
   keyboard to 30% reflects everywhere with no app rebuild.
3. **⌄ Hide key → detach** — pressing Hide must make the foreground app blur its
   editor (→ `notify-editor-detached` → arbiter clears overlay → host clears the
   inset). Either the IME sends the app an ESC/blur it acts on, or the arbiter
   drives the detach centrally. Today the dismiss path assumes "app loses focus →
   detach"; the Hide key doesn't cause that for non-Compose guests.

## Verify

- signal-ui composer (`repros/signal-ui`) rides just above the keyboard on focus
  and drops to the bottom when the keyboard is dismissed — **with no app-side
  keyboard-height constant** (revert the task-67 guest-side spacer/`KB_FRACTION`).
- A Compose text field (war.dioxus.demo's TextPanel, or a Compose app) gets the
  same behavior for free via `on-resize`.
- Temporarily change `INITIAL_OVERLAY_PX` (or `request-overlay-height`) and
  confirm both adapt without rebuilding the apps.

## Notes / building blocks already in place
- `DomRenderer::surface_size_logical()` + `set_keyboard_inset()` /
  `keyboard_inset` (`crates/dioxus-canvas`) — a guest-side inset already exists;
  the missing half is the host *driving* it (or driving `set_insets` so the
  surface shrinks and `on-resize` does the rest).
- Insets today: `canvas_impl::set_insets(top, bottom)`, seeded in `standalone.rs`
  from `WART_INSET_TOP/BOTTOM` (status bar / taskbar). The keyboard inset is an
  additional, dynamic bottom reservation on top of the taskbar.
- See [[reference_dioxus_taffy_rust_ui]] (edit-field + `min-height:0` notes),
  `tasks/67` Phase 2 item (2)/(3).
