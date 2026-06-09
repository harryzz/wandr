---
name: project_task73_modular_arbiter_wm
description: "Task 73 — modular wandr-arbiter-core + WM geometry move (insets/keyboard/orientation host→arbiter). Coded+builds, NOT device-verified. Resume/verify here."
metadata: 
  node_type: memory
  type: project
  originSessionId: d2da507a-7acc-449f-bdc9-0c9775ae1c20
---

**Task 73 (2026-06-01, "task A")** — first real slice of the
[[project_arbiter_window_server_design]]: stand up the modular arbiter kernel and
move WMS geometry from the per-app host into the arbiter. **DEVICE-VERIFIED
2026-06-01 on Pixel 2 XL (UNCOMMITTED)**; builds clean (arbiter 952K + host 56M
aarch64; 7 unit tests; clippy clean). Logcat proof: keyboard via `geometry`
(kb=0→924→0, logical reflow 2598→1674→2598 = byte-equiv old keyboard-inset);
orientation report-up→decide→push-back (`report-orientation 3`→host orient=7,
logical→landscape 2880x510, chrome flips coherently, rotate-back→0); insets=M1
sentinel 65535 (host keeps env 132/150). `wandr-arbiter list` works, no
panics/unknown-verb. Launcher renders clean. STILL UNTESTED physically: locked-app
stays portrait + kill-arbiter-then-rotate local fallback (logic in place).

**What was built**
- `runtime/wandr-arbiter/` → virtual **workspace** (`.cargo/config.toml` stays at
  root; artifact path unchanged → build/deploy scripts untouched). Members:
  - `wandr-arbiter-core` (lib): per-display `Store`, full `Event` vocab, `Ctx`
    (emit + deliver_to_host), `ArbiterModule` trait, `Registry` (try_dispatch +
    event cascade to fixpoint). Sentinels `ORIENT_HOST_OWNED=255`,
    `INSET_HOST_OWNED=0xFFFF`.
  - `wandr-arbiter-wm` (lib): owns `report-orientation` verb; reacts to
    `ImeHeightChanged`/`EditorFocusChanged`/`ForegroundChanged`/`OrientationChanged`;
    pushes ONE wire line `geometry <inset_top> <inset_bottom> <keyboard_px> <orient>`.
    Holds targeting caches (focused_editor, foreground_pid) learned from events.
  - `wandr-arbiter-bin`: legacy `match verb` INTACT; `module_owns(verb)` probed
    first → `dispatch_module`; `bus_emit()` bridges. 5 legacy→bus bridges:
    cmd_ime_overlay_height, cmd_attach_editor (2-emit: ImeHeightChanged height-sync
    then EditorFocusChanged), cmd_detach_editor, drop_editor_focus_of,
    promote_to_foreground. All `keyboard-inset` SENDS replaced by `geometry`.
- Host (dumb applier; skia dihedral matrix stays local): `InboundEvent::Geometry`
  + parser in `ime_inbound.rs` (sentinels `GEOM_INSET_KEEP`/`GEOM_ORIENT_KEEP`);
  drain applier + orientation report-up in `standalone.rs`. Orientation:
  fullscreen apps send `report-orientation <raw>` to the arbiter, HOLD for the
  decided orient via `geometry` push-back; **400ms timeout backstop + arbiter-down
  → apply locally** (no hang, no regression if arbiter absent). Overlays keep
  their local sensor + `/data/local/tmp/wandr-orient-lock` path.

**As-deployed staging (one build; not separately deployable):**
- **Insets = M1** (safe, no change): `run-hybrid-stack.sh` launches the arbiter
  WITHOUT `WANDR_INSET_*` (only the zygote gets them, line ~145) → WmModule sends
  `INSET_HOST_OWNED` → host keeps its env insets (132/150). **To activate M2**
  (arbiter authors insets): add `WANDR_INSET_TOP=.. WANDR_INSET_BOTTOM=..` to the
  `wandr-arbiter --daemon` launch env (run-hybrid-stack.sh ~L179/L190).
- **Keyboard** via `geometry` = byte-equivalent to the old `keyboard-inset`.
- **Orientation = M3 ACTIVE** on deploy, with the fallback as the safety net.

**Device verify (user runs):** build-host-android.sh → run-hybrid-stack.sh.
1. `wandr-arbiter list` works (legacy match intact). 2. focus an editor → keyboard
shows + content reflows; resize/hide OK (geometry/ImeHeightChanged path).
3. rotate device → app rotates; a `orientation=locked` app stays portrait; kill
the arbiter then rotate → host still rotates (local fallback). 4. `wandr-arbiter
overlay` → strip geometry unchanged (`overlay_rect` untouched). 5. logcat: watch
`wandr-arbiter-wm:` lines + `ime-inbound: unknown verb` (protocol skew).

**Deferred (Increment 2+):** panel-dims/density report-up → true-dp model;
`overlay_rect` relocation into WM; migrate state.rs role/overlay/ime into the
Store; `-am`/`-ime` crates; fan orientation out to chrome (statusbar/taskbar are
NOT arbiter-tracked — still independent sensor+lock-file this increment).

Plan file: `~/.claude/plans/cat-task-state-goofy-cake.md`. NB: codebase "task 72"
= background-connection-floor (different); this is **task 73**.
