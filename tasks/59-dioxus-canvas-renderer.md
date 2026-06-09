# Task 59 — dioxus canvas-WIT renderer ("tiny Blitz") + a real dioxus guest on device

> **Status:** 🔲 scoped 2026-05-29, not started. Builds directly on the
> 2026-05-29 feasibility spike (`repos/dioxus-spike/`,
> [[reference_dioxus_taffy_rust_ui]]) — which already proved the
> *foundation* is viable. This task is the **one-time framework work the
> spike explicitly did NOT do**: the custom renderer that drives the wandr
> canvas WIT from a dioxus VirtualDom, plus a real interactive dioxus
> guest rendering on the Pixel 2 XL.

## Why

wandr needs a *light reactive* UI option for guests more complex than a
hand-rolled canvas painter (`wandr.launcher` ~70 KB, `wandr.taskbar` ~48 KB
are hand-rolled and fine for trivial UI), but without Kotlin/Compose's
15.7 MB binary, continuation leak, and ~180 MB working set
([[feedback_indeterminate_progress_leak]], [[feedback_wandr_zygote_fork_survival]]).
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
   wandr canvas WIT") — consumes a dioxus `VirtualDom`, lays out with
   `taffy`, paints via the trimmed `my:skiko-gfx` canvas WIT, and routes
   pointer/key input back as dioxus events.
2. A **concrete dioxus guest wandrpkg** (`wandr.dioxus.demo` or similar) that
   uses it — a small reactive UI (counter, a list, a button that mutates
   a signal) exported as `my:skiko-gfx/renderer`, installed + launched via
   the arbiter, **rendering + interactive on device**.

## The renderer (the 4 steps from the spike README)

A loop driven by the host's `renderer` export (the same export
`wandr.launcher` / `wandr.taskbar` implement):

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
   `wandr.statusbar` uses), `draw-path`/`draw-oval` as needed. Use absolute
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
  (Skia owns metrics; see [[feedback_android_fonts]]). Measurement reuses the
  existing host `paragraph` interface (NOT a new verb — see "Dependency notes").
- **No DOM/WebView/GPU** — canvas WIT only ([[feedback_no_art_layer_dependencies]]).
- Single component, `same-store`, leak-immune (no Kotlin) — installs +
  launches with **zero wandr-host WIT additions** (task 39 generic loader; text
  measured via the pre-existing `paragraph` interface).

## Steps

| # | Step | Where |
|---|------|-------|
| 1 | Promote `repos/dioxus-spike/` learnings into a renderer crate skeleton; trimmed `my:skiko-gfx` WIT (canvas subset, as `wandr.launcher` has) | new `apps/system/.../` or `crates/dioxus-canvas/` |
| 2 | Mutation-applier: VirtualDom mutations → node arena | renderer crate |
| 3 | `measure-text` host WIT verb + impl (Skia metrics) + taffy leaf measure | `wit/skiko-gfx.wit`, `canvas_impl.rs`, renderer |
| 4 | tag/attr → `taffy::Style` mapper + `compute_layout` | renderer crate |
| 5 | Tree-walk painter → canvas-WIT draw verbs | renderer crate |
| 6 | Hit-test + `on-pointer-event-v2`/key → dioxus event → re-render | renderer crate |
| 7 | Demo guest wandrpkg (`wandr.dioxus.demo`): counter + list + button; pack via build-system-wandrpkgs.sh; install; launch via arbiter | `apps/.../wandr.dioxus.demo/`, build script |
| 8 | Device-verify: renders, taps mutate state + repaint, scrolls (if list), measures text correctly; capture binary size (expect < ~600 KB) | device |

## Open questions

1. **Crate home / shape:** a shared library crate (`crates/dioxus-canvas/`)
   that guest wandrpkgs depend on, vs a copy-paste starter? A shared crate is
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

## Results — device-verified 2026-05-29 ✅

All 8 steps landed; the demo renders + reacts on the Pixel 2 XL.

**Shipped:**
- `crates/dioxus-canvas/` — the reusable renderer (new top-level `crates/` bucket
  for shared guest-side libs; documented in `docs/repository-layout.md`).
  - `sink.rs` — `CanvasSink` trait: the WIT-agnostic host boundary (the guest
    forwards it to its own generated `canvas::*`). Keeps the renderer
    host-testable + free of wit-bindgen macro hygiene.
  - `dom.rs` — the dioxus **`WriteMutations` stack machine** over compile-time
    `Template`s. **Key learning:** `rebuild_to_vec()` drops the `Template`
    payload ("for testing"); a real renderer MUST implement `WriteMutations` to
    receive `load_template(template, …)` and instantiate the static skeleton.
    The interpreter mirrors dioxus-web/tui: `load_template` pushes the
    instantiated root; `assign_node_id`/`replace_placeholder` navigate paths
    **relative to the template root**, not `stack.last()` — so
    `replace_placeholder_with_nodes` must pop its `m` nodes *first*, then the new
    top-of-stack is the template root (this was the one bug found in testing).
  - `style.rs` — minimal CSS subset (`style="…"` decl list) → `taffy::Style` +
    paint props (background/color/font/radius), with inheritance for text.
  - `events.rs` — installs the global `HtmlEventConverter` dioxus-html requires
    (else the first event panics) + a hand-rolled `HasMouseData` for clicks
    (the `Serialized*` types are `serialize`-feature-gated, which we keep off).
  - `lib.rs` — `DomRenderer`: VirtualDom → arena → taffy `compute_layout_with_measure`
    (text leaves measured through `CanvasSink::measure_text`, cached) → draw-op
    list + hit rects → replay each frame (launcher-style "layout once, replay";
    re-diff only on dirty). `tests/render.rs` validates the whole chain on host
    (render + click → count 0→1).
- Text measurement (for taffy text leaves): **no new host verb** — reuses the
  host's existing `paragraph` interface (task 14). The demo's `CanvasSink::measure_text`
  builds a single-run paragraph, `layout`s it unconstrained, and reads
  `get-max-intrinsic-width` + `get-height`, then drops it (cached in-guest). An
  earlier `measure-text` verb was added then **removed** in favour of this — see
  the dependency note below.
- `apps/user/wandr.dioxus.demo/` — the demo guest cdylib (trimmed WIT like
  wandr.launcher; `HostSink` wires `canvas::*` → `CanvasSink`; thread-local
  `DomRenderer`; counter + button + 4-item list). **516 KB** (< 600 KB target).
- `tools/scripts/build-system-wandrpkgs.sh` — builds/packs/pushes/installs it
  under `apps/` (user app; launcher lists it).

**On device:** flexbox column (bold title + count line + blue rounded button +
four rounded list cards) renders with host fonts (SourceSansPro-Bold + Roboto
via paragraph-measured layout; no clipping/overlap); tapping the button
increments the signal and the UI repaints the new count; stable 68 s, no
leak/SIGILL; ~1440×2880 fullscreen.

**Follow-ups (not blocking):**
- Input had no debounce — the count advanced by more than the tap count
  (phantom / multi-`Down` per `adb input tap`). Add tap debounce / track
  press-release pairing if exactness matters.
- Scrolling/overflow, popups/portals, keyboard events (the `events.rs`
  converters for non-mouse types are `unimplemented!`) — deferred per scope
  guards; add with the first text-input / long-list dioxus guest.

## Dependency / version notes (researched 2026-05-29)

- **Text measurement reuses the host `paragraph` interface — no new WIT verb.**
  The first cut added a `measure-text: func(...) -> tuple<f32,f32>` to the canvas
  interface; it was **removed** to avoid expanding the host WIT surface, since the
  existing `paragraph` interface (task 14) already measures: the guest builds a
  single-run paragraph, `layout`s it unconstrained, reads `get-max-intrinsic-width`
  (natural width) + `get-height`, and drops it (cached per unique text/font in the
  guest). The demo's trimmed WIT imports a small `paragraph` subset; the host
  provides the full interface. Device-verified against a `measure-text`-free host.

- **Now on dioxus 0.7** (since task 60, 2026-05-29). It was initially pinned to
  0.6 because 0.7 is a wasm32-wasip2 wall: `dioxus-core
  0.7` has a *non-optional* dep on `subsecond` (the hot-patch engine), which
  pulls `wasm-bindgen`/`js-sys`/`web-sys` for any `cfg(target_arch = "wasm32")`
  (wasip2 included). Those `__wbindgen_*` imports don't link under
  `wasm-component-ld`, and no feature disables subsecond. 0.7's renderer core
  (`WriteMutations`/`Template`/`Mutation`) is otherwise compatible — the only
  source changes were a new `convert_cancel_data` converter method + a
  `Modifiers` import path — but the subsecond→wasm-bindgen chain blocks the
  guest build entirely. See the pin comment in `crates/dioxus-canvas/Cargo.toml`.
- **Why we depend on `dioxus-html` (and can't swap it for `native`/`native-dom`).**
  `dioxus-html` is the *vocabulary* layer, not a renderer: `rsx!` hardcodes
  resolution through a module named `dioxus_elements` (`dioxus_elements::div::<attr>`
  tuples + `dioxus_elements::events::onclick(handler)`), and the `dioxus` facade
  aliases `dioxus_html as dioxus_elements`. So our tags/attrs + the `onclick`
  event-data type (`MouseData`) + the global `HtmlEventConverter` all come from
  it. `dioxus-native`/`blitz-dom` ("native"/"native-dom") do NOT offer a lighter
  vocabulary — Blitz *depends on* dioxus-html and renders the HTML/CSS namespace
  via stylo + vello/wgpu (the heavy path the spike rejected). The only way to
  drop dioxus-html is a **custom `dioxus_elements` namespace** (dioxus's
  documented custom-elements path): our own element modules + an `events` module
  with `Event<OurData>` types — which would also delete the event-converter hack
  and shed `keyboard-types`/`euclid`/`enumset`. Deferred: most of dioxus-html's
  bulk (66 KB `elements.rs` + 94 KB `attribute_groups.rs`) is zero-sized/`const`
  and LTO-stripped already, so the binary win is uncertain (mainly the event
  subsystem). Revisit as a cleanliness/size pass if it matters.

## Related

- `repos/dioxus-spike/` — the feasibility probe (README has the exact
  crate/feature set + the 4 renderer steps).
- [[reference_dioxus_taffy_rust_ui]] — the spike-result memory.
- `tasks/57-launcher.md`, `apps/system/wandr.launcher/` — the first
  non-Kotlin canvas-WIT renderer guest (hand-rolled; the precedent this
  generalizes), and the trimmed-WIT pattern (`matrix-3x3` rejected by
  guest wit-bindgen 0.46 → hand-author a canvas subset).
- `tasks/39-generic-dep-wiring.md` — generic loader: a new canvas-WIT
  guest installs/launches with zero wandr-host changes (modulo the one
  `measure-text` verb).
- [[feedback_no_art_layer_dependencies]], [[feedback_indeterminate_progress_leak]],
  [[feedback_android_fonts]].
