---
name: project_task74_surface_role_model
description: Task 74 — core per-display surface/role + resource-focus model in wart-arbiter-core (the load-bearing schema). A+B+C-write-through committed+device-verified; D/E remain. Resume here.
metadata: 
  node_type: memory
  type: project
  originSessionId: d2da507a-7acc-449f-bdc9-0c9775ae1c20
---

**Task 74 (Increment 2, 2026-06-01)** — build the load-bearing core schema the
design doc ([[project_arbiter_window_server_design]]) flags as where design
effort belongs: the per-display **surface/role model + generalized
resource-focus** in `wart-arbiter-core`, dissolving the overlay↔foreground knot.
Follows the strangler plan A→E. **A+B+C-write-through DONE, committed+pushed,
device-verified (Pixel 2 XL); D/E remain.**

**Commits (codeberg main):** `9b21c035` (A+B), `d7962f6c` (C write-through).
After `d078ab88` (task 73).

**What's built + proven (the architecture):**
- `wart-arbiter-core/src/surface.rs`: `Surface{pid,app_id,role}`, `Role`
  (Foreground/OverlayBehind/Overlay/Background/Chrome/Headless), generalized
  `ResourceFocus` map (`ImeEditor` now; Window/Input/Audio/Notification are
  additive `+1` variants), per-display `DisplayState{geometry,surfaces,focus,
  active_ime}` with the **pure derivations** `visible_app()` + `overlay_desired()`.
  **The knot dissolves:** the IME is a `Role::Overlay` surface, the app a
  `Role::OverlayBehind` surface; "what's visible" is derived, not a foreground
  slot that lies.
- `core/lib.rs`: Store holds `DisplayState` (geometry nested; `geometry()`/
  `geometry_mut()` accessors); `Event::SurfaceRemoved`; the **`Effect` contract**
  (SetRole/Launch/Kill/Persist/HostLine) + `Ctx::request` — modules declare
  mechanism, the binary performs it. Registry dispatch returns `Vec<Effect>`.
- `bin/main.rs`: `execute_effects` + `apply_role` (Role→signal/oom/present — the
  ONE place Role→OS lives). The model is the **live single source** for the
  read/decision path: `active_app_pid()`=`visible_app()`,
  `reconcile_overlay`'s desired=`overlay_desired()`; maintained by **write-through**
  at every mutation site (`model_put_surface`/`set_role`/`remove_surface`/
  `set_active_ime`/`set_editor`), seeded once at daemon startup. `mirror_and_check`
  canary compares the live model to the legacy formula.
- `wart-arbiter-wm`: geometry accessor rename only (caches NOT yet removed = E).

**Device proof:** ZERO MODEL-DRIFT + ZERO OVERLAY-DESIRED-DRIFT across
launch/switch/cycle/overlay-engage+disengage/foreground/kill/go-home; the model
correctly returns the behind app (not the IME) during a split. Host wire
unchanged throughout (so parity is checkable at the wire).

**REMAINING (well-scoped; legacy singletons are still dual-written + read in ~15
sites — redundant but harmless, so behavior is unaffected):**
- **D (collapse dual-state):** migrate the ~15 legacy READ sites to model
  read-helpers (cmd_list fg/ime/editor markers, cmd_back, go-home/
  ensure_home_foreground, cmd_attach/detach IME+editor lookups, cmd_ime_route,
  cmd_overlay behind-hint, handle_child_exit overlay, `save_to` fg app-id);
  rewrite promote_to_foreground/promote_to_overlay/demote_from_overlay to source
  prev-fg/overlay from the model; THEN delete the 4 singletons + ActiveIme/
  EditorFocus/OverlayState structs + `legacy_active_app_pid`/`legacy_overlay_desired`
  canaries. Keep legacy until reads are migrated (canary stays green), delete
  last. Optionally move registry/home/persistence into the Store + have the
  death-watcher emit `Event::SurfaceRemoved`.
- **C-module extraction:** move the orchestration + verbs into a new
  `wart-arbiter-shell` crate (an `ArbiterModule` emitting `Effect::SetRole` via
  `Ctx`), registered with one line — the Open/Closed payoff.
- **E (WM cache removal):** drop WM's `focused_editor`/`foreground_pid`; read
  `store.ime_editor()`/`visible_app()`. **Gotcha:** write-through now populates
  the store BEFORE the trigger events fire, so naively reading the store in both
  the `ImeHeightChanged` and `EditorFocusChanged` handlers double-pushes on
  attach. Read the store only where there's no event payload (ImeHeight/
  Orientation); on detach push keyboard_px=0 to `visible_app()` (the editor's
  app, post overlay-teardown). Verify no double on_resize.

Build: `build-host-android.sh`; deploy: `run-hybrid-stack.sh`; NEVER
`build-system-warpkgs.sh`. Host is arbiter-only across all of task 74. Plan file:
`~/.claude/plans/cat-task-state-goofy-cake.md`.
