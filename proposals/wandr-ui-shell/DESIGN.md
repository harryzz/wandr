# wandr:ui-shell + the consolidation event — retiring my:skiko-gfx AND wasi:canvas@0.0.1

> Proposal (2026-06-12). Bound by the family goal (clean / no overlap /
> WASI-shaped / 100% reference-library consumable) and rules R1–R5
> (`../wasi-canvas/REDESIGN-0.0.2.md`). WIT: `wit/ui-shell.wit`
> (validated). The user-set end state: **neither legacy package
> survives** — my:skiko-gfx does not live on as "the rest", and
> wasi:canvas@0.0.1 does not stay served once its consumers move.

## The separation (every my:skiko-gfx interface, classified)

Grounded in source: slint-wandr consumes `window`+`theme`+`ime`,
dioxus `ime`, chrome the `renderer`/`frame-pacing` exports + per-app
services, Compose everything (the 2026-06-12 audit).

| Class | Interfaces | New home | Who needs it |
|---|---|---|---|
| **Drawing** | canvas, paragraph | `wasi:canvas@0.0.2` (done) | all UI libs |
| **Input delivery** | renderer pointer/key legs, key-input | `wasi:input-handlers@0.0.2` (done) | all UI libs |
| **UI shell — universal** | window(→metrics), theme, locale, clipboard, ime (app side), lifecycle, scheduler, text-segmentation, frame-pacing export, renderer's on-scheduled-callback + on-lifecycle-changed (→ shell-events export) | **`wandr:ui-shell@0.1.0`** (this draft) | every UI library — the cross-framework set the reference libraries proved |
| **Diagnostics** | canvas.log-message | `wasi:logging` (upstream, host impl to add) | everyone |
| **App/OS services** | audio, haptics, power, thermal, sensors, lights, assets, display, pointer-icon, keyboard (IME-app key send), launcher, status, keyguard-app, taskbar-app | `wandr:*` packages (the alarm/notify/events precedent): proposed grouping `wandr:device` {haptics, power, thermal, sensors, lights}, `wandr:audio`, `wandr:assets`, `wandr:chrome` {launcher, status, keyguard-app, taskbar-app, display, pointer-icon}; keyboard → the wandr:ime family | apps (any framework) — NOT a UI-library concern |

Nothing remains in my:skiko-gfx after this table — that is the point.

## Why scheduler/segmentation are in the shell (R5 audits)

- `scheduler`: a timer must WAKE a frame-paced idle host loop; not
  derivable from frame ticks that aren't arriving. Pairs with
  frame-pacing by design.
- `text-segmentation`: the managed-runtime ICU capability gap (the same
  argument as layout vs glyphs).
- Everything else in the shell is host-owned ambient state (metrics /
  theme / locale / clipboard / lifecycle) or the editor protocol.

## The consolidation event (ONE rebuild of everything, by request)

The user's constraint: no N× skiko/compose compiles. Sequencing:

**Phase A — host, purely additive (no guest rebuilds):**
1. Implement `wandr:ui-shell` (every impl already exists under
   my:skiko-gfx names — this is re-binding, not new code) + probe-only
   export worlds (shell-events, frame-pacing).
2. Implement `wasi:logging` (upstream package; ~small).
3. Keep my:skiko-gfx + wasi:canvas@0.0.1 + input-handlers@0.0.1 served.

**Phase B — every guest moves ONCE:**
- Kotlin (skiko + wandr-app + ime.keyboard): the finale as audited —
  canvas/paragraph → 0.0.2 (incl. images, drawables→scene,
  blobs→paragraphs, setMatrix→tracking, surface-w/h→canvas dims),
  input → 0.0.2 exports (+ pointer id-map assembly, scroll delivery),
  platform calls → ui-shell, log-message → wasi:logging. One skiko
  build, one compose×9, one build per app.
- Rust guests (slint-wandr, dioxus-canvas, 4 chrome + settings.wifi,
  taskmanager, connectivity, Signal, slint.test, ktcanvas spike):
  wasi:canvas 0.0.1 → 0.0.2 (regen + the 29-blend/paint-filter deltas
  are already source-compatible), renderer/frame-pacing exports →
  input-handlers@0.0.2 + ui-shell worlds, window/theme/ime → ui-shell.
  One cargo build each.
- Deploy = the stage-3 playbook (per-app installs + one zygote restart).

**Phase C — host cleanup (after B verifies):**
- Delete the skiko-ui bindgen world + my:skiko-gfx impls; instantiation
  becomes probe-only (no required exports).
- Drop wasi:canvas@0.0.1 + input-handlers@0.0.1 bindgen/linker entries
  and the wit/ trees; wit-0.0.2 renames to wit/.
- Delete wit/skiko-gfx.wit + every consumer mirror (the WIT-sync rule
  dies with it).

## Acceptance check (family goal)

| Criterion | Status |
|---|---|
| Clean | shell = exactly the cross-framework set; services = wandr:* like every other service; drawing/input untouched |
| No overlap | each legacy interface has exactly ONE new home (table above); shell-events carries only the legs input-handlers scoped out |
| WASI-shaped | shell is wandr-namespaced (embedder contract — the convergence doc's §4 "platform remainder" position: candidate small wasi packages LATER, not squatted now); logging adopts the real upstream package |
| 100% consumable | the universal row is the proven slint/dioxus/Compose set + the Avalonia/Flutter/egui memos' needs (metrics/theme/ime/clipboard/lifecycle) |
