# Task 59 — dioxus canvas-WIT renderer ("tiny Blitz") + a real dioxus guest on device

> **Status:** 🔲 scoped 2026-05-29, not started. Builds directly on the
> 2026-05-29 feasibility spike (`repos/dioxus-spike/`,
> [[reference_dioxus_taffy_rust_ui]]) — which already proved the
> *foundation* is viable. This task is the **one-time framework work the
> spike explicitly did NOT do**: the custom renderer that drives the wart
> canvas WIT from a dioxus VirtualDom, plus a real interactive dioxus
> guest rendering on the Pixel 2 XL.

## Why

wart needs a *light reactive* UI option for guests more complex than a
hand-rolled canvas painter (`war.launcher` ~70 KB, `war.taskbar` ~48 KB
are hand-rolled and fine for trivial UI), but without Kotlin/Compose's
15.7 MB binary, continuation leak, and ~180 MB working set
([[feedback_indeterminate_progress_leak]], [[feedback_wart_zygote_fork_survival]]).
A rich status bar, a settings app, a notification shade, a file picker —
those want reactivity + flexbox layout, not 300 lines of manual
`draw_rect` math.

## What the spike already de-risked (do NOT redo)

From `repos/dioxus-spike/` (README + [[reference_dioxus_taffy_rust_ui]]):

- `dioxus 0.6` (`default-features=false`, features `macro,html,signals,
  hooks`) + `taffy 0.7` **compile to `wasm32-wasip2`** with **no
  `wasm-bindgen`** (the killer risk — `dioxus-web` is off).
- **424 KB** stripped release binary — ~37× lighter than Compose.
- Runs under wasmtime headless: `VirtualDom::rebuild_to_vec()` yields the
  mutation list; `taffy::compute_layout` yields correct flexbox geometry;
  `rsx!` / `use_signal` / `onclick` all work.
- **Rejected:** full `dioxus-native`/Blitz (pulls Servo stylo + parley +
  **vello/wgpu** — GPU we don't have; megabytes; won't build for wasip2).
  Also egui (immediate-mode, GPU-mesh/atlas backend, doesn't map to our
  canvas WIT).

So the heavy unknowns are closed. This task is **renderer plumbing**, not
a viability question.

## Goal / deliverable

1. A reusable **`dioxus-canvas` renderer crate** (the "tiny Blitz for the
   wart canvas WIT") — consumes a dioxus `VirtualDom`, lays out with
   `taffy`, paints via the trimmed `my:skiko-gfx` canvas WIT, and routes
   pointer/key input back as dioxus events.
2. A **concrete dioxus guest warpkg** (`war.dioxus.demo` or similar) that
   uses it — a small reactive UI (counter, a list, a button that mutates
   a signal) exported as `my:skiko-gfx/renderer`, installed + launched via
   the arbiter, **rendering + interactive on device**.

## The renderer (the 4 steps from the spike README)

A loop driven by the host's `renderer` export (the same export
`war.launcher` / `war.taskbar` implement):

1. **VirtualDom → node arena.** On `render_frame` (or on a dirty flag),
   `vdom.rebuild_to_vec()` once at start, then `vdom.render_immediate()`
   for incremental mutations. Apply the mutation list (`AppendChildren`,
   `CreateElement`, `SetAttribute`, `SetText`, `Remove`, …) into a flat
   node arena keyed by dioxus `ElementId`.
2. **Map → taffy styles → layout.** Translate element tag + attributes
   (a small CSS-ish subset: `display:flex`, `flex-direction`, `width/
   height`, `padding`, `margin`, `gap`, `flex-grow`, `align/justify`,
   `background`, `color`, `font-size`) into `taffy::Style`. For **text
   nodes**, taffy needs a measure function — text measurement must come
   from the host (Skia owns fonts), so add a `measure-text(text, family,
   size) -> (w,h)` canvas-WIT verb (or reuse the paragraph path) and feed
   taffy a leaf measure closure. Run `taffy::compute_layout` against the
   surface size (already inset by task 56 for fullscreen apps).
3. **Walk laid-out tree → canvas WIT.** Depth-first over the taffy tree;
   for each node emit `draw-rrect`/`draw-rect` (background, border-radius),
   `create-text-blob`+`draw-text-blob` (text, host fonts — the same path
   `war.statusbar` uses), `draw-path`/`draw-oval` as needed. Use absolute
   coords from taffy's computed layout.
4. **Route input → dioxus events.** On `on-pointer-event-v2`, hit-test the
   laid-out tree (top-most node whose rect contains the point), find its
   `ElementId`, and `vdom.handle_event("click", …, element_id, …)` →
   `vdom.process_events()` → re-render. Same for key events into a focused
   element. Mark dirty so the next `render_frame` repaints.

Keep it **incremental**: only re-layout/repaint when the VirtualDom
reports mutations or input fired — no per-frame churn (mirrors the
launcher's "layout once, replay" discipline).

## Scope guards

- **Minimal CSS subset only** — flexbox + box model + text + color +
  rounded corners. NOT a browser; no grid, floats, position:absolute
  (except popups later), animations, or CSS cascade/specificity. Inline
  styles / a tiny style map, not a stylesheet engine.
- **Text via host fonts** — never bundle a font or measure text in-guest
  (Skia owns metrics; see [[feedback_android_fonts]]). The measure-text
  verb is the one new host capability this needs.
- **No DOM/WebView/GPU** — canvas WIT only ([[feedback_no_art_layer_dependencies]]).
- Single component, `same-store`, leak-immune (no Kotlin) — installs +
  launches with zero wart-host changes (task 39 generic loader), except
  the one new `measure-text` WIT verb.

## Steps

| # | Step | Where |
|---|------|-------|
| 1 | Promote `repos/dioxus-spike/` learnings into a renderer crate skeleton; trimmed `my:skiko-gfx` WIT (canvas subset, as `war.launcher` has) | new `apps/system/.../` or `crates/dioxus-canvas/` |
| 2 | Mutation-applier: VirtualDom mutations → node arena | renderer crate |
| 3 | `measure-text` host WIT verb + impl (Skia metrics) + taffy leaf measure | `wit/skiko-gfx.wit`, `canvas_impl.rs`, renderer |
| 4 | tag/attr → `taffy::Style` mapper + `compute_layout` | renderer crate |
| 5 | Tree-walk painter → canvas-WIT draw verbs | renderer crate |
| 6 | Hit-test + `on-pointer-event-v2`/key → dioxus event → re-render | renderer crate |
| 7 | Demo guest warpkg (`war.dioxus.demo`): counter + list + button; pack via build-system-warpkgs.sh; install; launch via arbiter | `apps/.../war.dioxus.demo/`, build script |
| 8 | Device-verify: renders, taps mutate state + repaint, scrolls (if list), measures text correctly; capture binary size (expect < ~600 KB) | device |

## Open questions

1. **Crate home / shape:** a shared library crate (`crates/dioxus-canvas/`)
   that guest warpkgs depend on, vs a copy-paste starter? A shared crate is
   better long-term (one renderer, many guests) but the guests are separate
   `cdylib` components — the renderer compiles *into* each guest, so a
   normal Rust path/git dep works.
2. **Text measurement cost:** a host round-trip per text leaf per layout
   could be chatty. Cache `(text,family,size) -> (w,h)` in-guest; only
   measure on change. Is one `measure-text` verb enough, or batch?
3. **Scrolling / overflow:** taffy lays out the full content; clipping +
   scroll offset is the renderer's job (a clip-rect verb exists). Defer to
   a later step or include in the demo?
4. **Popups / layers:** dioxus has no portal concept by default — overlays
   (dropdowns, dialogs) would need a convention. Defer; the demo is flat.
5. **Is this needed yet?** No current guest *requires* it — the launcher /
   taskbar / status bar are hand-rolled and fine. This is the framework
   investment to make BEFORE the first genuinely complex Rust guest
   (settings, notification shade, file picker). Prioritize when such a
   guest is actually scoped, or to retire a Compose guest that leaks.

## Related

- `repos/dioxus-spike/` — the feasibility probe (README has the exact
  crate/feature set + the 4 renderer steps).
- [[reference_dioxus_taffy_rust_ui]] — the spike-result memory.
- `tasks/57-launcher.md`, `apps/system/war.launcher/` — the first
  non-Kotlin canvas-WIT renderer guest (hand-rolled; the precedent this
  generalizes), and the trimmed-WIT pattern (`matrix-3x3` rejected by
  guest wit-bindgen 0.46 → hand-author a canvas subset).
- `tasks/39-generic-dep-wiring.md` — generic loader: a new canvas-WIT
  guest installs/launches with zero wart-host changes (modulo the one
  `measure-text` verb).
- [[feedback_no_art_layer_dependencies]], [[feedback_indeterminate_progress_leak]],
  [[feedback_android_fonts]].
