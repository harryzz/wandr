# Task 56 — system taskbar / navigation (proposal)

> **Status:** 🔲 proposal 2026-05-29, not started. Design options below;
> pick a proposal + scope before implementing.

## Why this matters

With SystemUI and the launcher force-stopped (tasks 33, 46), there is
**no way for the user to switch apps, go home, or go back** once an app
is foreground. The arbiter can launch/foreground/kill apps over its
socket (`wart-arbiter launch <id>` etc.), but only from an adb shell —
the on-device user has no UI for it. The taskbar is that UI: the wart
equivalent of Android's navigation bar **+** taskbar/dock, since wart
has no separate launcher.

This is the bottom-strip sibling of [`tasks/55-status-bar.md`](55-status-bar.md)
and shares its foundation (overlay surface + insets + `war:shell` WIT +
arbiter routing — see task 55's **Shared foundation**).

## What it is (scope of "taskbar, like Android")

Android splits this across the nav bar (back/home/recents) and, on
tablets/desktop mode, a taskbar (app dock + running apps). wart has one
bottom strip and no launcher, so propose **one combined bar**:

- **Launcher/dock:** icons for installed apps → tap launches /
  foregrounds. This replaces the missing launcher.
- **Running-app switcher:** the currently-running apps (the arbiter
  already tracks these in `state.rs`) → tap switches foreground.
- **Affordances:** a home action (show the dock / go to a default app)
  and optionally back (route a back event to the focused app).

## Data + actions are all arbiter-owned

The arbiter already is the authority here — it owns the running-app
registry (`state.rs`) and the launch/foreground/kill commands. The
taskbar is a *view + controller* over the arbiter. Two new data needs:

| Need | Source (ART-free) |
|------|-------------------|
| Installed apps (id, label, icon) | scan `<APPS_ROOT>/apps/*` + `system-apps/*`, read `package.toml` — needs a manifest extension for `label` + `icon` (see below). The installer/loader already enumerates these. |
| Running apps (id, pid, fg) | `wart-arbiter list` / the `state.rs` registry — already exists. |
| Launch / switch / kill | arbiter socket commands — already exist (`launch`, `foreground`, `kill`). |

No `system_server`, no PackageManager — consistent with
[[feedback_no_art_layer_dependencies]] and the roadmap (§6.3 PackageManager
dropped, replaced by the component-graph loader, which is exactly our
`app_installer.rs` install-dir layout).

**Manifest extension:** add optional `label` + `icon` (path to a PNG in
the warpkg's `assets/`, reusing task 38 asset bundling) to `package.toml`.
The taskbar reads them via a new host verb.

## Proposals

### Proposal A — taskbar as a guest warpkg (recommended)

A first-party `war.taskbar` warpkg (Kotlin/Compose) on a **bottom-strip
overlay SF surface**, launched by the arbiter at startup. It renders the
dock + running row in Compose and drives the arbiter via a new
`war:shell/launcher` host interface:

```wit
interface launcher {
    record app-entry { app-id: string, label: string, icon: u32, running: bool, foreground: bool }
    list-apps:     func() -> list<app-entry>;   // installed + running merged
    launch:        func(app-id: string);        // host → arbiter LAUNCH/foreground
    switch-to:     func(app-id: string);        // host → arbiter foreground
    close-app:     func(app-id: string);        // host → arbiter KILL
    go-home:       func();                       // show dock / default app
}
```

The host implements these by forwarding to the arbiter socket — the
same shape as `keyboard_host_impl.rs` routing IME input to the arbiter
in task 47/49.

- **Pros:** rich/themeable, consistent with IME + status bar, reuses
  skiko, icon rendering via the existing image path.
- **Cons:** another always-on Compose process; needs careful redraw
  discipline (only redraw on app-list change, not per frame).

### Proposal B — host-drawn taskbar (MVP / fallback)

Draw a minimal bar (text labels of running apps + tap targets) directly
in `wart-host`. No guest.

- **Pros:** tiny, no extra process, fast to prototype the
  switch-foreground flow.
- **Cons:** no icons/theming without reimplementing layout; grows badly.

### Proposal C — gesture-only navigation (complementary, not exclusive)

Instead of (or in addition to) a visible bar: bottom-edge swipe = home,
left-edge swipe = back, swipe-up-hold = recents. Routes through the
existing InputFlinger drain (task 33 step 3).

- **Pros:** zero screen real estate, modern Android-like.
- **Cons:** undiscoverable without hints; still want a visible dock for
  launching. Best as a **complement** to A (gestures for nav, the bar
  for the app dock).

**Recommendation:** **Proposal A** as the core (it's the only thing that
replaces the missing launcher), with **C's home/back gestures** as a
fast-follow once the bar works. B only if we want a throwaway MVP to
prove the arbiter round-trip from on-device taps.

## Coexistence with the IME (important)

The IME keyboard (task 47) is *also* a bottom overlay. The taskbar and
the keyboard can't both own the bottom strip at full height. Policy
(arbiter-owned):

- When an editor focuses and the IME shows, the taskbar **hides** (or
  the keyboard composites above it). Simplest v1: taskbar hides while the
  IME overlay is visible, reappears on `editor-detached`. The arbiter
  already knows both (active-ime + editor-focus state from task 49), so
  it can sequence the two overlays' visibility + the foreground app's
  bottom inset.

## Steps (Proposal A)

| # | Step | Where |
|---|------|-------|
| 1 | Bottom-strip overlay reuse (task 47 already has this) + a fixed taskbar height + bottom inset to the fg app (shared with task 55) | `sf_surface.*`, arbiter |
| 2 | `package.toml` `label` + `icon` manifest fields; installer records them; loader exposes them | `app_installer.rs`, `app_loader.rs` |
| 3 | `war:shell/launcher` WIT + host impl forwarding to the arbiter socket | `wit/shell.wit`, `wart-host/src/launcher_impl.rs` |
| 4 | Arbiter: `list-apps` (merge installed-scan + running registry), and have launch/switch/close reachable from the host forwarder | `wart-arbiter/src/{state,main}.rs` |
| 5 | `war.taskbar` warpkg — Compose dock + running row, icons from assets, on-change redraw | `apps/system/war.taskbar/` |
| 6 | Arbiter auto-launches the taskbar at startup; coexistence policy with the IME overlay | `wart-arbiter` |
| 7 | (fast-follow) home/back/recents edge gestures via the InputFlinger drain | `wart-host/src/standalone.rs`, `input.rs` |
| 8 | Device-verify: tap launches/switches/closes apps; IME and taskbar don't fight for the bottom; rotation re-lays it | device |

## Out of scope

- Split-screen / freeform windowing (single fullscreen app at a time for
  now — matches the locked Hybrid model).
- Drag-to-reorder dock, app folders, search.
- Per-app jump lists / shortcuts / notification badges (badges depend on
  task 55's notification subsystem).

## Open questions (decide before implementing)

1. **Core shape:** Proposal A (guest dock + switcher) — confirm. Add C
   (gestures) now or later?
2. **Icons:** require an `icon` in `package.toml`/assets, or auto-generate
   a letter tile from the label for v1 (avoids art for every app)?
3. **Home action semantics:** with no launcher, what is "home" — a
   dedicated dock view, or a designated default app?
4. **IME coexistence:** hide-taskbar-while-keyboard (simple) vs
   composite-keyboard-above-taskbar (nicer, more layout work)?

## Related

- [`tasks/55-status-bar.md`](55-status-bar.md) — top strip; shares the
  overlay + insets + `war:shell` foundation.
- [`tasks/47-ime-via-guest-app.md`](47-ime-via-guest-app.md) — bottom
  overlay surface + arbiter input routing (the pattern reused here).
- [`tasks/46-wart-arbiter-mvp.md`](46-wart-arbiter-mvp.md) — the arbiter
  registry + launch/foreground/kill commands the taskbar drives.
- [`tasks/35-app-install.md`](35-app-install.md) /
  [`tasks/38-warpkg-assets.md`](38-warpkg-assets.md) — install-dir
  enumeration + asset bundling (icons).
- `post-art-roadmap.md` §6.3 — PackageManager dropped → component-graph
  loader (our install dir is the app list).
