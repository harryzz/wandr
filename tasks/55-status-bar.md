# Task 55 — system status bar

> **Status:** 🟢 device-verified 2026-05-29 (Proposal A — top-overlay
> guest). v1 ships clock + battery. Known v1 limitations: no content
> insets (the strip floats over the top ~88 px of apps), and
> `war.statusbar` currently installs under `apps/` so the launcher lists
> it (move to `system-apps/` + an installed-load path for it).
>
> ## Results (2026-05-29)
>
> Built as a **light Rust canvas guest** on a **top-anchored overlay
> surface** — same shape as `war.launcher`/the IME, no Kotlin/Compose.
>
> **Surface abstraction (per an on-device design discussion):** instead
> of per-anchor shim functions, the libgui shim's overlay create is now
> **geometry-parameterized** — `sf_create_overlay_surface(x, y, w, h)`
> with conventions `w/h<=0` → full panel dim, `y<0` → bottom-anchored.
> The status bar passes `(0,0,0,88)`, the IME `(0,-1,0,1200)`. New bars
> need no new shim symbol / no new a-03 build. The **runtime owns the
> semantics** (`standalone::OverlayMode {None,Bottom,Top}` → rect; the
> arbiter is the per-process surface-role manager); the shim is purpose-
> agnostic. Rebuilt `libsf_surface.so` on a-03 via the direct-ninja path.
>
> **Data (ART-free):** `my:skiko-gfx/status` host interface —
> `clock-text()` (local time via the native `date` binary, not
> `system_server`) + `battery-text()` (sysfs
> `/sys/class/power_supply/battery/capacity`). `status_impl.rs`.
>
> **Guest:** `apps/system/war.statusbar/` (~48 KB Rust `wasm32-wasip2`,
> trimmed `my:skiko-gfx` WIT). Draws `wart` (left) · clock (center) ·
> battery (right) on an 88 px strip; polls the status verbs ~1 Hz and
> rebuilds text blobs only on change (no animation loop).
>
> **Plumbing:** `sf_surface.rs` `create_overlay(x,y,w,h)`; `standalone`
> `OverlayMode` + `STATUS_BAR_PX`; `zygote` `LAUNCH_GUI_OVERLAY_TOP`;
> `main` `--standalone-overlay-top`. Launched directly as a top-overlay
> daemon for v1 (`wart-host --standalone-overlay-top --app war.statusbar`)
> alongside the launcher stack; composites above the fullscreen app
> (equal `i32::MAX` layer, created later → on top).
>
> **Device-verified:** top strip shows `wart · 11:58 · 100%` over the
> launcher. Follow-ups: content insets (`on-insets-changed` → guests lay
> out below the bar), `system-apps/` placement, arbiter `launch-overlay-top`
> + boot integration, notification area, explicit chrome z-layer reservation.

---

> **Original proposal (kept for history):**

## Why this matters

Post-ART, wart force-stops SystemUI and owns the whole screen
(tasks 33, 46). There is currently **no clock, no battery indicator,
nothing** at the top of the screen — the foreground app draws edge to
edge. The roadmap already calls this out:

> §6.3: *StatusBar / SystemUI / WallpaperManager — runtime draws
> everything.* §6.3: *NotificationManager — replaced by WIT
> `post-notification` into a runtime-owned compositor row.*

So the status bar is part of the wart "system shell" we now have to
provide ourselves. This task is the top strip; the bottom strip
(taskbar / nav) is [`tasks/56-taskbar.md`](56-taskbar.md). They share a
foundation (see **Shared foundation** below).

## ART-free constraint

Per [[feedback_no_art_layer_dependencies]], the status bar's data must
NOT come from `system_server`. Source map:

| Item | Source (ART-free) | Difficulty |
|------|-------------------|-----------|
| Clock | wall-clock time in the host | trivial |
| Battery % + charging | sysfs `/sys/class/power_supply/battery/{capacity,status}` (or IPower/health HAL via rsbinder) | easy |
| Notifications | wart-native `post-notification` WIT verb → arbiter → status bar (the roadmap's "compositor row") — NO NotificationManagerService | medium (new subsystem) |
| Wifi | `wpa_supplicant` control socket (roadmap §6.4, deferred) | medium-hard |
| Cellular / signal | **out of scope** — telephony dropped (roadmap §6.3, "not a phone replacement") | n/a |

v1 should land **clock + battery**; notifications + wifi are stretch /
follow-ups.

## Shared foundation (with task 56)

Status bar, taskbar, and the IME keyboard (task 47) are all
**system overlay surfaces** — fixed strips composited above the
foreground app. Three pieces of shared plumbing should be built once
(in whichever of 55/56 lands first, or as a small precursor task):

1. **Top/bottom-anchored overlay surface.** Task 47's
   `sf_create_overlay_surface(height_px)` makes a bottom strip via
   `SurfaceControl::setPosition`. Generalize it with an anchor/position
   arg so the status bar can take the *top* strip. This is a small
   `cpp/sf_surface.cpp` extension built on the a-03 AOSP host — that's
   the **normal build workflow, not a blocker** (see
   [[project_boot_model_libgui_build]]).
2. **Content insets for the foreground app.** With a status bar
   overlaying the top N px, the app must lay its UI out *below* it or
   the app's top content hides behind the bar. Add a host→guest
   `on-insets-changed(top, bottom, left, right)` export; the guest maps
   it to a Compose `WindowInsets` provider (`LocalWindowInsets`-style).
   The arbiter computes insets from which system strips are visible and
   pushes them to the foreground app over its per-host control socket
   (same channel as task 49's editor events).
3. **`war:shell` WIT surface + arbiter routing.** A small shell
   interface the system warpkgs import, host-implemented, with the
   arbiter as the policy owner (mirrors `keyboard_host_impl.rs` →
   arbiter routing from task 47/49).

## Proposals

### Proposal A — status bar as a guest warpkg (recommended)

A first-party warpkg `war.statusbar` (Kotlin/Compose, like
`war.ime.keyboard`) on a **top-strip overlay SF surface**. The arbiter
launches it via `launch-overlay` (extended with a top anchor). It draws
the clock/battery/notification row in Compose and reads data via a new
`war:shell/status` host interface:

```wit
interface status {
    record battery { percent: u8, charging: bool }
    now-millis:  func() -> u64;       // host wall clock
    get-battery: func() -> battery;   // host reads sysfs
    // notifications (stretch): host buffers post-notification calls
    // from other apps (routed via arbiter) and the bar polls them.
    record notification { id: u32, app-id: string, title: string, body: string, icon: u32 }
    list-notifications: func() -> list<notification>;
}
```

- **Pros:** rich/themeable (Material3), consistent with the IME pattern,
  hot-reloadable as a warpkg, reuses the whole skiko render path.
- **Cons:** a second always-on Compose process (~80–180 MB working set
  per the zygote spike); on-change redraw discipline needed so it isn't
  burning a render loop at 60 fps for a clock that ticks once a second.

### Proposal B — host-drawn status bar (MVP / fallback)

Draw the bar directly in `wart-host` (Rust + skia-safe) into a top-strip
overlay surface — no guest, no Compose. Just clock + battery text +
icons.

- **Pros:** tiny footprint, no extra process, dead simple for
  clock+battery, no warpkg build.
- **Cons:** not Compose (hand-rolled skia layout), awkward to grow into
  notifications/quick-settings, duplicates text/layout logic the guest
  path already has.

### Proposal C — fold into the foreground app (rejected)

Have each app draw its own status bar via a Compose component.

- **Rejected:** every app would reimplement it; no consistency; the bar
  would die/redraw on every app switch; notifications couldn't span
  apps. The whole point is a *system*-owned strip.

**Recommendation:** **Proposal A** for the real thing, but consider
landing **B as a 1-day MVP** (clock + battery only) to validate the
top-overlay + insets foundation cheaply, then graduate to A when
notifications/theming are wanted.

## Interaction (stretch)

- **Notification shade:** swipe-down to expand the bar into a full
  notification list. Big — needs gesture handling on the overlay,
  a resize of the overlay surface (reuse task 47's
  `request-overlay-height`), and the `post-notification` subsystem.
  Defer past v1.
- **Quick settings** (wifi/brightness toggles): further out; depends on
  the wifi/wpa_supplicant work and a brightness sysfs write.

## Steps (Proposal A, v1 = clock + battery)

| # | Step | Where |
|---|------|-------|
| 1 | Top-anchored overlay surface (anchor arg) — a-03 shim build | `cpp/sf_surface.cpp`, `sf_surface.rs` |
| 2 | `war:shell/status` WIT + host impl (`now-millis`, `get-battery` via sysfs) | `wit/shell.wit`, `wart-host/src/status_impl.rs` |
| 3 | `on-insets-changed` host→guest export + arbiter inset computation + push over per-host socket | `wit/skiko-gfx.wit`, `ime_inbound.rs`, `wart-arbiter` |
| 4 | Compose `WindowInsets` provider in wart-app consuming `on-insets-changed` | `apps/user/wart-app` |
| 5 | `war.statusbar` warpkg (Compose top row, on-change redraw) | `apps/system/war.statusbar/` |
| 6 | Arbiter auto-launches the status bar at startup, top layer, never demoted | `wart-arbiter` |
| 7 | Device-verify: bar shows correct time + live battery; app content insets below it; rotation re-lays the bar (task 43 interplay) | device |

## Out of scope

- Cellular/signal indicators (telephony dropped).
- Full notification shade + quick settings (stretch; needs the
  `post-notification` subsystem + wifi/brightness HAL work).
- Lock screen / always-on display.

## Open questions (decide before implementing)

1. **v1 surface:** Proposal A (guest warpkg) or B (host-drawn MVP)?
2. **Battery source:** sysfs (simplest, may vary by device) vs the
   IPower/health HAL via rsbinder (more portable, more code)?
3. **Notifications in v1 or deferred?** (Drives whether the
   `post-notification` subsystem is in scope now.)
4. **Insets model:** real content insets (app lays out below the bar) vs
   translucent float-over (app draws full, bar on top)? Real insets are
   the right answer but need the `on-insets-changed` + Compose work.

## Related

- [`tasks/56-taskbar.md`](56-taskbar.md) — bottom strip; shares the
  overlay-surface + insets + `war:shell` foundation.
- [`tasks/47-ime-via-guest-app.md`](47-ime-via-guest-app.md) — the
  overlay-surface + arbiter-routing pattern this reuses.
- [`tasks/43-screen-orientation.md`](43-screen-orientation.md) — the bar
  must re-lay-out on rotation; insets interact with the rotated logical size.
- `post-art-roadmap.md` §6.3 — "runtime draws everything" + the
  `post-notification` compositor row.
- [[feedback_no_art_layer_dependencies]] — battery via sysfs, not
  BatteryManagerService; no telephony.
