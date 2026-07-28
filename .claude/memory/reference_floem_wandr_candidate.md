---
name: reference_floem_wandr_candidate
description: "Floem (lapce native Rust UI, Leptos-lineage reactivity) as a wandr guest candidate. SPIKE 2026-07-28: renderer is top-tier decoupled — clean Renderer trait over peniko/kurbo, 4 backends incl. Skia + a GPU-free CPU one. PROVEN wasip2-clean: floem_reactive as-is; floem_renderer + floem_tiny_skia_renderer after a 1-file winit-cut. Obstacle is winit in core's window/app/event layer (23/131 files) — bounded fork, not a rewrite. Not built end-to-end."
metadata:
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-07-28T20:03:21.170Z
---

**Floem = the strongest web-adjacent / reactive-Rust guest candidate examined so
far** — it solved the exact logic↔renderer decoupling Leptos refused to (Leptos
DELETED its renderer seam pre-0.7; see [[reference_dart_wasm_component_status]]
neighbors and the Leptos thread). Floem is `lapce/floem`: native Rust UI, fine-
grained reactivity whose primitives are **inspired by `leptos_reactive`** (same
lineage as dioxus-signals, which we already ship via
[[reference_dioxus_taffy_rust_ui]]).

## Renderer decoupling = A+ (this is NOT the obstacle)
`renderer/src/lib.rs` `trait Renderer` is a clean drawing interface over the
**peniko/kurbo** vocabulary (linebender types; also used by vello/Blitz/xilem):
`set_transform(Affine)`, `clip(Shape)`, `push_layer`/`pop_layer`,
`fill(Shape,BrushRef,blur)`, `stroke(Shape,BrushRef,StrokeStyle)`,
`draw_glyphs(GlyphRunProps, Iterator<Glyph>)` (text arrives PRE-SHAPED),
`draw_svg`, `draw_img(Img,Rect)`, `begin`/`finish`. Every method maps **1:1 onto
a Skia canvas / our WIT canvas verbs** (Affine→matrix, kurbo Shape→SkPath, peniko
BrushRef→SkPaint, push_layer→saveLayer, draw_glyphs→drawGlyphs).
**Four independent backend crates implement it** — `floem_skia_renderer`
(skia-safe — the SAME Skia we build in-host!), `floem_tiny_skia_renderer`
(pure-CPU raster, no wgpu), `floem_vello_renderer` (wgpu), `floem_vger_renderer`
(wgpu). Four backends = the seam is real + exercised, opposite of Leptos's
deleted one. A 5th "floem-wandr-canvas" backend = implement ~10 trait methods
translating peniko/kurbo → our WIT canvas verbs (small, pure-Rust).

## SPIKE RESULTS (2026-07-28) — repro: `repros/floem-wandr-spike/` (patched clone)
Built against installed `wasm32-wasip2`. Findings:
- ✅ **`floem_reactive`** compiles for wasip2 **as-is** (the Leptos-lineage
  reactive core is clean).
- ❌→✅ **`floem_renderer`** initially FAILS wasip2: winit leaks in via ONE file
  `renderer/src/gpu_resources.rs` (wgpu adapter/device acquisition — 3 refs,
  `winit::window::{Window,WindowId}`, nothing in the trait or CPU path). Any
  winit in the graph → `compile_error!("platform not supported by winit")`.
  **Surgical cut** (verified): make `winit` optional in `renderer/Cargo.toml`,
  add `gpu = ["dep:winit"]` feature, `#[cfg(feature="gpu")] pub mod
  gpu_resources;`. → **both `floem_renderer` AND `floem_tiny_skia_renderer`
  then compile clean for wasip2.** So the renderer half is wasip2-viable; the
  winit dep there was pure GPU-surface plumbing, not intrinsic.
- ❌ **`floem` core** — `winit` + `ui-events-winit` are **non-optional** deps
  (renderer backends are optional, winit is not). Coupling is **localized, not
  smeared**: **23 / 131 rust files, 118 refs**, concentrated in `src/window/*`
  (id/handle/mod/mock/state/tracking), `src/app/handle.rs` (12), `src/event/mod.rs`
  (5) — i.e. the window/app/EVENT-LOOP layer, exactly the layer wandr SUPPLANTS
  (host owns surface, drives frame ticks, feeds pointer/key over WIT). The
  view/layout/style/reactive bulk (~108 files) is winit-free. Note: floem already
  ships **`src/headless.rs` + `src/window/mock.rs`** (winit only 2×/8×) — the
  likely starting seam for a host-driven window adapter.

## SPIKE ROUND 2 (2026-07-28) — core dep-set PROVEN wasip2-clean; winit is the SOLE blocker
Answered open-Q(1). repro: `repros/floem-dep-probe/` (core's non-winit deps at
floem's versions, built for wasip2):
- ✅ **`peniko` + `raw-window-handle` + `swash` (text) + `taffy 0.9.2` (layout) +
  `image 0.25` + `resvg 0.46` (SVG, incl. its fontdb/memmap2 chain) ALL compile
  for wasip2.** So floem core's rendering/text/layout/SVG dependency surface is
  wasip2-clean — **no dependency dead-ends.** (taffy already known-good from
  dioxus-canvas.)
- ⇒ **winit is the ONLY hard wasip2 blocker in core** (plus native clipboard/menu
  deps `copypasta`/`clipboard-win`/`muda` and `sys-locale`, all native-only /
  target-gated / optional → trivially feature-cut).

**winit-excision surface sized** (`grep winit:: src/`): mostly SHIMMABLE small
types, not deep logic — `WindowId` (13×, opaque newtype), `Theme`(Light/Dark)
(19×, 2-variant enum), `ResizeDirection`/`WindowLevel`/`WindowButtons`/`Icon`/
`Fullscreen`/`LogicalPosition`/platform decoration opts (macos `OptionAsAlt`,
windows corner/backdrop, web) — all local-shimmable / cfg-gate-to-native. Only
`winit::window::Window` (7×) is real machinery. Files split **DEEP ~7**
(`window/handle.rs`, `window/mod.rs`, `app/handle.rs`, `event/mod.rs`,
`window/state.rs`, `window/tracking.rs`, `headless.rs` — the event-loop/window
layer wandr SUPPLANTS) vs **SHALLOW ~16** (1–3 refs each: paint, layout/screen,
view/id, inspector, event/listener+dispatch, app/mod, style/theme, action — just
consume the shimmable types). Excision ≈ a local shim module (~10 types) + gating
the ~7 machinery files behind a native feature + a host-driven window/event
adapter. Bounded + mechanical, same shape as Slint Platform / dioxus event-inject.

## Verdict + integration path
Renderer decoupling is **confirmed top-tier** (better than dioxus — Skia backend
already exists, CPU backend wasip2-clean with a 1-file cut). **The obstacle moved
off the renderer entirely onto winit in core's window/app/event layer** — a
BOUNDED fork job (feature-gate winit out of ~23 files; provide a host-driven
window+event adapter starting from headless.rs/mock.rs; feed host frame-tick→paint
and host pointer/key→floem dispatch), **not a rewrite**. This is the same shape of
work every guest framework needed (Slint Platform, dioxus event injection, egui
raw input).
Path: floem_reactive + floem views/layout/style (fork, winit-cut) + a 5th
`floem-wandr-canvas` Renderer backend, driving CPU/our-canvas (no wgpu in the
guest graph).
**Open questions before committing:** (1) ✅ ANSWERED — core's non-winit dep-set
(swash/resvg/image/taffy/peniko/raw-window-handle) all build wasip2; winit is the
sole blocker (see round 2). Still open: (2) can events be injected without winit
via the `ui-events` abstraction (vs the `ui-events-winit` bridge)? (3) reactor-
shape fit — floem assumes it runs the loop; wandr drives it. (4) actually perform
the winit-excision + a `floem-wandr-canvas` backend and get a green whole-`floem`
wasip2 build (that's the port itself, not a spike). Contrast the tradeoff with dioxus-canvas (minimal, fully-owned)
vs floem (batteries-included widgets/layout/animation for free, at the cost of
taming winit).

Related: [[reference_dioxus_taffy_rust_ui]] (same signal lineage, the
already-shipped alternative), [[reference_slint_wasip2]] (Platform-adapter
precedent), [[reference_egui_wandr]], `docs/wasm-component-language-support.md`.
