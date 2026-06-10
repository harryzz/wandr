---
name: reference_wasi_webgpu_gfx
description: wasi:webgpu (Phase 2) + wasi-gfx (surface/frame-buffer) — the guest-owns-the-renderer model vs wandr's skiko-gfx command stream; our sf_media child surfaces are the natural host primitive if we ever implement it
metadata:
  type: reference
---

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
