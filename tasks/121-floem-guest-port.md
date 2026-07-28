# Task 121 — Floem as a wandr guest lane (SCOPED / not started)

> **Status: SCOPED, not started.** Scoped 2026-07-28 from a read-only spike (repros
> `floem-wandr-spike/` + `floem-dep-probe/`), which took Floem from "interesting
> lead" to **de-risked port candidate** — no dependency dead-ends remain, winit is
> the sole wasip2 blocker in core, and its surface is sized. This is a **guest-side
> library port**: no WIT change (rule #4), and the aim is **zero host change** —
> the renderer targets the existing `wasi:canvas` verb set (the one dioxus-canvas
> and slint-wandr already exercise). Full assessment: `[[reference_floem_wandr_candidate]]`.

## What / why

Add **Floem** (`lapce/floem` — native Rust UI, fine-grained reactivity whose
signals are Leptos-lineage, same family as dioxus-signals) as a guest framework,
the way we shipped Slint and dioxus-canvas. It buys a **batteries-included
reactive-Rust lane** — widgets, taffy layout, styling, animation, and a mature
`Renderer` abstraction — complementing the two Rust options we already run:
`[[reference_dioxus_taffy_rust_ui]]` (minimal, fully-owned) and
`[[reference_slint_wasip2]]` (declarative DSL). It is also the concrete,
**browser-free** answer to the recurring "Leptos on wandr?" question: Leptos's
view layer is welded to the DOM and it *deleted* its renderer seam pre-0.7, so
Leptos is ruled out; Floem is what "Leptos-style reactivity + a swappable native
renderer" actually looks like.

## What the spike already PROVED (de-risking — don't re-derive)

Built against installed `wasm32-wasip2`:
- ✅ **`floem_reactive`** (the reactive core) compiles for wasip2 **as-is**.
- ✅ **`floem_renderer`** (the `Renderer` trait over peniko/kurbo) + **`floem_tiny_skia_renderer`**
  (pure-CPU backend) compile for wasip2 **after a 1-file winit-cut** — winit
  leaked only via `renderer/src/gpu_resources.rs` (wgpu surface setup); gate it
  behind a `gpu` feature and it's gone. The trait vocabulary (`set_transform`,
  `clip`, `push_layer`/`pop_layer`, `fill`, `stroke`, `draw_glyphs`(pre-shaped),
  `draw_svg`, `draw_img`) maps **1:1 onto our `wasi:canvas` verbs**.
- ✅ **Core's non-winit dep-set — `swash` (text), `resvg` (SVG, incl. fontdb/memmap2),
  `image`, `taffy 0.9.2`, `peniko`, `raw-window-handle` — ALL compile for wasip2.**
  No dependency dead-ends.
- ⇒ **winit is the ONLY hard wasip2 blocker in `floem` core** (plus native
  clipboard/menu deps `copypasta`/`clipboard-win`/`muda`/`sys-locale` — all
  native-only/target-gated/optional → trivially feature-cut).
- **winit-excision sized:** mostly SHIMMABLE small types (`WindowId`, `Theme`,
  `Icon`, `Fullscreen`, `ResizeDirection`, `WindowLevel`, `LogicalPosition`,
  platform decoration opts) + only `winit::window::Window` (7×) is real machinery.
  Files: **~7 DEEP** (`window/handle.rs`, `window/mod.rs`, `app/handle.rs`,
  `event/mod.rs`, `window/state.rs`, `window/tracking.rs`, `headless.rs` — the
  event-loop/window layer wandr supplants) vs **~16 SHALLOW** (1–3 refs, just
  consume the shimmable types). Existing `headless.rs` + `window/mock.rs` are the
  likely adapter seed.

## Milestones

- **M1 — Fork + winit shim.** Vendor/fork floem (decide submodule under `external/`
  vs vendored crate — ASK before adding a submodule). Make `winit` +
  `ui-events-winit` optional in core; add a small `floem_shim` module with local
  `WindowId`/`Theme`/`Icon`/`Fullscreen`/… replacing the winit re-exports;
  cfg-gate the platform-decoration opts (macos/windows/web) to native. Gate the
  ~7 DEEP machinery files behind a `native`/`winit` feature. **Exit:** whole
  `floem` crate compiles for `wasm32-wasip2` (`--no-default-features` + our
  feature set), no winit in the guest graph.
- **M2 — `floem-wandr-canvas` Renderer backend.** A 5th backend implementing
  floem's `Renderer` trait by translating peniko/kurbo calls into `wasi:canvas`
  verbs (the set dioxus-canvas/slint-wandr use; Affine→matrix, Shape→path,
  BrushRef→paint, push_layer→save-layer, draw_glyphs→draw-glyphs, draw_img→
  draw-image). **No host/WIT change** — reuse existing verbs; if a genuine gap
  appears, STOP and ask (rule #4). **Exit:** a static Floem view tree renders to
  the wandr canvas on the **desktop dev loop** (`[[project_desktop_dev_loop]]`).
- **M3 — Guest-side event adapter (NO host work).** The host already drives guests
  — the shared runtime owns the surface, drives frame ticks, and feeds
  pointer/key/lifecycle/resize over WIT; every guest consumes it. This milestone
  writes floem's equivalent of that shim (cf. `crates/slint-wandr/src/lib.rs` /
  `crates/dioxus-canvas/src/launch.rs`): implement `wasi::input_handlers::{frame,
  pointer,key}_handler::Guest` (`on_frame`/`on_resize`/`on_pointer`/`on_key`) +
  `wandr::ui_shell::shell_events::Guest` (`on_lifecycle_changed`) INSIDE the floem
  fork, routing each into floem's event dispatch and **replacing floem's winit
  event loop** (floem gets these from winit today; we hand it ours instead).
  Start from `headless.rs`/`mock.rs`; probe whether the `ui-events` abstraction
  accepts injected events without the `ui-events-winit` bridge (open-Q 2).
  **Exit:** a Floem app is interactive on desktop (tap/scroll/type/animate).
- **M4 — Text/glyph path.** `draw_glyphs` takes pre-shaped runs from swash — wire
  floem's shaping (swash) → our glyph draw / host text infra; verify fonts +
  emoji (reuse the Slint fontique/NotoColorEmoji learnings).
- **M5 — `wandr.floem.demo` wandrpkg** (mirror `wandr.dioxus.demo`/`wandr.slint.test`)
  — a component gallery; **device-verify on the Pixel 2 XL** (AOT `.cwasm`), visual
  check WITH the user (`[[feedback_visual_verification]]`).
- **M6 — IME/lifecycle if earned** (same tail as Slint task 100 M5).

## Constraints (binding)

- **Guest-side only.** No WIT contract edits (rule #4) — the renderer targets the
  existing `wasi:canvas`/input verbs; **zero host change is the goal**. Any
  perceived verb gap → stop and ask.
- **Don't hardcode** device geometry/density — derive (rule #2); Floem gets size
  from the host surface, not constants.
- **No new git branch** without asking (rule #5); work on `main`.
- **Fork discipline:** the winit-cut + shim is a real fork of floem — keep the
  diff minimal + documented for upstreamability (some of it, e.g. `gpu` feature-
  gating `gpu_resources`, is a clean upstream PR to lapce).

## Open questions / risks

1. **~~Do non-winit core deps build on wasip2?~~** ✅ answered by the spike (all do).
2. **Event injection without winit** — does `ui-events` (vs `ui-events-winit`)
   expose a seam to feed host events? (M3 probe.)
3. **Reactor-shape fit** — floem assumes it *owns* the loop; wandr drives it via
   frame-tick + bg-tick. The adapter must invert control cleanly (same problem
   solved for Slint's Platform/WindowAdapter).
4. **Fork-maintenance cost** vs the payoff — is a batteries-included Floem lane
   worth carrying a floem fork, given dioxus-canvas already covers reactive Rust?
   This is the real **adopt-vs-skip** decision, made at M2/M3 once effort is
   concrete (mirrors Slint's open "adopt-vs-dioxus" call).
5. **wgpu/GPU backends are out of scope** — drive CPU/our-canvas only; vello/vger
   never enter the guest graph.

## Pointers
- De-risked assessment + spike detail: `[[reference_floem_wandr_candidate]]`.
- Repros (patched clone + dep-probe): `repros/floem-wandr-spike/`,
  `repros/floem-dep-probe/`.
- Precedents to mirror (all on the 2D `wasi:canvas` lane): `[[reference_slint_wasip2]]`
  (Platform adapter, task 100), `[[reference_dioxus_taffy_rust_ui]]` (canvas backend,
  task 59), `[[reference_avalonia_wandr]]`.
- **Why Floem fits `wasi:canvas` (lane check):** floem's `Renderer` is 2D vector —
  `fill(Shape)`/`stroke(Shape)`/`draw_glyphs`/`draw_img` over kurbo/peniko — which
  maps to Canvas2D-superset verbs (kurbo `BezPath`→SVG-path-string `draw-path`,
  `draw_glyphs`→`draw-glyphs`, `draw_img`→`draw-image`). Same lane as Slint/Avalonia.
  This is the OPPOSITE of egui, which is out-of-layer: egui's whole output is
  textured triangle meshes (`draw-mesh` = `SkCanvas::drawVertices`, NO Canvas2D
  analog) → belongs on a future `wasi-webgpu` GPU lane, not `wasi:canvas`. See
  `[[reference_egui_wandr]]` — egui is the counterexample, not a precedent.
