---
name: project_task74_surface_role_model
description: Task 74 — per-display surface/role + resource-focus model in wart-arbiter-core, now the SOLE arbiter state; orchestration extracted to a wart-arbiter-shell module; WM reads the Store. COMPLETE + device-verified.
metadata:
  node_type: memory
  type: project
  originSessionId: d2da507a-7acc-449f-bdc9-0c9775ae1c20
---

**Task 74 (Increment 2) — COMPLETE + device-verified (Pixel 2 XL, 2026-06-01.)**
Built the load-bearing core schema the design doc
([[project_arbiter_window_server_design]]) flagged as where design effort belongs
— the per-display **surface/role model + generalized resource-focus** in
`wart-arbiter-core` — then collapsed the legacy state onto it and reaped the
Open/Closed payoff. The overlay↔foreground knot is dissolved: the IME is a
`Role::Overlay` surface, the app a `Role::OverlayBehind`; "what's visible" is the
pure derivation `visible_app()`, not a foreground slot that lies.

**Commits (codeberg main), after `d078ab88` (task 73):**
- `9b21c035` (A+B core schema + drives reads), `d7962f6c` (C write-through) — the
  model became the live read/decision source while legacy singletons stayed as a
  parity canary.
- `a9f6ba73` **(D)** — deleted the 4 legacy singletons
  (`foreground`/`active_ime`/`editor_focus`/`overlay_state`) + structs + the
  `legacy_*` canaries; migrated ~15 read sites + the promote/demote/reconcile
  write helpers onto the model; `seed_surface_model` at boot. Model is **sole**
  arbiter state.
- `c8d00643` **(C1)** — moved app-registry + home + ime-height + hand-rolled-JSON
  persistence into the core `Store` (`wart-arbiter-core/registry.rs`); `state.rs`
  became a thin shim. **Caught a reentrant-lock deadlock**: a fn holding the
  `core_store` lock must NOT call `state::*` (now Store-backed → re-locks); fixed
  `seed_surface_model` to read `store.apps_snapshot()` from the held guard.
- `4272848b` **(C)** — extracted the AMS+IMMS orchestration into a new
  `wart-arbiter-shell` `ArbiterModule` (foreground/kill/set-ime/attach/detach/
  ime-*/back/cycle-task/overlay/overlay-clear/list + promote/demote/reconcile/
  drop-editor-focus + `on_event(SurfaceRemoved)` pruning), emitting
  `Effect::SetRole`/`HostLine`/`Kill`. **Registered with one line.** Zygote-coupled
  verbs (launch*/preload, go-home/set-home via `ensure_home_foreground`) stay in
  the binary and bridge via `dispatch_foreground` ("foreground" → module);
  `handle_child_exit` injects `Event::SurfaceRemoved` (module prunes) then does the
  binary-side home-fallback launch. Reply-text change: editor delivery deferred via
  `Effect::HostLine`, so attach/detach/ime-route report `routed` not synchronous
  `delivered` (safe — host's `send_oneshot` drops the reply).
- `5c9f3cb4` **(E)** — deleted WM's `focused_editor`/`foreground_pid` caches; it
  reads `ime_editor()`/`visible_app()` from the Store for the no-payload re-push
  handlers (ImeHeight/Orientation). **`Event::EditorFocusChanged` changed to
  `{editor: i32, focused: bool}`** so a blur push targets the editor that lost
  focus even after the Store focus is cleared — the focus-follows-foreground blur
  must un-shrink the *backgrounded* editor, NOT `visible_app()`. Focus-gain is a
  WM no-op (the co-emitted `ImeHeightChanged` does the inset push), avoiding a
  double-push on attach.

**Architecture now:** `wart-arbiter-core` = Store (per-display surface/role +
resource-focus + registry/home/ime-height) + Event/Effect/Ctx + Registry +
ArbiterModule. Modules: `wart-arbiter-wm` (geometry) + `wart-arbiter-shell`
(AMS+IMMS). The binary owns only transport + mechanism (`execute_effects`/
`apply_role` = the one place Role→signal/oom/present lives) + zygote calls +
death-watcher threads + persistence file IO. Adding a responsibility = +1 crate,
+1 `reg.register(...)` line.

**Device proof:** knot resolves (visible_app = behind app, never the IME); overlay
engage/disengage, cycle-task off the visible app, launch bridge, kill
(`Effect::Kill`→zygote), and death-watcher home-fallback all correct; logs tagged
`wart_arbiter_shell`; no panic/drift/dispatch-miss. Keyboard: attach → exactly ONE
geometry push (`kb=1200`, no double); detach → `kb=0` (host logical height
restored); cycle-away-with-keyboard → the backgrounded editor (not the launcher)
gets `kb=0`. Host wire unchanged throughout.

**Tests:** 17 unit tests (core 9 + shell 3 + wm 5, incl. `blur_targets_carried_pid`
regression). Build `tools/scripts/build-host-android.sh`; deploy `run-hybrid-stack.sh`
or an arbiter-only restart (push binary + `pkill -9 -f wart-arbiter` + restart
detached — re-attaches running apps from `state.json`); **NEVER**
`build-system-warpkgs.sh`. Host is arbiter-only across all of task 74.

**Deferred (cosmetic / additive, not blocking):** `state.rs` is still a thin
registry/home/persistence shim (binary uses it; fully deleting it is harmless
cleanup). Chrome-coherence (statusbar/taskbar as `Chrome` surfaces), overlay_rect
relocation, panel/density report-up, audio/notification `ResourceFocus` variants,
and the `-am`/`-ime` crate split are all now purely additive on this model. Plan
file: `~/.claude/plans/cat-task-state-steady-stallman.md`.
