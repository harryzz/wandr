# Task 56 — system taskbar / navigation

> **Status:** ✅ device-verified 2026-05-29 — see **Results** below. The
> original proposal (full Compose dock + app switcher) was descoped: the
> dock/launcher role is already filled by the separate `war.launcher`
> (task 57), so task 56 shipped as the **navigation half** only — a
> minimal Android-style Back/Home/Recents nav bar as a LIGHT Rust canvas
> guest (the user's "3 icons only, rust app, like android" directive).
> The original design options are preserved below for reference.

## Results (v1 — device-verified 2026-05-29)

Shipped a three-button nav bar — **Back (◀) · Home (●) · Recents (■)** —
as `apps/system/war.taskbar/`, a ~68 KB Rust `wasm32-wasip2` canvas guest
(no Kotlin/Compose → leak-immune + tiny, mirroring `war.launcher` /
`war.statusbar`). It runs on a thin always-visible **bottom-strip overlay**
and forwards taps to the arbiter.

**Architecture (reused the status-bar pattern):**
- New `OverlayMode::BottomBar` in `standalone.rs` — bottom-anchored, fixed
  height `WART_TASKBAR_PX` (default 150 px, env-tunable), launched as a
  **direct standalone process** (`wart-host --standalone-overlay-bottom-bar
  --app war.taskbar`), exactly like the status bar's top overlay. No zygote
  command needed.
- **Z-stack reservation:** fullscreen apps at `0x40000000`, the taskbar at
  `0x60000000` (above apps, below chrome), the IME + status bar at
  `i32::MAX` — so the keyboard draws over the taskbar when typing, and the
  taskbar draws over the foreground app.
- **Nav routing:** extended the existing `my:skiko-gfx/launcher` host
  interface with `go-home` / `go-back` / `recents`; `launcher_impl.rs`
  forwards each to the arbiter socket. Two new arbiter commands:
  - `cycle-task` — foreground the next running user app (chrome overlays
    `war.statusbar` / `war.taskbar` / `war.ime.keyboard` excluded from the
    ring; the launcher participates so cycling wraps through home).
  - `back` — route an ESC key (Compose key-id 27) to the foreground app's
    control socket (apps treat it as dismiss/escape; full Android
    `OnBackPressedDispatcher` driving is deferred — ESC is the honest v1
    stand-in). `go-home` already existed (task 57).
- **Icons drawn as shapes, not glyphs:** the device sans-serif lacks the
  geometric-shapes Unicode block (◁○□ → tofu), so the icons are drawn via
  `draw-path` (left triangle) / `draw-oval` (circle) / `draw-rect` (square)
  added to the guest's trimmed WIT. A tapped button flashes a translucent
  pill for ~8 frames.

**Device-verified:** all three buttons work end-to-end via real
`input tap` on the bottom strip → taskbar guest `on_pointer_event_v2`
hit-test → `launcher::{go_back,go_home,recents}` → host forwarder →
arbiter action. Logs confirm `cycle-task → fg=war.launcher (ring of 2)`,
`back → ESC delivered to fg`, and `go-home`. The complete wart shell now
runs with no ART SystemUI/launcher: status bar (top) + launcher + taskbar
(bottom) + IME, all light Rust canvas guests plus the Compose demo app.

**Files:** `apps/system/war.taskbar/{Cargo.toml,wit/taskbar.wit,src/lib.rs}`;
`runtime/wart-host/src/standalone.rs` (`BottomBar` mode + `taskbar_height_px`
+ z-layer), `runtime/wart-host/src/main.rs` (`--standalone-overlay-bottom-bar`),
`runtime/wart-host/src/launcher_impl.rs` (nav verbs → arbiter),
`runtime/wart-arbiter/src/main.rs` (`back` + `cycle-task` cmds),
`wit/skiko-gfx.wit` (launcher nav verbs), `tools/scripts/{build-system-warpkgs,
run-hybrid-stack}.sh`.

**Deferred (v1 follow-ups):**
- Full Android Back via the guest `OnBackPressedDispatcher` (currently ESC).
- A proper recents UI (thumbnail switcher) — v1 just cycles the fg ring.
- Bottom content-inset for the foreground app (launcher tiles are
  top-aligned so nothing is hidden today; add `on-insets-changed` when an
  app's content reaches the bottom 150 px).
- IME/taskbar coexistence is z-order only (keyboard covers the taskbar
  when up); no explicit hide/sequence.
- Edge-gesture navigation (proposal C).

---

## Original proposal (2026-05-29 — preserved for reference)

> Design options below; the implementation above descoped to the nav-bar
> half (Rust, 3 icons) per the build directive.

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
