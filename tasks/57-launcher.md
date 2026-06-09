# Task 57 — system launcher / home (proposal)

> **Status:** ✅ device-verified 2026-05-29 (Proposal A). Steps 1–6 done
> AND the dedicated `wandr.launcher` wandrpkg shipped — as a **light Rust
> canvas guest**, not Kotlin/Compose.
>
> ## Dedicated launcher (2026-05-29) — the light Rust canvas guest
>
> Built `apps/system/wandr.launcher/` as a Rust `wasm32-wasip2` component
> (~70 KB) that **exports `my:skiko-gfx/renderer`** and draws the app
> grid directly via the canvas WIT — **no Kotlin/Wasm runtime, no
> Compose**, so it's immune to the continuation leak
> ([[feedback_indeterminate_progress_leak]]) and has a tiny working set,
> which is what a persistent home process needs. egui was evaluated and
> rejected: it renders by tessellating to GL/wgpu *meshes*, but our guest
> has no GL — only the high-level host-Skia canvas WIT — so egui's output
> doesn't map (and it'd add weight for a trivial UI).
>
> **Key constraints solved:**
> - Guest wit-bindgen 0.46 rejects the canonical `skiko-gfx.wit` (the
>   `matrix-3x3` identifier — a WIT word can't start with a digit; the
>   *host* parser is lenient). → hand-authored a **trimmed `my:skiko-gfx`
>   WIT** (`wit/launcher.wit`) with just the verbs used + `paint-attrs`
>   and its enums copied **verbatim** (field/variant order matters for
>   structural type-matching). The component imports a *subset* of the
>   host's full canvas; instantiation links cleanly (verified — this is
>   the first non-Kotlin renderer guest).
> - Layout is built **once** (on app-list load + on resize) into a flat
>   draw list + tile hit-rects; `render_frame` just replays it — no
>   per-frame allocation, no animation loop.
>
> **Device-verified:** keystone (clear-screen) instantiated + painted
> (center px exactly `0x1A1A2E`); the grid renders the "Apps" title +
> letter tiles + labels via host fonts; tapping the "Demo" tile chained
> Rust `on-pointer-event-v2` hit-test → `launch-app` → host
> `forwarded launch` → arbiter `launched → pid / promoting foreground`.
> Wired into `build-system-wandrpkgs.sh` (builds/packs/installs it) and set
> as the **default boot home** (`WANDR_HOME_APP`, default `wandr.launcher`)
> in `run-hybrid-stack.sh` + the Magisk module — the device now boots
> straight to the Rust launcher.
>
> wandr-app keeps its `LauncherCard` as a Kotlin-side demo of the same WIT
> (harmless; the Rust `wandr.launcher` is the actual home).
>
> **Step 6 — boot to the launcher** (device-verified): both the dev
> entry point (`run-hybrid-stack.sh`) and the Magisk module
> (`service.sh`) now `set-home $HOME_APP` (default
> `com.example.wandr-app`, `WANDR_HOME_APP=""` to disable) once the arbiter
> socket is up — the dev script does it from a background waiter since
> the arbiter runs foreground; the Magisk module does it inline after its
> socket-wait. Verified by replaying the startup flow with fresh state
> (no persisted home, **no manual `launch`**): the device came up with
> the launcher grid foreground (pid …, `[fg]`).
>
> ## Results (2026-05-29)
>
> Built Proposal A. Per [[feedback_prefer_wandr_app_edits]] the launcher
> UI was proven inside wandr-app (a `LauncherCard` at the top of the
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
>   `wandr-host/src/launcher_impl.rs` scans `<APPS_ROOT>/apps/*` for the
>   id + top-level `label` from each `package.toml` (the manifest is
>   **flat**, not `[package]`-nested — note vs the proposal text), and
>   forwards `launch-app` to the arbiter socket (one-shot, mirrors
>   `ime_host_impl`). No installer change (package.toml copied verbatim;
>   serde ignores the unknown `label`).
> - **Step 5 — launcher UI in wandr-app:** hand-written canonical-ABI
>   bindings `LauncherImports.kt` (modeled on `AssetsImports.kt` — string
>   param + caller-allocated 8-byte return area for the string return);
>   `LauncherCard.kt` renders a `FlowRow` of letter-tile icons (first
>   label char on a hash-derived color) over a theme-gradient backdrop,
>   tap → `launchApp`. wandr-app's `package.toml` gains `label = "Demo"`.
> - **Device-verified:** `set-home com.example.wandr-app` → boots to the
>   launcher card; tile renders **"D" / "Demo"** (label resolved); tapping
>   it logs the full chain guest `tap → launch` → host `forwarded launch`
>   → arbiter `launched → pid … / promoting foreground`.
>
> **Remaining (small follow-ups):** Step 6 — `run-hybrid-stack.sh` /
> Magisk auto-`set-home` at boot (the home concept + persistence already
> make boot-to-home work once set). And a dedicated full-screen
> `wandr.launcher` wandrpkg (the wandr-app card proves the path; a real
> launcher app is the clean spin-out — its richer demo value is gated on
> having ≥2 GUI apps to launch anyway). Letter-tile icons + theme
> gradient shipped as the v1 choices.

---

> **Original proposal (kept for history):** 🔲 proposal 2026-05-29.

## Why this matters

Right now a standalone wandr device boots to **nothing usable** — the
Hybrid stack comes up (zygote + arbiter) but no app is foreground until
someone runs `wandr-arbiter launch <app-id>` from an adb shell. There is
no home screen, no app grid, and no "home" to return to when an app
exits or is killed. The launcher is what makes wandr a *usable device*
rather than a dev harness:

- the **boot target** (what shows when the stack starts),
- the **home** the user returns to (go-home, or an app exits/LMK-dies),
- the **app grid / drawer** to launch installed apps without adb.

The roadmap frames this as part of "runtime draws everything" (§6.3,
SystemUI/launcher dropped) — wandr provides its own.

## Launcher vs taskbar (task 56) — they're different things

Android (and wandr) have two distinct surfaces; don't conflate them:

| | Launcher (this task, 57) | Taskbar / nav (task 56) |
|---|---|---|
| Surface | **Fullscreen** app, takes the foreground like any app | **Overlay** strip, always composited on top |
| Visibility | Visible only when it's foreground (boot / home) | Persistent across apps |
| Role | App grid + wallpaper + "home" | Switcher + quick dock + back/home affordances |
| Process | A normal fg wandrpkg (`wandr.launcher`) | An overlay wandrpkg (`wandr.taskbar`) |

They **share** the app-enumeration + `wandr:shell/launcher` WIT + the
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

New arbiter state: `home_app_id` (set via `wandr-arbiter set-home
<app-id>`, persisted in `wandr-arbiter-state.json`).

## Proposals

### Proposal A — full-screen home app (recommended)

A first-party `wandr.launcher` wandrpkg (Kotlin/Compose), a normal
**fullscreen** foreground app the arbiter designates as home. It renders:

- an **app grid** of installed apps (icons + labels), tap → launch /
  foreground via the shared `wandr:shell/launcher` WIT (task 56);
- a **clock** (host wall-clock, same source as the status bar);
- a **wallpaper** — a static image asset or solid/gradient (WallpaperManager
  is dropped per roadmap §6.3; a live/extracted wallpaper is out of scope).

- **Pros:** the real thing; uses the existing fullscreen surface path
  (no overlay/insets/shim work — unlike 55/56); reuses skiko + the app
  enumeration; the natural boot target.
- **Cons:** another wandrpkg; needs the arbiter home-app concept (but that
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
fullscreen wandrpkg) and the highest user payoff (the device becomes
usable without adb). The arbiter home-app concept is small and dovetails
with task 54's death-notification hook.

## Steps (Proposal A)

| # | Step | Where |
|---|------|-------|
| 1 | Arbiter `home_app_id` state + `set-home <app-id>` command + persist in state.json | `wandr-arbiter/src/{state,main}.rs` |
| 2 | Arbiter foregrounds home at startup; `go-home` command; **fallback-to-home in `handle_child_exit`** (task 54 hook) when fg app dies | `wandr-arbiter` |
| 3 | `package.toml` `label`+`icon` manifest fields + installed-app enumeration (shared with task 56; build here if 56 hasn't) | `app_installer.rs`, `app_loader.rs` |
| 4 | `wandr:shell/launcher` WIT host impl forwarding to the arbiter (shared with task 56) | `wit/shell.wit`, `wandr-host/src/launcher_impl.rs` |
| 5 | `wandr.launcher` wandrpkg — Compose app grid + clock + static wallpaper; icons from assets (task 38); letter-tile fallback when no icon | `apps/system/wandr.launcher/` |
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
  shares the `wandr:shell/launcher` WIT + `label`/`icon` manifest + app
  enumeration. Launcher = home screen; taskbar = persistent strip.
- [`tasks/55-status-bar.md`](55-status-bar.md) — top strip; shares the
  clock source + theme.
- [`tasks/54-arbiter-death-notification.md`](54-arbiter-death-notification.md)
  — `handle_child_exit` is the hook for "fg app died → fall back to home."
- [`tasks/46-wandr-arbiter-mvp.md`](46-wandr-arbiter-mvp.md) — arbiter
  registry + foreground command + state persistence the home-app concept
  extends.
- `post-art-roadmap.md` §6.3 — launcher/SystemUI/WallpaperManager dropped,
  "runtime draws everything"; PackageManager → install-dir component graph.
