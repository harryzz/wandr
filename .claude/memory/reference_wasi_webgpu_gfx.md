---
name: reference_wasi_webgpu_gfx
description: "wasi:webgpu vs our wasi:canvas = COMPLEMENTS, not competitors — two rendering lanes at different layers (guest-owns-GPU-pipeline vs host-owns-2D-rasterizer; the WebGPU-vs-Canvas2D split). Plus wasi-gfx surface/frame-buffer notes; sf_media child surfaces = the natural host primitive if we implement the GPU lane."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-07-28T20:35:49.457Z
---

## TL;DR — wasi:canvas vs wasi-webgpu: COMPLEMENTS, not competitors (recurring Q)
They sit at **different layers**, so they don't compete — they're two rendering
**lanes**:
- **wasi:canvas (ours)** = HIGH-level **2D drawing** (Canvas2D-superset, Skia-shaped):
  guest emits draw verbs (paths/fills/strokes/glyphs/images/clips/layers); **the
  HOST owns the rasterizer** + fonts. Tiny guests, OS-portable, host GPU-accel
  shared. Consumers = retained/declarative UI: Compose, Slint, Avalonia, SwiftUI,
  Flutter, dioxus, Floem (~95% of UI).
- **wasi-webgpu** = LOW-level **GPU pipeline** (faithful W3C WebGPU mirror, verified
  webgpu.wit): **the GUEST drives the pipeline** (buffers/bind-groups/WGSL/render
  passes/`draw`); host only lends the GPU device. Consumers = GPU-native: games
  (bevy), 3D, GPU compute/ML, egui/wgpu. NO high-level draw verb (no `draw-mesh`)
  → zero vocabulary overlap with canvas.
- **Browser precedent:** wasi:canvas ≈ Canvas2D-for-wasm; wasi-webgpu ≈
  WebGPU-for-wasm. Both are W3C APIs, complementary; one page (or one guest) uses
  either or both (2D HUD over a 3D scene). They **compose both ways** — sf_media can
  host a wasi:surface; a 2D canvas can sit on top of wasi:webgpu.
- **The one tension + why we keep both:** webgpu is strictly more powerful (you CAN
  build a 2D rasterizer on it — our host canvas backend already runs on GPU
  underneath), so it could *subsume* canvas in theory. But that puts a full
  rasterizer + font stack in EVERY guest (bloat, per-language, lost host accel/
  fonts, worse portability) — exactly wandr's anti-goal. So: rasterizer stays
  host-side (canvas = default); webgpu is the **escape hatch** for the minority
  needing raw GPU, NOT the default UI path.
- **wandr status:** wasi:canvas = the PRODUCTION lane (implemented; all shipped
  frameworks ride it). wasi-webgpu = a PLANNED second lane, to be implemented
  wholesale host-side (skia/EGL already give a GPU device) with **zero canvas
  changes / zero speculative verbs** — egui is the guest that would drive building
  it. See [[reference_egui_wandr]] (egui = the webgpu case; mesh, no Canvas2D
  analog), [[project_wasi_canvas_migration]]. Detail + this same split below /
  in `docs/skiko-gfx-vs-wasi-gfx.md`.

**wasi:webgpu + wasi-gfx — investigated 2026-06-10 (Slint follow-on).**

- `wasi:webgpu` (WebAssembly/wasi-webgpu, **Phase 2**, champions Mendy Berger /
  Sean Isom): the WebGPU spec as WIT — the GUEST owns the whole rendering
  pipeline (WGSL shaders, pipelines, command encoding); the host provides the
  GPU device. Windowing/display explicitly OUT of scope. Targets incl. Android.
- `wasi-gfx` (wasi-gfx org, sibling): `surface` (window/surface + input events:
  pointer/keyboard/resize/frame) and `frame-buffer` (raw pixel buffer — the
  CPU-raster path). Runtimes: `wasi-gfx-runtime` (hosts wasi-gfx + wasi:webgpu,
  wgpu-based) + a web shim. NO releases yet; API churn expected.
- **Model comparison:** wandr's `my:skiko-gfx/canvas` = HIGH-level retained
  draw commands (host Skia executes on GPU, host owns fonts/text, tiny WIT
  traffic, host keeps semantic control — insets/theming). wasi-gfx/webgpu =
  LOW-level: maximal toolkit freedom in the guest (Slint-GPU/egui/bevy/games)
  but each guest ships its renderer + fonts; host loses semantics; per-app GPU
  contention/power; bigger guests.
- **wandr fit if ever wanted:** the Phase-4 `sf_media` child surfaces are
  EXACTLY the host primitive for a wasi-gfx `surface` impl (child
  SurfaceControl + BBQ producer per guest surface; `frame-buffer` = CPU buffer
  upload into it; `webgpu` = host wgpu device → ANativeWindow; input = the
  existing SfInputEvent per-host routing). A SECOND guest graphics path
  alongside skiko-gfx, not a replacement. Adreno 540: wgpu via Vulkan/GLES
  works-ish (aging driver).
- Verdict: standards-track answer for "arbitrary guest renderers/games" later;
  skiko-gfx remains the right default for app UIs. Don't adopt while Phase 2 /
  unreleased — track it.
- **Full comparison + standardization analysis (2026-06-11):
  `docs/skiko-gfx-vs-wasi-gfx.md`** — two LAYERS of one stack (GPU-driver vs
  2D-canvas, the WebGPU-vs-Canvas2D split); they compose both ways
  (sf_media hosts wasi:surface; a wasi-canvas can sit on wasi:webgpu).
  "wasi-canvas" from our contract = real gap + only shipped contender
  (3 languages of consumers); needed first: de-Skia naming, resources not
  u32 ids, shed wandr-isms, fix indexed-getter warts. Path: publish WIT +
  skia-wit-mapping as versioned de-facto spec when stable; formal WASI
  phase-0 only after — positioned as wasi-gfx's 2D companion.
- **DRAFT WRITTEN 2026-06-11: `proposals/wasi-canvas/`** (wasm-tools-valid
  wasi:canvas@0.0.1: canvas/picture/shader/image resources, paint record w/
  borrow<shader> + mask-blur, SVG paths + fill-rule, per-corner rrects,
  glyphs + optional layout text layers; drawing-only scope = the red line).
  Gate analysis in COMPATIBILITY.md: NO hard breaks (coexistence — second
  add_to_linker over the same SkiaRenderer; my:skiko-gfx untouched).

**RECHECK 2026-06-12 (source-grounded, docs/surface-convergence-proposal.md
§Upstream recheck):** upstream MOVED — surface/graphics-context/
frame-buffer live in `wasi-gfx/wasi-gfx-runtime` wit/deps (WebAssembly/
wasi-webgpu keeps only webgpu.wit). Their shapes are pre-stable (ambient
context constructor — violates their own capability rule; present =
"TODO maybe remove"; empty frame-event; pointer events = {x,y} ONLY — no
multi-touch/buttons/pressure). Verdicts: canvas-context = same idiom,
deliberately FUSED (canvas instead of abstract-buffer indirection) —
third-context alignment deferred until upstream stabilizes; our
input-handlers 0.0.2 records are STRICTLY richer = the credible shared
event vocabulary; video claim holds at INFRA level only (sf_media child
surface = the wasi:surface primitive) — wandr:video fuses placement
verbs (set-rect/visible/rotation) into the decoder; factoring lane
(decoder as a fourth graphics-context consumer) recorded for if
wasi:surface ever lands on wandr.
