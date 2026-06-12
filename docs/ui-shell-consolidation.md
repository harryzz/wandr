# wandr:ui-shell + the consolidation event — retiring my:skiko-gfx AND wasi:canvas@0.0.1

> Proposal (2026-06-12). Bound by the family goal (clean / no overlap /
> WASI-shaped / 100% reference-library consumable) and rules R1–R5
> (`proposals/wasi-canvas/REDESIGN-0.0.2.md`). WIT: `wit/ui-shell.wit`
> (repo root — a wandr:* platform contract beside alarm/notify/etc.,
> deliberately NOT in proposals/: that tree is the WASI-facing family)
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

**Phase C — host cleanup (after B verifies). Sized inventory
(2026-06-12 audit), two kinds of work:**

*C1 — the delegation inversion (the one structural task).* Phase A made
new-trait impls DELEGATE to the legacy trait impls whose logic is
inline. When the legacy bindgen dies, the logic must live in the new
homes: for each of the ~22 `*_impl.rs` files (theme, locale, clipboard,
window, lifecycle, scheduler, text-segmentation, ime/keyboard, haptics,
power, thermal, sensors, lights, pointer-icon, assets, audio, launcher,
status, display-geometry, …) RE-TARGET the `impl … Host for HostState`
block to the new bindgen trait (same method bodies; enum type paths
swap) and delete the matching delegation block in
`consolidated_impl.rs`. Mechanical, file-per-file, compiler-guarded.

*C2 — deletions (~3,700 lines of host code):*
- `canvas_impl.rs` (2,314 lines): the legacy canvas trait impl — the
  38-verb `bc-*` family, text-blob machinery + raster caches, the u32
  maps (images/shaders/pictures/recorders/drawables/text-blobs),
  `recording_stack` modal routing (0.0.2 recordings are table
  resources; once Compose is on `scene`, nothing uses it).
  KEEPS: SkiaRenderer core (surface/EGL/base-matrix/flush/canvas(),
  font-collection + get-typeface), the WasiDrawable wrapper + FFI
  (scene's machinery), dihedral transform. ~60% shrink.
- `paragraph_impl.rs` (297 lines): delete entirely.
- `wasi_canvas_impl.rs` (1,139 lines, 0.0.1): delete entirely; the
  0.0.2 impl renames into its place.
- `lib.rs`: `mod bindings` (the skiko-ui world) + its 23 `SkikoUi`
  usage sites → probe-only instantiation (dispatch fallback arms in
  input.rs die; standalone/zygote/preload typed paths rework);
  `key_input_bindings` (superseded by input-handlers) and the legacy
  `frame_pacing_bindings` (superseded by ui-shell pacing) die.
- `input.rs`: 0.0.1 GuestInput fields + legacy dispatch arms +
  `input_handlers_bindings` (0.0.1).
- WIT trees: `wit/skiko-gfx.wit` + every consumer mirror deleted (the
  WIT-sync rule retires from CLAUDE.md with it); proposals' 0.0.1
  canvas/input trees dropped, `wit-0.0.2` renames to `wit/`.

*Guest-side cleanup rides Phase B, not C:* skiko's legacy arms (the
`wc()` gate fallbacks, `witAttrs`, blob paths, `node/` legacy verbs,
the `.bak/.orig/.checkpoint` junk files), wandr-app's smoke-test +
log-message scatter.

## Acceptance check (family goal)

| Criterion | Status |
|---|---|
| Clean | shell = exactly the cross-framework set; services = wandr:* like every other service; drawing/input untouched |
| No overlap | each legacy interface has exactly ONE new home (table above); shell-events carries only the legs input-handlers scoped out |
| WASI-shaped | shell is wandr-namespaced (embedder contract — the convergence doc's §4 "platform remainder" position: candidate small wasi packages LATER, not squatted now); logging adopts the real upstream package |
| 100% consumable | the universal row is the proven slint/dioxus/Compose set + the Avalonia/Flutter/egui memos' needs (metrics/theme/ime/clipboard/lifecycle) |
