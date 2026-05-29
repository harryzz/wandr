# Task 57 — system launcher / home (proposal)

> **Status:** 🟢 device-verified 2026-05-29 (Proposal A, proven in
> wart-app). Steps 1–5 done; Step 6 (boot-script auto-`set-home`) + a
> dedicated `war.launcher` warpkg are the remaining follow-ups.
>
> ## Results (2026-05-29)
>
> Built Proposal A. Per [[feedback_prefer_wart_app_edits]] the launcher
> UI was proven inside wart-app (a `LauncherCard` at the top of the
> showcase) rather than spinning up a brand-new Kotlin module first.
>
> - **Step 1 — arbiter home-app concept** (committed `7d6575f3`):
>   `home_app_id` state + persistence; `set-home`/`go-home` commands;
>   `ensure_home_foreground()`; boot-foreground; and the
>   fall-back-to-home hook in `handle_child_exit` (task 54). Verified:
>   set-home launches+foregrounds; killing the fg app auto-relaunches
>   home (instant, via the death-notification subscriber); arbiter
>   restart restores home from `state.json` + foregrounds the survivor.
> - **Steps 2–4 — host data/plumbing:** new `my:skiko-gfx/launcher` WIT
>   (`list-apps -> string` newline/TAB-delimited; `launch-app(app-id)`);
>   `wart-host/src/launcher_impl.rs` scans `<APPS_ROOT>/apps/*` for the
>   id + top-level `label` from each `package.toml` (the manifest is
>   **flat**, not `[package]`-nested — note vs the proposal text), and
>   forwards `launch-app` to the arbiter socket (one-shot, mirrors
>   `ime_host_impl`). No installer change (package.toml copied verbatim;
>   serde ignores the unknown `label`).
> - **Step 5 — launcher UI in wart-app:** hand-written canonical-ABI
>   bindings `LauncherImports.kt` (modeled on `AssetsImports.kt` — string
>   param + caller-allocated 8-byte return area for the string return);
>   `LauncherCard.kt` renders a `FlowRow` of letter-tile icons (first
>   label char on a hash-derived color) over a theme-gradient backdrop,
>   tap → `launchApp`. wart-app's `package.toml` gains `label = "Demo"`.
> - **Device-verified:** `set-home com.example.wart-app` → boots to the
>   launcher card; tile renders **"D" / "Demo"** (label resolved); tapping
>   it logs the full chain guest `tap → launch` → host `forwarded launch`
>   → arbiter `launched → pid … / promoting foreground`.
>
> **Remaining (small follow-ups):** Step 6 — `run-hybrid-stack.sh` /
> Magisk auto-`set-home` at boot (the home concept + persistence already
> make boot-to-home work once set). And a dedicated full-screen
> `war.launcher` warpkg (the wart-app card proves the path; a real
> launcher app is the clean spin-out — its richer demo value is gated on
> having ≥2 GUI apps to launch anyway). Letter-tile icons + theme
> gradient shipped as the v1 choices.

---

> **Original proposal (kept for history):** 🔲 proposal 2026-05-29.

## Why this matters

Right now a standalone wart device boots to **nothing usable** — the
Hybrid stack comes up (zygote + arbiter) but no app is foreground until
someone runs `wart-arbiter launch <app-id>` from an adb shell. There is
no home screen, no app grid, and no "home" to return to when an app
exits or is killed. The launcher is what makes wart a *usable device*
rather than a dev harness:

- the **boot target** (what shows when the stack starts),
- the **home** the user returns to (go-home, or an app exits/LMK-dies),
- the **app grid / drawer** to launch installed apps without adb.

The roadmap frames this as part of "runtime draws everything" (§6.3,
SystemUI/launcher dropped) — wart provides its own.

## Launcher vs taskbar (task 56) — they're different things

Android (and wart) have two distinct surfaces; don't conflate them:

| | Launcher (this task, 57) | Taskbar / nav (task 56) |
|---|---|---|
| Surface | **Fullscreen** app, takes the foreground like any app | **Overlay** strip, always composited on top |
| Visibility | Visible only when it's foreground (boot / home) | Persistent across apps |
| Role | App grid + wallpaper + "home" | Switcher + quick dock + back/home affordances |
| Process | A normal fg warpkg (`war.launcher`) | An overlay warpkg (`war.taskbar`) |

They **share** the app-enumeration + `war:shell/launcher` WIT + the
manifest `label`/`icon` fields (defined in task 56). The launcher is the
full home experience; the taskbar is the persistent quick bar. You can
ship either first — the launcher has no overlay/insets dependency, so
it's the cheaper, higher-payoff one (see **Prioritization** note below).

## Key new concept: the arbiter "home app"

The launcher forces the arbiter to learn one new idea — a designated
**home app** — and three behaviors around it:

1. **Boot:** at Hybrid-stack startup the arbiter foregrounds the home
   app automatically (no adb needed). Device boots to a home screen.
2. **Go-home:** the taskbar/gesture `go-home` action (task 56)
   foregrounds the home app.
3. **Fallback on empty:** when the foreground app exits or is killed,
   the arbiter foregrounds the home app instead of leaving a black
   screen. **This plugs directly into task 54** — `handle_child_exit`
   already fires when the fg app dies; it just needs to foreground the
   home app afterward instead of clearing fg to nothing.

New arbiter state: `home_app_id` (set via `wart-arbiter set-home
<app-id>`, persisted in `wart-arbiter-state.json`).

## Proposals

### Proposal A — full-screen home app (recommended)

A first-party `war.launcher` warpkg (Kotlin/Compose), a normal
**fullscreen** foreground app the arbiter designates as home. It renders:

- an **app grid** of installed apps (icons + labels), tap → launch /
  foreground via the shared `war:shell/launcher` WIT (task 56);
- a **clock** (host wall-clock, same source as the status bar);
- a **wallpaper** — a static image asset or solid/gradient (WallpaperManager
  is dropped per roadmap §6.3; a live/extracted wallpaper is out of scope).

- **Pros:** the real thing; uses the existing fullscreen surface path
  (no overlay/insets/shim work — unlike 55/56); reuses skiko + the app
  enumeration; the natural boot target.
- **Cons:** another warpkg; needs the arbiter home-app concept (but that
  synergizes with task 54).

### Proposal B — app-drawer overlay (no dedicated home)

No fullscreen home; instead "home/launch" pops an **app-drawer overlay**
(invoked from the taskbar or a gesture) over whatever's foreground. When
nothing is foreground, show a bare wallpaper + clock drawn by the host.

- **Pros:** no always-around home process; lighter; the drawer is just a
  transient overlay.
- **Cons:** no real home screen; "what shows at boot / when an app dies"
  is an awkward bare host-drawn screen; widgets/wallpaper have nowhere to
  live. Weaker as a device experience.

### Proposal C — merge launcher into the taskbar (rejected as the launcher)

Make the taskbar dock the only app-launching surface; no separate home.

- **Rejected for the launcher role:** still leaves "what's foreground at
  boot / after an app dies" unanswered, and there's no room for an app
  grid in a strip. The taskbar dock (task 56) is a *complement* to the
  launcher, not a replacement.

**Recommendation:** **Proposal A.** It's the cheapest of the three
shell tasks to build (no overlay/insets/a-03 shim work — it's a plain
fullscreen warpkg) and the highest user payoff (the device becomes
usable without adb). The arbiter home-app concept is small and dovetails
with task 54's death-notification hook.

## Steps (Proposal A)

| # | Step | Where |
|---|------|-------|
| 1 | Arbiter `home_app_id` state + `set-home <app-id>` command + persist in state.json | `wart-arbiter/src/{state,main}.rs` |
| 2 | Arbiter foregrounds home at startup; `go-home` command; **fallback-to-home in `handle_child_exit`** (task 54 hook) when fg app dies | `wart-arbiter` |
| 3 | `package.toml` `label`+`icon` manifest fields + installed-app enumeration (shared with task 56; build here if 56 hasn't) | `app_installer.rs`, `app_loader.rs` |
| 4 | `war:shell/launcher` WIT host impl forwarding to the arbiter (shared with task 56) | `wit/shell.wit`, `wart-host/src/launcher_impl.rs` |
| 5 | `war.launcher` warpkg — Compose app grid + clock + static wallpaper; icons from assets (task 38); letter-tile fallback when no icon | `apps/system/war.launcher/` |
| 6 | `run-hybrid-stack.sh` / Magisk module sets home + foregrounds it at boot | `tools/scripts/`, `magisk-module/` |
| 7 | Device-verify: boots to home; tap launches an app; that app exits/killed → returns to home (not black); rotation re-lays the grid (task 43) | device |

## Out of scope

- Home-screen widgets, multiple home pages, icon folders, drag-to-arrange.
- Live / wallpaper-extracted theming (WallpaperManager dropped; static
  asset or solid only). Accent color can reuse the existing `theme` WIT.
- Search / all-apps-vs-favorites split, app shortcuts.
- Lock screen (separate concern).

## Open questions (decide before implementing)

1. **Proposal A vs B** — dedicated fullscreen home (A) or drawer-overlay
   only (B)?
2. **Wallpaper** — bundled static asset, solid color, or the existing
   accent/theme gradient? (No live wallpaper.)
3. **Boot policy** — always boot to launcher, or boot to the
   last-foreground app (restore session)?
4. **App-death fallback** — always return to home, or only when there's
   no other running app to surface? (Interacts with task 54 + future
   recents.)

## Related

- [`tasks/56-taskbar.md`](56-taskbar.md) — the persistent bottom bar;
  shares the `war:shell/launcher` WIT + `label`/`icon` manifest + app
  enumeration. Launcher = home screen; taskbar = persistent strip.
- [`tasks/55-status-bar.md`](55-status-bar.md) — top strip; shares the
  clock source + theme.
- [`tasks/54-arbiter-death-notification.md`](54-arbiter-death-notification.md)
  — `handle_child_exit` is the hook for "fg app died → fall back to home."
- [`tasks/46-wart-arbiter-mvp.md`](46-wart-arbiter-mvp.md) — arbiter
  registry + foreground command + state persistence the home-app concept
  extends.
- `post-art-roadmap.md` §6.3 — launcher/SystemUI/WallpaperManager dropped,
  "runtime draws everything"; PackageManager → install-dir component graph.
