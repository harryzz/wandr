# Task 62 — Orientation for overlay surfaces (IME / status bar / taskbar)

✅ **Device-verified 2026-05-30.** Spun out of task 61 testing — the user
observed the soft keyboard didn't rotate with the device. Resolved via Option
**A** (geometry-flip in the shim), generalized to a per-package manifest flag,
and the sibling **task 58** (status bar + taskbar rotation) was folded in once
the abstraction made it a small extension. All three overlays + the fullscreen
app now rotate coherently.

## What shipped

### 1. Generic per-package rotation flag

- `package.toml` gained `orientation = "auto" | "locked"` (default `locked`),
  parsed in `app_installer.rs` (`enum Orientation`, `Manifest.orientation`) and
  read at runtime by `app_loader.rs::LoadedApp::rotation_policy()` (reads
  `<install_dir>/package.toml`; mirrors `assets_dir()`). **Not** in the
  AOT cache-key — pure policy, so toggling it doesn't invalidate the `.cwasm`.
- `standalone.rs` gate: `let rotates = loaded.rotation_policy() || mode == None;`
  — fullscreen always rotates (task 43), overlays rotate iff their manifest opts
  in. The flag is fully generic: any warpkg enables rotation the same way.
- Opted in: `war.ime.keyboard` (`pack-ime-keyboard.sh`), `war.statusbar`,
  `war.taskbar` (`build-system-warpkgs.sh`).

### 2. Generic shim geometry (a-03 `libsf_surface.so` rebuild)

- New `sf_set_overlay_geometry(x,y,w,h)` — a superset of `sf_resize_overlay`
  (moves AND resizes; the latter reimplemented on top of it). New
  `sf_panel_dims(*w,*h)` so the host knows `PANEL_H` to size a vertical side
  strip (the overlay buffer is only strip-thick). Rust wrappers + `panel_w/
  panel_h` in `sf_surface.rs`. Built on a-03 via the direct-ninja path;
  `llvm-nm` confirmed both symbols exported.

### 3. Unified anchor-aware placement (`overlay_rect`)

`overlay_rect(mode, orient, pw, ph, t, sb, tb)` places each chrome strip at its
**user-space** edge in the fixed portrait buffer. Which physical edge is the
user's bottom is device-verified handedness: **0→South, 3→North, 4→West,
7→East** (user-top = opposite). A strip is `th` thick, full-span along the edge,
pushed `off` inward:

- **status bar** (`Top`) → user-top, `th = sb`, `off = 0`.
- **taskbar** (`BottomBar`) → user-bottom, `th = tb`, `off = 0`.
- **IME** (`Bottom`) → user-bottom, `th = depth`, `off = tb` (sits above the
  taskbar). Landscape depth is scaled `t·pw/ph` (~42% of the screen, not 83%).

On a rotation event the loop calls `sf.set_overlay_geometry` → `renderer.resize`
(rebuilds the GL buffer; `resize` now also `recompute_transform`s so logical
dims track) → `set_orientation` (content pre-rotation) → `on_resize`. Touch is
inverse-mapped via `base_matrix.invert()` (unchanged). The portrait
`request-overlay-height` path was rerouted through `overlay_rect` too, so the
keyboard sits above the taskbar in portrait as well.

### 4. IME guest resize fix (the plan's wrong assumption)

The plan assumed the IME guest needed no changes. **Wrong** — `war.ime.keyboard`'s
`Main.kt` render delegate discarded the per-frame `w/h`, so the `ComposeScene`
stayed at its startup (portrait 1200-px) size and overflowed/clipped the rotated
surface (rows ran off-panel). Applied the same fix wart-app got in task 43:
delegate now updates `realScene.size` + a new `MutableSceneWindowInfo.containerSize`
on change. The keyboard's `weight(1f)` rows then fill whatever depth they're given.
(The Rust canvas bar guests already adapt via their `on_resize` handlers.)

## Device-verified iteration log

1. First handedness guess was mirrored — keyboard landed left in landscape; swapped
   `4`/`7` edge arms (host-only, no shim rebuild). See [[project-standalone-orientation]].
2. Landscape depth too large (83%) → scaled `t·pw/ph`.
3. Keyboard covered the status bar → safe-area top clip, then generalized.
4. Keyboard rows clipped → root cause was the IME guest scene-size bug (#4 above).
5. Keyboard covered the taskbar → safe-area / `off = tb` inset.
6. Repeated redeploys left **stale stacked overlay processes** (3 of each) which
   corrupted the visible keyboard. `pkill -f wart-host` is unreliable through the
   Magisk `su` wrapper — use `pkill -x wart-host` + kill the zygote (it reaps its
   forked children). Bring the stack up with a single clean sequence.

## Files

- `runtime/wart-host/src/{app_installer,app_loader,standalone,canvas_impl}.rs`
- `runtime/wart-host/cpp/sf_surface.{cpp,h}` + `src/sf_surface.rs` (a-03 rebuild)
- `apps/system/war.ime.keyboard/src/wasmWasiMain/kotlin/{Main,RealComposeApp}.kt`
- `tools/scripts/{pack-ime-keyboard,build-system-warpkgs}.sh`

## Follow-up — app-driven orientation LOCK propagates to chrome

🔲 Not built. An app can declare `orientation = "locked"` today (stays portrait,
ignores the device). The missing piece is **cross-process propagation**: when the
**foreground** app is locked, the system chrome (status bar / taskbar / IME) should
lock to the same orientation too, rather than each following the device sensor
independently. The arbiter owns the foreground app, so it's the right place to push
a "locked orientation = N" signal to the overlay processes (over the existing
per-host control sockets), which would override their sensor-driven rotation. This
keeps the whole screen coherent when a locked app is up. Natural next task.
