---
name: project_keyboard_overlay_lifecycle
description: "Task 71 — IME/overlay sizing + lifecycle reworked to the Android-correct model (derived state, focus-follows-foreground, intrinsic dp). Device-verified 2026-06-01."
metadata: 
  node_type: memory
  type: project
  originSessionId: b4642c38-ac22-459b-92dd-7b4430418889
---

Task 71 grew from "display geometry namespace" into a full keyboard/overlay
sizing + lifecycle rework. **All device-verified by the user 2026-06-01.**
Uncommitted at time of writing.

**Sizing — no hardcodes ([[feedback_no_hardcoding]]):**
- IME keyboard height is DERIVED in the guest (`RealComposeApp.kt`): `maxRows ×
  ROW_HEIGHT_DP(48) + gaps`, `× window.getDensity()`, capped at
  `MAX_SCREEN_FRACTION(0.45) × display.displaySize().height`. Portrait → ~924px
  (≈32%, cap doesn't bite); landscape → caps to ~45% of the short edge (the cap's
  only job). `MAX_SCREEN_FRACTION` is the single landscape-size tunable.
- Host is a PURE APPLIER: `recompute_transform`/`overlay_rect` apply the reported
  px verbatim — deleted all per-orientation scaling (×pw/ph, ×w/h, ×11/10) AND
  the `MIN_CONTENT_PX` magic floor.
- Overlays can read the REAL panel: `display.display-size` returns
  `sf_panel_dims` (host `canvas_impl::set_panel_dims`), not the overlay's own
  strip surface — that's what lets the IME compute a screen fraction.

**Rotation crash fix:** the `clamp(48, vh)` in `crates/dioxus-canvas/src/lib.rs`
(scrollbar thumb) panicked `min>max` when `vh<48` after rotation → SIGILL in BOTH
Rust guests (signal, dioxus; Compose unaffected). Fixed with non-inverting
`proportional.min(vh).max(48.min(vh))`. The guest guards its own clamp — NOT a
host floor.

**Overlay lifecycle = derived state (the real design win):** IME visibility is
reconciled in ONE place, `reconcile_overlay()` in wandr-arbiter `main.rs`:
`desired = active_ime && editor_focus && focus.pid == visible_app && ime != app`.
Every transition (attach/detach/cycle/launch/foreground) just updates state then
calls it. Killed the scattered imperative promote/demote/re-engage that caused
cross-app yank + stuck cycle. Mirrors Android: **editor focus is a child of
window focus is a child of foreground.** `drop_editor_focus_of(pid)` =
Android `finishInput()` — switching apps drops the old app's editor focus (so no
stale-focus yank) AND sends `keyboard-inset 0` to restore its layout (else a
BLANK GAP where the keyboard was). `OverlayState.ime_pid`→`overlay_pid`
(pid-keyed, overlay-agnostic — ready for future side/utility panels, not just IME).
`active_app_pid()` = behind-app when overlay engaged, else foreground (so the
cycle ring uses the visible app, not the IME).

**WMS-authority slice:** arbiter `send_present(pid)` → host `present` inbound
event (`ime_inbound.rs` + `standalone.rs`) forces a repaint when a surface is
shown, instead of relying on the async SIGUSR2 role signal alone. First real bit
of WMS moving into the arbiter (see [[project_arbiter_window_server_design]]).

**Correct end behavior (Android model):** switch apps → keyboard DISMISSES,
content fills down (no blank strip); re-tap a field → keyboard returns. Not a bug
— that's how Android works.

**Deploy discipline learned the hard way:** per-app `wandr-host --install` only;
NEVER `build-system-wandrpkgs.sh` (wipes APPS_ROOT — destroyed user Signal state
once) and NEVER `rm -rf` an app's `cache/` dir. See
[[feedback_build_system_wandrpkgs_wipes_apps_root]], [[feedback_dont_delete_app_cache_dir]].
