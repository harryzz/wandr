---
name: reference_swift_openswiftui_wandr
description: "Swift + OpenSwiftUI on wandr — researched 2026-06-13, NOT practical yet (gated on upstream, Qt/Flutter-class): the RenderBox display-list shape would fit wasi:canvas, but Swift wasip2/WIT-gen is immature (no Swift wit-bindgen → C-interop or hand-roll) AND OpenSwiftUI cross-platform is early (no text, AttributeGraph incomplete, no WASI); memo = docs/swift-openswiftui-wandr-feasibility.md"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 00de4e50-ab0d-4032-8361-7d93d41cf043
---

**Swift + OpenSwiftUI on wandr — web research 2026-06-13, no spike. Full
memo: `docs/swift-openswiftui-wandr-feasibility.md`.**

Verdict: **wait** — gated on upstream (Qt/Flutter class), NOT "almost
possible" (corrected after digging forums/issues 2026-06-13). The
architecture shape is right (pluggable renderer + wasip1+adapter route both
real), but BOTH gating pieces are absent today: (a) toolchain — NO public
Swift custom-WIT precedent (no wit-bindgen-swift, no componentize-swift, no
POC exporting a WIT iface; WasmKit's component work = host runtime, not
guest), so the wit-bindgen-c+C-interop path is plausible-but-UNPROVEN
(skiko-tier unknowns); (b) framework — OpenSwiftUI's Skia renderer is
DIAGRAM-ONLY (issue tracker has zero WASM/WASI/Skia/renderer-roadmap items;
shipped off-Apple = GTK4, no text, no WASI). Closer to [[reference_qt_wandr]]
/ [[reference_flutter_go_ui_wandr]] than [[reference_avalonia_wandr]].

- **Shape fits (confirmed by Package.swift + arch diagram):** SwiftUI =
  view tree → AttributeGraph (reactive) → RenderBox (C++) emits a **display
  list** → CoreAnimation/Metal + CoreText. Maps to `wasi:canvas` like
  Slint's ItemRenderer / Avalonia's IDrawingContextImpl. OpenSwiftUI HAS a
  **pluggable renderer abstraction** — `main` Package.swift selects per
  platform: `renderBoxCondition` (RenderBox), `renderGTKCondition` (GTK4/
  Cairo = current Linux path, windowing-bound, NOT wandr-usable),
  `swiftUIRenderCondition` (Darwin). The arch diagram
  (Screenshots/Architecture/arch.png) ALSO shows a **Skia** renderer — but
  NO Skia target in main yet (roadmap; SkiaKit bindings exist —
  migueldeicaza/UnGast). A Skia backend is the green flag: wasi:canvas is
  skia-shaped (skiko-grown) so Skia draw ops map ~1:1 → WandrRenderer = thin
  reskin. So pluggable-renderer bar = CLEARED; what's missing is a non-Apple
  non-GTK rasterizing backend (the Skia one) + text.
- **Toolchain = NOT the main blocker (corrected):** Swift emits wasip1
  (Swift SDK for WASM 6.2+), and that's enough — component production is the
  wasip1 module + **`wasm-tools component new --adapt
  wasi_snapshot_preview1.wasm`** route that BOTH Compose (Kotlin, wandr-fork
  adapter) and Avalonia (C#, componentize-dotnet's bundled adapter) already
  use; neither emits native wasip2. Swift (linear-memory like C#) would use
  the STOCK adapter, not the fork. WasmKit's wasip2 work = host-runtime,
  irrelevant to guest production. The ONLY real gap is the **custom WIT
  bindings** (no Swift wit-bindgen generator; ships Rust/C/C++/C#/Go) →
  `wit-bindgen c` + Swift C-interop (practical) or hand-rolled ABI
  (skiko-tier, [[feedback_wit_bindgen_no_kotlin_generator]]) — bounded, same
  class Kotlin pays. Footprint: full Swift runtime in wasm = tens-of-MB
  class; Embedded Swift shrinks it but drops reflection/existentials SwiftUI
  needs.
- **Framework blocker:** OpenSwiftUI ~2yr, active but early off-Apple:
  Ubuntu partial (no deploy), Android/Windows unsupported, **no WASI**,
  **text not supported yet**, OpenAttributeGraph "only API-compatible" (most
  features Apple-only). Hasn't reached the "pluggable renderer exists" bar
  that made Slint/Avalonia cheap.
- **Recommendation:** don't spike now; track (1) OpenRenderBox non-Apple
  display-list + text, (2) Swift wasip2 component guests. Spike-gate if both
  move: bare Swift wasip2 component exporting a trivial WIT via C-interop +
  one draw-rect (toolchain unknown), then OpenRenderBox→wasi:canvas.
  Cross-ref `docs/wasm-component-language-support.md` (Swift not first-class).
