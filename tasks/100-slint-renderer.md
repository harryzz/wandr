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
- [ ] **M3 — text**: `GlyphRenderer::draw_glyph_run` → create-typeface
      (cached per font-blob hash, Slint FontCache pattern) + draw-glyphs.
- [ ] **M4 — proof app on device**: wandr.slint.test exercising text
      (sizes/weights), a TextInput (cursor/selection = parley metrics
      path), scroll, image, drop shadow, opacity animation. Deploy the
      new-verbs host + full stack restart (stale-zygote rule). Visual
      verification with the user (subjective outcome rule).
- [ ] **M5 (only if M4 earns it)** — IME attach/detach, lifecycle,
      clipboard, density/font-scale from the `window` interface.

## Known traps to carry in

- wasip2 single-thread: no thread assumptions; Slint's
  `unsafe-single-threaded` feature is the no_std path — we use `std` and
  stay on the one wasm thread.
- `i_slint_core` internal APIs shift between revs — every signature this
  crate touches must be checked against the PINNED rev, not docs.rs.
- frame-pacing: report busy only while Slint has running animations or a
  pending redraw — otherwise idle CPU regresses (see
  reference_on_demand_rendering).
