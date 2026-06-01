# Visual sizing — design patterns, and where wart differs

> One-page note. The problem (non-hardcoded, runtime-scalable sizing across
> density / orientation / multiple surfaces) is a **solved design-pattern
> problem**, not a WASM/C++/wart problem. This captures the proven patterns and
> — the important part — where our setup genuinely deviates from them.

## The problem
Surface / keyboard / chrome sizing must never be a literal. It has to be
**derived** from real inputs and recomputed as they change: display density,
orientation, available space, and the presence of other surfaces (chrome, IME).

## The proven patterns (Android, Compose, Flutter, CSS all use these)
- **Density-independent units (dp/sp)** — all logic in dp; convert to px only at
  the pixel edge, `dp × density`. (Kills raw-px hardcodes.)
- **Intrinsic sizing + constraints** — a thing's size is `f(its content)` *bounded
  by* the space it's given (measure → layout pass). The keyboard's height is
  `rows × comfortable_row_dp`, capped by available height — not a number.
- **Fractional / constraint layout** — sizes are *relationships* to available
  space, recomputed; never absolute pixels for a specific panel.
- **Insets / safe-area** — the system *reports* occluded regions (status bar, nav,
  IME); content lays out inside the safe rect. Insets are **data that flows in**,
  not margins you guess. (Android `WindowInsets`, CSS `env(safe-area-inset-*)`.)
- **Reactive recompute on configuration change** — geometry is *derived state*;
  on rotate / resize / inset change you **re-measure**, you don't store absolutes.
- **Negotiated sizing** (the meta-pattern) — the *producer* declares its
  **intrinsic** size; the *container* supplies **constraints** (the available
  space); neither hardcodes. This is the spine of all the above.

## Where wart differs (why we can't copy Android/Compose verbatim)
1. **The producer is a separate process, not a view in one layout tree.** In
   Android/Compose a single measure pass over one tree sizes everything. Here the
   IME, each app, and each overlay are *separate WASM guests with separate
   surfaces*. → Negotiated sizing must happen **across the WIT / IPC boundary**,
   not inside a shared measure pass.
2. **An overlay can't see the screen.** A normal container knows its full space; a
   wart overlay guest sees only *its own strip surface*. → The available-space
   constraint must be **pushed to the guest over WIT** (the `display` interface) —
   otherwise it sizes blind (this is exactly why the IME couldn't compute %).
3. **Split authority.** Android has one WindowManager/InsetsController that
   computes *and* dispatches insets. wart splits it three ways: **host** owns
   surfaces/geometry, **arbiter** owns roles + the overlay split, each **guest**
   owns its own layout. → The "system" in the pattern is *multiple processes*;
   insets/constraints become a **contract between them**, not a library call.
4. **Inverted ownership — producer-owns-size.** Android's *system* sizes the IME
   inset. Our deliberate model: the **IME guest decides** its size and reports it;
   the host is a dumb applier. Still "negotiated sizing", but the policy lives in
   the producer — so the producer *must be handed the screen constraint* to decide
   well (ties back to #2).
5. **Multi-language dp.** Compose has `dp` built in; our **Rust** guests don't. →
   dp must be a **WIT contract** (`window` reports density; every guest converts
   itself), not an assumed framework primitive.
6. **Layout ≠ render.** Standard toolkits repaint on layout invalidation. wart
   **gates rendering** on frame-pacing / `dirty`. → "Reactive recompute" must also
   **force a render**, or you get the present-but-empty-surface bug we hit
   (recompute happened; the paint didn't).

## What our version of the patterns therefore is
Same patterns, **negotiated over WIT**:
- Host *measures* the real panel and **pushes** to each guest: screen size,
  density, and the current **insets** (chrome + IME occlusion) — i.e. the
  display ⊃ content ⊃ safe rectangles, in dp.
- Each guest computes its **own intrinsic size in dp** from that constraint
  (keyboard = `rows × row_dp`, capped by safe height); the host **applies it
  verbatim** (dumb applier).
- The arbiter **composites** anchored surfaces (bottom IME, future side/top
  panels) by role + z-order.
- Any configuration change **re-pushes geometry AND marks dirty** (recompute +
  repaint together).

The single rule underneath all of it: **no literal in logic** — every value is a
function of a measured input, named once where it's genuinely a policy.

## Target architecture — the arbiter as the native shell / window-server

The coordinator Android put in `system_server` (WMS + IMMS + AMS) was dropped with
ART and never replaced — its responsibilities scattered across the per-app
**wart-host**, the **wart-arbiter**, and the **guests**. That scatter is why
geometry/inset state disagrees between processes (empty keyboard, stale overlay).
The fix is not more peer coordination — it is to **re-centralize that authority
into the arbiter**, reborn as the lean, native equivalent of system_server's
coordinator: **it decides, it never renders.**

**The arbiter absorbs three `system_server` roles** (the *responsibilities*, not
the Java or binder protocols):
- **AMS** (ActivityManager) — *already there*: foreground/background roles, the
  task ring, launch, lifecycle.
- **IMMS** (InputMethodManager) — *already partial*: editor-focus routing
  (`attach`/`detach-editor`), `set-ime`. Completed by owning IME show/hide as a
  function of focus + the overlay split.
- **WMS** (WindowManager) — *the missing piece, moving in*: panel size + density +
  orientation (from the sensor HAL + SurfaceFlinger), the set of surfaces with
  anchors + z-order, and the computed insets/rects per app (display ⊃ content ⊃
  safe). All inset math now duplicated in the host's `recompute_transform` /
  `overlay_rect` moves **here**, computed once, globally.

**Keep Android's own policy/mechanism split** — this is the proof the design isn't
exotic; it's Android with `system_server` replaced by a native arbiter:

| Responsibility | Android | wart |
|---|---|---|
| decide layout / insets / z-order / focus | WMS·AMS·IMMS (`system_server`, Java) | **arbiter** (native, lean) |
| composite surfaces | SurfaceFlinger | SurfaceFlinger (unchanged) |
| dispatch input | InputFlinger | InputFlinger (unchanged) |
| render a window's content | app `ViewRootImpl` | **per-app host** (skia/EGL) |
| the UI | app | **guest** |

WMS never rendered — it decided and handed off to SurfaceFlinger. Same here:
arbiter decides → SF composites → hosts render → guests lay out.

**Data flow (one owner, event-driven):**
```
sensor / launch / focus / size-request
   → arbiter computes global layout
   → pushes per-surface geometry (size, orientation, safe rect, density — in dp)
   → host applies (dihedral transform + EGL) + relays constraints to its guest
   → guest computes its intrinsic size in dp → reports desired size up
   → arbiter recomputes → re-pushes  (+ marks the surface dirty so it repaints)
```
Geometry recomputes on **events**, not per frame; per-frame rendering stays in the
host. Re-push always pairs with dirty (recompute + repaint together).

**Rules that keep it lean (so we don't rebuild the `system_server` monster):**
- Reimplement *responsibilities*, not binder/Java — a state machine over the
  existing arbiter socket protocol.
- The arbiter is **policy + state only**: it owns no surface and never paints.
- **No literals in logic** — every value derived from a measured input (the
  no-hardcode rule), named once only where it is genuinely a policy knob.

**Patterns underneath:** Mediator (one authority) + inversion of control
(constraints are *pushed* to guests, never pulled/guessed) + the insets /
negotiated-sizing patterns above.

## Modularity — a core + responsibility crates, not a monolith

The arbiter inherits many `system_server` responsibilities over time (WMS, IMMS,
AMS already; then notifications, alarms/background-execution, audio-focus,
clipboard, keyguard/wallpaper, multi-display — see *Foreseen responsibilities*).
It must grow by **adding a crate and wiring one line**, never by digging into
working code (Open/Closed).

**Shape.** A thin **`wart-arbiter-core`** kernel owns only:
1. the **event loop + socket transport** (verbs in, replies out);
2. a typed **shared-state store** — the single source of truth (displays,
   surfaces, focus, roles);
3. an **event bus** (`ForegroundChanged`, `OrientationChanged`,
   `EditorFocusChanged`, `DisplayAdded`, …);
4. a **module trait** + **registry**.

```rust
trait ArbiterModule {
    fn verbs(&self) -> &[&str];                          // commands it owns
    fn on_command(&mut self, v: &str, args, ctx: &mut Ctx) -> Reply;
    fn on_event(&mut self, e: &Event, ctx: &mut Ctx);    // react to others' changes
}
```
Each responsibility is its own crate (`wart-arbiter-wm`, `-ime`, `-am`,
`-notify`, `-alarm`, `-audiofocus`, …); the binary only:
```rust
reg.register(WmModule::new());
reg.register(ImeModule::new());   // a new responsibility = +1 crate, +1 line
```

**The rule that keeps additions non-invasive:** modules **never call each other**
— they emit / observe **events** and read/write the **shared store** via `Ctx`.
The WM module reacts to an `EditorFocusChanged` from the IME module without either
knowing the other exists. New module = new verbs + new event reactions; existing
modules untouched. (This also removes the scattered `if verb == "x"` chains — an
anti-hardcode.)

**Honest cost.** Modularity isn't free — it moves the hard part into getting the
**core contracts** right up front: the event vocabulary, the shared-state schema
(the per-display surface/role + resource-focus model), and the module trait. Get
those right → responsibilities are additive crates forever; get them wrong → you
still edit the core. So design effort belongs in the **core**, not the modules.

**Migration without rewriting the working arbiter (strangler):**
1. Add `wart-arbiter-core` (bus + store + trait + registry) *alongside* today's code.
2. Wire **new** responsibilities (WM-geometry, notifications, alarms, audio-focus)
   as modules from day one.
3. Migrate **existing** logic (roles, overlay-split, ime-routing) into modules
   **only when already changing it** — e.g. the WM/geometry move planned above
   creates that seam once; code that works and isn't changing stays as-is until it
   naturally needs migrating.

## Foreseen `system_server` responsibilities (design the core to absorb these)

Native survivors stay **mechanism** (the arbiter directs, never absorbs):
SurfaceFlinger, InputFlinger, AudioFlinger, sensors HAL, vibrator, gralloc/GPU,
battery sysfs.

`system_server` **policy** the arbiter will inherit as modules:

| Responsibility | Note for wart |
|---|---|
| **DisplayManager** | **Bake in now** — key all geometry *per-display*; "one panel" is a hardcode. |
| **Audio focus** | Same shape as window/IME focus → arbiter is a **resource-focus arbiter**, generalize now. |
| **AlarmManager + JobScheduler** | Wake/run apps that aren't foregrounded — **real gap, hits Signal** (background message receipt); `scheduler` only fires while running. |
| **Notification + StatusBar** | Cross-app; the status-bar guest needs an arbiter notification authority. |
| **PowerManager** | Screen on/off, wakelocks, doze/suspend policy (ties to AMS lifecycle). |
| **Keyguard / Wallpaper / Dream** | Special **surface roles** in the same overlay/z-order model, not specials. |
| **Clipboard** | Make it arbiter-global (cross-app), not in-process. |
| **PackageManager perms** | Maps to WIT capability grants; arbiter owns runtime grant/revoke. |
| Accessibility · Location · Connectivity/telephony | Longer-term; flag, don't design yet. |

**Two decisions cheap now / costly later (and both anti-hardcodes):** key the
state model **per-display**, and model focus as a **generalized resource-focus**
(window, input, IME, audio) — so each new responsibility is "one more
resource/role," not a refactor.
