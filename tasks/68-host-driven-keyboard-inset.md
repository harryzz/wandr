# Task 68 — Host-driven soft-keyboard inset

**Status:** IN PROGRESS (started 2026-05-31). Design agreed with user:
- **Keyboard height = computed from the panel** (resolution-independent), the IME
  is the source of truth (reads panel height, requests a fraction). Not a magic px.
- **Lift scope = bottom composers only** — host shrinks the foreground app's bottom
  inset → `on-resize` → bottom-anchored bars rise automatically. No guest
  scroll-to-field work this task.

## Confirmed integration points (code read 2026-05-31)
- Inset already shrinks the guest area: `surface_height()` returns `logical_height`,
  and `recompute_transform()` = `physical − inset_top − inset_bottom`
  (`canvas_impl.rs:629`). So `set_insets(top, base_bottom + kb)` + re-issue
  `on_resize(lw,lh)` makes a bottom composer rise — zero app constant.
- Base insets seeded from `WART_INSET_TOP/BOTTOM` at startup (`standalone.rs:163`);
  the keyboard adds ON TOP of the base bottom → must track base (top,bottom).
- Per-host control socket `ime_inbound.rs` (`InboundEvent` enum + queue, drained in
  `standalone.rs:801`) is the arbiter→host channel. Add `KeyboardInset{px}` + a
  `keyboard-inset <px>` wire line, mirroring `KeyEvent`.
- IME already sets its own overlay height: `requestOverlayHeight(1200)`
  (`war.ime.keyboard RealComposeApp.kt:126,132`) → `keyboard_host_impl.rs:50` →
  `sf_surface::request_overlay_resize` (sizes the IME's OWN surface only — not
  propagated to the foreground app; that's the gap).

## Shipped (2026-05-31) — host + arbiter only, no Kotlin rebuild

**Flow:** editor focuses → app's `notify-editor-attached` → arbiter `attach-editor`.
The arbiter now ALSO pushes `keyboard-inset <H>` down the focused-editor host's
per-host socket (`ime_inbound.rs`, new `InboundEvent::KeyboardInset`); `0` on
detach. H is the IME's live overlay height — the IME's `request_overlay_height`
host impl reports it to the arbiter via the new `ime-overlay-height` cmd
(`state::ime_overlay_height`, default 1200), so the keyboard is the source of truth.

**The key fix — one clean rule in `canvas_impl::recompute_transform`:** rotate the
FULL panel, then reserve ALL insets in USER space — status bar at user-top,
taskbar + soft-keyboard at user-bottom (`logical_height = rotated_h − top − bottom
− keyboard`; translate content down by the top inset). The keyboard depth is the
portrait-reference height, scaled `×width/height` in landscape (mirrors
overlay_rect's `ime_depth`). `keyboard_base_px` is set via `set_keyboard_base`
from the drain loop, which re-issues `on_resize` so any guest re-lays-out.

Subtlety that bit us: the OLD code subtracted chrome insets from the PHYSICAL
height *before* the rotation, so in landscape they ate logical WIDTH, not the user
top/bottom — apps drew under the taskbar sideways (Signal showed it; Compose
masked it with its own content margins). The user-space model fixes it for all
guests; **verified Compose is not broken** in landscape.

Device-verified: Signal composer rides above the keyboard in portrait AND
landscape, and clears the taskbar when the keyboard is down. `recompute_transform`
auto-recomputes on rotation, so rotating with the keyboard up self-corrects.

### Not done (deferred follow-ups)
- ⌄ **Hide key → detach** (M3): dismissing the keyboard from its own button should
  also clear the inset (today it clears on focus-blur/back). Surface-only today.
  (Likely already works via Hide→ESC→editor-detach→arbiter `keyboard-inset 0`;
  unverified.)
- **Landscape keyboard sizing → task 71.** The IME now requests a fixed 864px
  (30% portrait); the host scales it `×pw/ph` to ~432px in landscape, which is too
  small to read. The clean fix is the percent-of-content model in
  `tasks/71-display-geometry-namespace.md` (IME sends 30/42% per orientation, host
  applies to `content-size().height`, drop the px-scaling). Until then landscape is
  undersized.
- IME computing its height as a **fraction of the panel** (resolution-independent)
  — folded into task 71.

---
(original problem statement below)


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
