---
name: reference_slint_wasip2
description: "Slint WORKS on wandr (task 100 COMPLETE 2026-06-11, user-verified) — crates/slint-wandr renderer + wandr.slint.test; gotchas = detect_operating_system browser-trap, ESC-as-hide convention, emoji via fontique GenericFamily::Emoji; dioxus-canvas stays production unless the DSL earns a switch"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**STATUS: SHIPPED + USER-VERIFIED (task 100, 2026-06-11) — and since the
same day slint-wandr is ALSO the wasi:canvas PROVING CONSUMER: it draws
exclusively through the draft (proposals/wasi-canvas; my:skiko-gfx keeps
only window+ime; embedding handoff = get-graphics + begin/end-frame).
‼️ wandr.slint.test now requires a host built with --features
wasi-canvas; the my:skiko-gfx canvas era of this crate ended at commit
4c66471a (check out before fc9ff5c3 for the old backend).**
`crates/slint-wandr` (Platform/WindowAdapter/RendererSealed/ItemRenderer/
GlyphRenderer + `launch!` macro) + `apps/user/wandr.slint.test`. Render +
touch + typing + selection + scroll + animation + IME summon/dismiss +
emoji + multiline TextEdit all verified live on device (~0.7% idle CPU).
Implementation gotchas (beyond the browser-trap below): keyboard hide
button = ESC (task-47 convention — blur the focus item on it; swallow so
`back` still works); tap-outside dismissal = post-dispatch rect test on
the focus item; emoji = register the DEVICE NotoColorEmoji (via the
/system-fonts preopen) under `fontique::GenericFamily::Emoji` — what
parley queries for emoji clusters (SLINT_FONT_PATH would CLOBBER Inter's
generic chains instead); wit-bindgen `generate!` with `inline` ALSO parses
the crate's default `wit/` dir → `path: []` to stay hermetic. Original
investigation (2026-06-10) below.

**Slint on wandr guests — investigated 2026-06-10, spike completed same day
(probe: /tmp/slint-probe; source inventory: /tmp/slint @ master 1.17.0-dev).**

- ✅ **Compiles for wasm32-wasip2**: `slint = { default-features = false,
  features = ["compat-1-2", "renderer-software", "std"] }` → 4.4 MB component,
  ONLY `wasi:*` imports. The no_std combo does NOT build (browser-assuming
  wasm32 cfg paths). ✅ **The parley text stack also compiles**:
  `i-slint-core --features std,shared-parley` (parley 0.10 + fontique + skrifa
  + icu_normalizer) builds clean for wasm32-wasip2.
- **The old "text wall" is GONE on Slint master**: `renderer-skia` no longer
  uses SkParagraph — text is shaped GUEST-SIDE by parley
  (`i_slint_core::textlayout::sharedparley`); the renderer seam is the
  `GlyphRenderer` trait: it receives font-data blobs + glyph IDs + positions
  and draws via `canvas.draw_glyphs_at`. All text metrics
  (`text_size`/`char_size`/`font_metrics`/cursor mapping) are shared
  `sharedparley::*` helpers any renderer reuses wholesale. Glyph IDs stay
  consistent because host typeface is created from the SAME font bytes the
  guest shaped with (fonts: guest-embedded or fetched once via `assets.read`,
  registered with fontique — no system-font discovery on wasi).
- **Right seam = custom renderer crate** (`i-slint-renderer-wandr`)
  implementing `RendererSealed` + `ItemRenderer` (~25 high-level methods:
  draw_rectangle/border/image/text/path/box_shadow, clip/opacity/layer
  visits) + `GlyphRenderer`, exactly like `FemtoVGRenderer` (the proven
  non-skia template, internal/renderers/femtovg). The user's
  impostor-skia-safe idea (the skiko trick at the Rust level) WOULD work —
  Slint's skia canvas surface is small (~20 Canvas methods, Paint/Path/Image/
  gradients all map to existing WIT) — but it's strictly more work: you'd
  replicate skia-safe types/Handle semantics AND still need the same WIT
  additions. Direct ItemRenderer is the same mapping minus the type
  impersonation. i-slint-* internal crates are published but semver-unstable
  → pin exact version (Slint's own renderers do the same).
- **WIT gaps: CLOSED 2026-06-11** — the additive batch shipped (6 verbs:
  `create-typeface`/`drop-typeface`, `draw-glyphs` + `bc-` twin,
  `draw-shadow-rrect` + `bc-` twin), host impl in canvas_impl.rs
  (`guest_typefaces` map), WIT mirrored, host builds. Canonical contract +
  full union inventory (Compose/dioxus/Slint): **`docs/skia-wit-mapping.md`**.
  Everything else maps: gradients ✓ (linear/radial/sweep), images ✓,
  SVG-string paths ✓ (Slint paths are lyon → serialize), layers/opacity →
  save-layer + bc-* bitmap canvases ✓, clip ✓ (track clip stack guest-side
  like femtovg; no canvas query verbs needed). Colorize-by-gradient
  (image-filters) deliberately deferred.
- **Effort estimate**: renderer crate ≈ skia itemrenderer (1.3k lines) +
  renderer/lib glue (~500) + Platform/WindowAdapter wiring to wandr's
  input/frame-pacing/IME ≈ 2–3 wandr-task-units; WIT additions are a day
  (plus shared-WIT rebuild-all-consumers + zygote restart). Parley switch is
  master-only (1.17-dev) — build against git or wait for release.
- ‼️ **Runtime browser-trap on wasip2** (bit us 2026-06-11, M4):
  `i_slint_core::detect_operating_system()` is cfg'd `target_family="wasm"`
  → `web_sys::window()` → js-sys "cannot access imported statics" panic →
  SIGILL — called on EVERY key event (TextInput shortcut matching), so it
  only fires on first keystroke, not render. Fix: set the public
  `OPERATING_SYSTEM_OVERRIDE` (→ Android) in init_platform BEFORE any
  Slint code runs (slint-wandr does). Same class of lurkers: `sys-locale`
  (js feature) fires if bundled translations are used — untested.
- ‼️ **Continuous/data-driven animation needs a live Slint `Timer` (cost 6
  device iterations, task 108 visualizer, 2026-06-15).** The host renderer is
  ON-DEMAND: it reschedules the next render ONLY via the guest's
  `next_frame_delay()` (slint-wandr line ~682), polled by the host AFTER each
  render. When the guest looks idle that returns up to 60 s, so `next_render_at`
  is pushed far out — and `needs_redraw`/`window().request_redraw()` set later
  from a bg-tick CANNOT pull it back (the host only renders on
  `now>=next_render_at` or a host-side `dirty` from input/timers). So a
  visualizer whose data updates in bg-tick never repainted — the surface sat at
  ~4.5 fps (measured via `dumpsys SurfaceFlinger --latency wandr#NNNN`). FIX:
  keep a repeating `slint::Timer` (≈16 ms) alive while animating; a pending
  Slint timer makes `duration_until_next_timer_update` → `next_frame_delay`
  return ~16 ms, so the host renders continuously (4.5 → ~35 fps). Its callback
  calls `request_redraw`. Bootstraps off the input event (tap) that started the
  animation (input → render → re-poll). Stop the timer when not animating so
  idle stays on-demand-cheap. (A perpetual Slint animation / `animation-tick()`
  binding works too but has the same bootstrap need + harder to gate.)
  `set_row_data` on a PERSISTENT model (not a fresh ModelRc each frame) for the
  animated data → partial redraw, not a repeater rebuild.
- License: tri-license (GPLv3 / royalty-free / commercial).
- **Verdict vs dioxus-canvas**: now a real option, not a wall — but still not
  worth a migration for its own sake; dioxus-canvas already has
  input/IME/rotation/frame-pacing integrated. Adopt only if the Slint DSL /
  widget set / animations earn it; derisk with one test app
  (wandr.slint.test) implementing the renderer against the 4 new verbs.
