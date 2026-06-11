# Task 100 — slint-wandr: Slint renderer over `my:skiko-gfx`

> Evaluation track (started 2026-06-11). dioxus-canvas stays the production
> guest-UI library until M4 proves Slint's DSL/widgets/animations earn a
> switch. Groundwork: `docs/skia-wit-mapping.md` (canonical Skia↔WIT
> contract; the 2026-06-11 additive glyph/shadow verb batch was built for
> this) + the `reference_slint_wasip2` memory (full investigation).

## Why it works (all de-risked 2026-06-10/11)

- Slint compiles for wasm32-wasip2 (`std` + `compat-1-2`; the no_std combo
  does NOT — browser-assuming wasm32 cfg paths).
- The parley text stack (`i-slint-core --features std,shared-parley`:
  parley 0.10 + fontique + skrifa + icu_normalizer) compiles clean for
  wasip2. Text is shaped GUEST-side; the renderer receives font bytes +
  glyph ids + positions (`GlyphRenderer` trait) — no host shaping needed.
- The plug point is `RendererSealed` + `ItemRenderer` (~25 high-level
  methods) + `GlyphRenderer`, exactly what `FemtoVGRenderer`
  (internal/renderers/femtovg) implements — the proven non-skia template.
  All text metrics delegate to shared `sharedparley::*` helpers.
- Host side is ready: `create-typeface`/`draw-glyphs`/`draw-shadow-rrect`
  (+ bc twins) shipped in wandr-host commit 30a61c12. NOT yet deployed to
  the device — M4 is the first live exercise (deploy host + restart stack).

## Pins / decisions

- **Slint pin: git rev `46cfde659f21de52bb0fa3693826ca99a6466d88`**
  (master, v1.17.0-dev — the parley switch is NOT in any release; re-pin
  forward consciously, the i-slint-* internals are semver-unstable).
- **License: royalty-free tier** for the in-repo evaluation app (Slint is
  GPLv3 / royalty-free / commercial — revisit before anything ships).
- Crate: `crates/slint-wandr` (guest-side, sibling of dioxus-canvas).
  Test app: `apps/user/wandr.slint.test`.
- Fonts: compile-time embedding first; `assets.read` + fontique
  registration as fallback.

## Milestones

- [x] **M1 — crate skeleton compiles for wasip2** (DONE 2026-06-11; gotcha: generate! with `inline` also parses the default wit/ dir — `path: []` makes launch! hermetic): WIT bindings (inline
      trimmed canvas/window import + renderer/frame-pacing export, the
      dioxus-canvas `launch.rs` pattern), `Platform` +
      `WindowAdapter`, `RendererSealed` delegating text metrics to
      `sharedparley`, event wiring (render-frame → draw pass,
      on-pointer-event-v2/on-key-event-v2 → Slint window events,
      on-resize, frame-pacing ← animation-active).
- [x] **M2 — ItemRenderer over canvas verbs** (DONE 2026-06-11, compile-verified; group opacity = save-layer + extra-restore counter, NOT bc-* offscreen — simpler than planned): rects/borders/images/paths/
      clip direct; opacity+clip layers via create-bitmap-canvas + bc-* +
      snapshot → draw-image; box shadows via draw-shadow-rrect; guest-side
      matrix/clip-stack tracking (femtovg pattern).
- [x] **M3 — text** (DONE 2026-06-11, compile-verified; typeface cache pins the parley blob — HashedBlob trick — so the ptr-keyed id stays valid): `GlyphRenderer::draw_glyph_run` → create-typeface
      (cached per font-blob hash, Slint FontCache pattern) + draw-glyphs.
- [x] **M4 — proof app on device** (DONE 2026-06-11, USER-VERIFIED interactive: touch/typing/selection/scroll/slider/animation all work; renders text/widgets/gradient/shadow/ListView correctly, ~0.7% idle CPU): wandr.slint.test exercising text
      (sizes/weights), a TextInput (cursor/selection = parley metrics
      path), scroll, image, drop shadow, opacity animation. Deploy the
      new-verbs host + full stack restart (stale-zygote rule). Visual
      verification with the user (subjective outcome rule).
- [x] **M5 (keyboard + emoji, USER-VERIFIED 2026-06-11)** — IME
      summon/dismiss + emoji fallback + multiline TextEdit all live:
      * summon: `WindowAdapterInternal::input_method_request` (via
        `WindowAdapter::internal`) → notify-editor-attached/-detached
        (byte→char offset conversion; per-keystroke `Update` ignored).
      * ‼️ first-keystroke SIGILL: `detect_operating_system()` on
        target_family=wasm calls web_sys → js-sys panic; fixed by setting
        `OPERATING_SYSTEM_OVERRIDE` → Android in init_platform.
      * hide button = ESC (task-47 convention): intercept while attached →
        blur focus item → Disable → detach; swallowed (back still works).
      * tap-outside dismissal: post-dispatch rect test on the focus item.
      * emoji: register device NotoColorEmoji (the /system-fonts preopen)
        under `fontique::GenericFamily::Emoji` (parley's emoji query).
      * TextEdit multiline + word-wrap + 2D cursor placement verified.
      Remaining (unscheduled follow-ups): clipboard wiring, lifecycle
      pause, font-scale, faux bold/italic + variable-font axes,
      colorize-by-gradient, per-corner radii, image tiling.

**TASK 100 COMPLETE** — Slint is a fully working guest-UI option on wandr.
The adopt-vs-dioxus-canvas decision stays open (per the recorded verdict:
dioxus-canvas remains production until the Slint DSL earns a switch).

## Known traps to carry in

- wasip2 single-thread: no thread assumptions; Slint's
  `unsafe-single-threaded` feature is the no_std path — we use `std` and
  stay on the one wasm thread.
- `i_slint_core` internal APIs shift between revs — every signature this
  crate touches must be checked against the PINNED rev, not docs.rs.
- frame-pacing: report busy only while Slint has running animations or a
  pending redraw — otherwise idle CPU regresses (see
  reference_on_demand_rendering).
