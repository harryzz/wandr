---
name: reference_dioxus_taffy_rust_ui
description: "dioxus-core + taffy LIGHT reactive Rust UI framework on wasm32-wasip2 — the Compose alternative for complex wart guests. SHIPPED as crates/dioxus-canvas/ (task 59, device-verified); demo apps/user/war.dioxus.demo/."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 5c0eb8cc-cdbc-4cfe-b6cf-7d5eb0c39607
---

**SHIPPED 2026-05-29 (task 59) — the renderer is now real + device-verified.**
`crates/dioxus-canvas/` is the "tiny Blitz" (VirtualDom → node arena → taffy
flexbox → canvas WIT draw verbs → pointer hit-test → dioxus event → re-render).
First reactive dioxus guest on device: `apps/user/war.dioxus.demo/` — a 5-tab
Compose-style component gallery (task 61): checkbox, switch, radio, stepper,
progress, dropdown, color swatches, calendar, slider (drag), HSV color picker
(drag), and a text edit box with the real soft keyboard (IME). The renderer
supports flexbox, click, **drag (pointer move/up + capture + element-relative
coords)**, and **keyboard + focus + ime-attach** — all with **no new host WIT
verbs** (reuses `paragraph` for text measure, `ime` for editor-attach, and
`renderer.on-key-event-v2` for key delivery). Circles/pills via `border-radius:50%`
(rrect clamp); HSV gradients discretized into solid cells (no gradient primitive).
Reach for `dioxus-canvas` (path dep) for any rich Rust guest; hand-rolled
canvas (war.launcher ~70 KB) stays lighter for trivial UI. Impl gotchas baked
in: implement `WriteMutations` (NOT `rebuild_to_vec`, which drops the
`Template` payload); `replace_placeholder` paths are template-root-relative so
pop the `m` nodes first; install the global dioxus-html `HtmlEventConverter`
(`set_event_converter`) or the first event panics; the `Serialized*` data
types are `serialize`-gated (off) so hand-roll `HasMouseData`; text is measured
via the existing host `paragraph` interface (build single-run paragraph →
layout unconstrained → get-max-intrinsic-width + get-height → drop; cached) —
**no new host WIT verb** (an initial `measure-text` verb was added then removed).
Renderer is WIT-agnostic via a `CanvasSink` trait (host-unit-testable). See
`tasks/59-dioxus-canvas-renderer.md`.

**A dioxus guest is now PURE dioxus + one line (2026-05-30, `launch!` macro).**
A guest is just its components + `dioxus_canvas::launch!(app)` (optional
`pre_frame: |r| ...` hook), depending ONLY on `dioxus` + `dioxus-canvas` — **no
`.wit` file, no `wit_bindgen::generate!`, no `HostSink`, no `export!`, no
wit-bindgen dep** in source or manifest. How (`crates/dioxus-canvas/src/launch.rs`):
the library stays WIT-agnostic; the macro **expands in the guest cdylib** (where
component exports must live) and emits ONE `wit_bindgen::generate!` over the full
`my:skiko-gfx` world (imports canvas/paragraph/ime + exports renderer/frame-pacing)
+ the `CanvasSink` host adapter + `measure_text`/`editor_attach`/`editor_detach`
helpers (components call these, not raw WIT) + the renderer/frame-pacing Guest
impls. Key wit-bindgen 0.57.1 levers: `pub_export_macro` + `export_macro_name`
(so the `#[macro_export]` export-wiring macro is callable across the macro
boundary — there's NO inline `exports:` option in 0.57), and `runtime_path:
"::dioxus_canvas::__wit_bindgen::rt"` + a `pub use wit_bindgen as __wit_bindgen`
re-export (so the guest needs no wit-bindgen dep). **Traps:** a single generate!
only — two generate! both declaring `package my:skiko-gfx` → "re-add package"
error (don't split imports/exports across crates); and **delete the guest's old
`wit/` dir** — generate! auto-discovers `./wit` even with `inline:`, double-
declaring the package. Reference generated modules as `crate::my::skiko_gfx::*`
/ `crate::exports::*` (absolute — bare paths fail macro hygiene). See
[[feedback_clean_library_usage]].

**Engine-backed guest (extra host import) — `launch!` split (2026-05-30, task 67
item 2).** A guest that needs an EXTRA host import (e.g. `wart:signal/chat`)
**can't** use `launch!`: a second `generate!` for the extra import conflicts on
`_rt` / `cabi_realloc` / `__WIT_BINDGEN_COMPONENT_TYPE` / the component-type
section. So `launch!` is now split into composable halves (backward-compat —
`launch!` = both; demo + 9 render tests pass): `skiko_world!()` (the `my:skiko-gfx`
`generate!`) + `wire!(app, pre_frame: …)` (the CanvasSink/IME/renderer wiring,
which only assumes the skiko bindings exist). The engine-backed guest does ONE
**combined** `generate!` itself — skiko world + the extra import in one `wit/`
dir, the extra package under `deps/`, with `generate_all` + `pub_export_macro` +
`export_macro_name: "__dioxus_canvas_export"` + `runtime_path:
"::dioxus_canvas::__wit_bindgen::rt"` — then calls `dioxus_canvas::wire!(app)`.
Inline multi-package (`package a { } package b { }`) is REJECTED by the inline
parser → use the `path:"wit"` + `deps/` layout. Example: `repros/signal-ui`
(imports chat, exports renderer/frame-pacing; `wac plug`s onto signal-engine).
**Polling an external source** (engine `poll-events`, which only advances when
called): `DomRenderer::set_min_frame_delay(ms)` lowers the on-demand idle floor so
the host keeps calling `render_frame`; `mark_dirty()` forces a re-diff on change;
the guest root calls `dioxus::core::needs_update()` (NOT in the `dioxus` facade
prelude — `dioxus::core` is the `dioxus_core` re-export) to stay armed, and reads
a `thread_local` model that the `pre_frame` hook updates. See
[[project_wart_step_executor]], [[project_signal_client_architecture]].

**Edit-field gotcha — IME won't attach without an `onmousemove` (2026-05-30).**
dioxus-canvas only dispatches `onmousedown`/`onmouseup` for **draggable** elements
— ones that registered an `onmousemove` listener (`F_MOVE` in `DomRenderer`). A
text field with only `onmousedown` (+ `onkeydown`) **focuses** on tap (the renderer
sets focus from the `data-input` `focused` attr → caret shows) but its
`onmousedown` is NEVER fired, so a handler that calls `editor_attach` there never
runs and the soft keyboard never appears (symptom: "tap field → caret shows but
frozen, no keyboard"). Fix: every composer/edit field needs an `onmousemove`
(the demo's `EditField` has one for drag-select; reuse `caret_at`-style tap→caret
positioning via the `measure_text` helper). Confirmed via logcat:
`arbiter: attach-editor … → route to war.ime.keyboard … delivered=true` only
appears once the field is draggable.

**`display:none` now actually hides (2026-05-30 fix).** It WAS a no-op visually:
taffy gave the node a zero layout but `paint_walk` still drew it + its subtree at
(0,0) (symptom: a hidden element's text bled into the top-left corner). Fixed —
`paint_walk` early-returns on `display:none`. Useful pattern: to keep a focused
`data-input` alive for key routing while hiding it (e.g. hide a composer when the
soft keyboard is up), wrap it in a `display:none` bar — the element stays in the
DOM (ElementId stable, pointer-set focus + key dispatch survive) but paints
nothing. Per-frame UI scale: `DomRenderer::set_scale(2.0)` from `pre_frame` for a
hi-dpi panel (1× author px are tiny); remember anything sized in px (e.g. an
in-canvas QR module width) is multiplied by it — keep logical sizes small enough
that `size × scale` fits the panel.

Spike result (2026-05-29, `repos/dioxus-spike/`): **`dioxus-core` +
`taffy` is a viable light reactive Rust UI framework for wart guests** —
the leading Compose alternative for when a *complex* Rust guest UI is
needed (rich status bar, settings app, etc.).

**Verified on wasm32-wasip2:**
- Compiles — `dioxus 0.6` with `default-features=false, features=["macro","html","signals","hooks"]` + `taffy 0.7`. The key risk (dioxus pulling `wasm-bindgen`) is avoided by keeping `dioxus-web` off; nothing else needs it.
- **424 KB** release binary (framework + reactive component + layout engine) — ~37× lighter than Kotlin/Compose's 15.7 MB, and no continuation-leak / ~180 MB working-set baseline.
- Runs under `wasmtime`: `VirtualDom::rebuild_to_vec()` yields the mutation list; `taffy::compute_layout` yields correct flexbox geometry. `rsx!`/`use_signal`/`onclick` all work.

**Architecture fit:** our guest has no DOM / WebView / GPU — only the
high-level host-Skia **canvas WIT** (see
[[feedback_no_art_layer_dependencies]] for the boundary). So:
- **Do NOT use full `dioxus-native`/Blitz** — it pulls Servo's
  stylo/parley + **vello/wgpu** (GPU we lack); megabytes; likely won't
  build for wasip2.
- **Light path** = `dioxus-core` + `taffy` + a **custom canvas-WIT
  painter** (the one-time framework work, not yet built): consume
  VirtualDom mutations → node arena → map to taffy styles → compute
  layout → walk tree → emit `draw-rrect`/`draw-text-blob` (host fonts) →
  route `on-pointer-event-v2` back as dioxus events. Essentially "a tiny
  Blitz for our canvas WIT."

**egui was rejected** earlier (immediate-mode, tied to a GPU mesh +
font-atlas backend that doesn't map to our canvas WIT). dioxus-core wins
because its VirtualDom is renderer-agnostic.

**When to reach for it:** first genuinely complex Rust guest. For simple
system UI (launcher, basic taskbar) hand-rolled canvas-WIT drawing is
lighter — see the ~70 KB Rust `war.launcher` (the first non-Kotlin
renderer guest, `apps/system/war.launcher/`, task 57).
