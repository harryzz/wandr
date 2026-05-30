# Task 63 — App-driven orientation lock propagates to the chrome

🟡 **In progress 2026-05-30.** Follow-up to task 62 (overlay rotation). An app
can declare `orientation = "locked"` to stay portrait regardless of the phone's
position; when that app is **foreground**, the system chrome (status bar /
taskbar / IME) locks to portrait too, so the whole screen stays coherent.

## Problem

Task 62 made every surface follow the device's orientation sensor independently.
But a portrait-locked app needs the *chrome* to stay portrait as well — otherwise
the app stays upright while the bars rotate around it. The chrome overlays don't
know which app is foreground, so they can't decide this on their own.

Two gaps:
1. A *fullscreen* app's `orientation = "locked"` was ignored — the rotation gate
   was `rotation_policy() || mode == None`, so fullscreen always rotated.
2. No channel carried the foreground app's lock to the (directly-launched,
   non-arbiter-tracked) status bar / taskbar.

## Design

**Semantics** (`app_loader.rs`): the manifest `orientation` field is now read as
three states — `"auto"`, `"locked"`, absent:
- **fullscreen app**: rotates UNLESS explicitly `locked` (absent ⇒ rotates,
  preserving task-43); `orientation_locked()` is true only for explicit `locked`.
- **overlay**: rotates only on explicit `"auto"` (`rotation_policy()`), unchanged.

**Global lock signal** — a plain file `/data/local/tmp/wart-orient-lock` (`1` =
portrait-locked). The status bar / taskbar are launched directly (not
arbiter-tracked), so a per-socket push can't reach them; a global file they all
poll is the simplest reliable channel.
- The **foreground fullscreen app** publishes the lock from its own policy on its
  `AppRole::Foreground` transition (`standalone.rs`; overlays never write it).
  Self-corrects as apps swap foreground (the launcher publishes `0`, a locked app
  publishes `1`, the arbiter's fall-back-to-home restores `0` on exit).
- **Overlays** read it each frame in the rotation poll; the target orient is the
  device orient, except an overlay forced to `0` (portrait) while the lock is set.
  Driven by target-vs-current so it also re-applies when the lock toggles without
  the device moving (tracks `last_dev_orient`).
- A locked fullscreen app has no sensor (gate), so it simply stays portrait.

## Files

- `runtime/wart-host/src/app_loader.rs` — `orientation_locked()` + `orientation_field()`.
- `runtime/wart-host/src/standalone.rs` — gate (fullscreen respects locked),
  `run_cwasm_loop(orientation_locked)`, `publish_orientation_lock` /
  `orientation_lock_active` helpers, lock write on Foreground, lock-aware poll.

## Verify

Set a fullscreen app to `orientation = "locked"` (e.g. edit the installed
`com.example.wart-app` package.toml), foreground it, rotate the device →
the app AND the status bar / taskbar / IME all stay portrait. Switch to the
launcher (auto) → chrome follows the device again.

## Follow-up

- Landscape-lock (not just portrait) — the field is binary today; a future
  `orientation = "landscape"` (or a numeric lock) would publish a non-zero
  target orient. Not needed yet.
- Centralize via the arbiter if the status bar / taskbar ever become
  arbiter-tracked (then a typed socket push could replace the file).
