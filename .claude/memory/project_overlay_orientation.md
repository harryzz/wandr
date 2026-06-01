---
name: project-overlay-orientation
description: "Task 62 — overlay (IME/status bar/taskbar) rotation; manifest flag, anchor-aware overlay_rect, handedness, guest resize-fix, redeploy gotcha"
metadata: 
  node_type: memory
  type: project
  originSessionId: d33723e5-8289-42cf-b090-a027d6a8e217
---

**SUPERSEDED 2026-06-01 for the orient-lock + overlay-rotation source — see
[[project_chrome_coherence]] (commit 1da7adb0).** The `wart-orient-lock` FILE is
RETIRED; chrome/IME overlays no longer poll a sensor or the file — the arbiter is
the single orientation authority and pushes orient via `geometry`. The
anchor-aware `overlay_rect` handedness below is STILL current (the host applies it
on the arbiter-pushed orient). Orientation lock is now `set-orientation-lock` to
the arbiter, not a file.

Task 62 (device-verified 2026-05-30) made the IME, status bar, and taskbar
overlays rotate with the device, folding in sibling task 58 (chrome rotation).
Builds on task-43 fullscreen rotation ([[project-standalone-orientation]],
[[feedback-no-art-layer-dependencies]]).

**Generic rotation flag:** `package.toml` `orientation = "auto" | "locked"`
(default locked). Parsed in `app_installer.rs`; read at runtime by
`app_loader.rs::LoadedApp::rotation_policy()` (reads the installed package.toml,
like `assets_dir()`). NOT in the AOT cache-key — toggling doesn't invalidate the
.cwasm, so you can even edit the installed package.toml in place to flip it.
`standalone.rs`: `rotates = rotation_policy() || mode==None` (fullscreen always
rotates; overlays opt in). Fully generic — any warpkg uses the same flag.

**Anchor-aware placement** (`standalone.rs::overlay_rect`): the panel buffer is
fixed portrait; each chrome strip is placed at its USER-space edge. Device-verified
handedness (which physical edge is the user's BOTTOM for a content-rotation orient):
**0→South, 3→North, 4→West, 7→East**; user-top = opposite. status bar=user-top
(th=sb), taskbar=user-bottom (th=tb), IME=user-bottom offset `tb` inward (above the
taskbar). Landscape IME depth scaled `t*pw/ph` (~42% of screen, not 83%). If a strip
lands on the wrong side, swap the 4/7 arms — host-only, no shim rebuild.

**Shim (a-03 rebuild):** added `sf_set_overlay_geometry(x,y,w,h)` (superset of
`sf_resize_overlay`, which now calls it) + `sf_panel_dims(*w,*h)` (host needs
PANEL_H to size a vertical side strip; the overlay buffer is only strip-thick).

**IME guest needed a fix (plan was wrong that it didn't):** `war.ime.keyboard`'s
`Main.kt` render delegate discarded the per-frame `w/h` → ComposeScene stayed
portrait-1200 size → landscape overflow/clipped rows. Fix = the SAME task-43
wart-app fix: delegate updates `realScene.size` + `MutableSceneWindowInfo.containerSize`
on change. **Every Compose wasi guest that can rotate needs this** — see
[[feedback-host-side-transforms]]. Rust canvas guests (statusbar/taskbar) already
adapt via their `on_resize` handlers. Also: `canvas_impl.rs::resize()` now calls
`recompute_transform()` so logical dims track a buffer change without an orient change.

**Redeploy gotcha:** `pkill -f wart-host` is UNRELIABLE through the Magisk `su`
wrapper — repeated redeploys left 3 generations of every overlay stacked, which
visually corrupted the keyboard. Use `pkill -x wart-host` (exact name) + kill the
zygote (PPID 1 daemon; it reaps its forked children). The Magisk wart-stack module
is disabled (no runtime respawner). Bring the stack up with one clean manual
sequence (zygote → arbiter → set-home → statusbar/taskbar overlays → launch-overlay
IME → set-ime), not the blocking `run-hybrid-stack.sh` (its final `adb shell -t`
daemon hangs when backgrounded and its waiter relaunches overlays → duplicates).

**App orientation lock → chrome (task 63, device-verified 2026-05-30):** a fullscreen
app's `orientation = "locked"` now (a) keeps the app portrait — the gate is
`mode==None ? !orientation_locked() : rotation_policy()` (fullscreen rotates UNLESS
explicitly locked; absent still rotates, no task-43 regression), and (b) publishes a
global lock file `/data/local/tmp/wart-orient-lock` (`1`=portrait) from the FOREGROUND
fullscreen app on its `AppRole::Foreground` transition. Overlays read it each frame and
force portrait while set (tracking `last_dev_orient` so they re-apply when the lock
toggles without the device moving). A file (not a socket) because the status bar /
taskbar are launched directly, not arbiter-tracked. `app_loader.rs::orientation_locked()`.
Standing config: **launcher = locked** (home stays portrait + locks chrome), **apps =
auto**. See `tasks/63-app-orientation-lock.md`.
