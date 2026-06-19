---
name: reference_swift_openswiftui_wandr
description: "Swift + OpenSwiftUI on wandr — UPGRADED from 'wait' to ACTIVE SPIKE WORKING (2026-06-19): OpenSwiftUICore+OpenSwiftUI COMPILE for wasm32-wasip1 and RENDER a DisplayList incl @State + multi-child VStack (2 fills); AttributeGraph runs on wasm. Live state = repros/openswiftui-wasm/RESUME.md (NOT this file). Phase 4 next = DisplayList -> Option-B CGContext drawer -> wasi:canvas -> device. memo = docs/swift-openswiftui-wandr-feasibility.md"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 00de4e50-ab0d-4032-8361-7d93d41cf043
---

## ⬆️ STATUS UPGRADE 2026-06-19 — the "wait" verdict below is SUPERSEDED by a working spike
The spike happened and got through **phase 3**. Live, authoritative state =
**`repros/openswiftui-wasm/RESUME.md`** (read it, not this memo). What's proven on
wasm32-wasip1 now: OpenSwiftUICore + OpenSwiftUI **compile** (0 errors); Apple's
AttributeGraph (Compute fork) runs reactively; primitive + custom Views, **@State**, Text
construction, single-type protocol conformance, and **multi-child `VStack`/`TupleView`** all
**render a DisplayList** (`VStack{Color.red;Color.blue}` → 2 fills, exit 0, deterministic).
Target app = `eleev/swiftui-2048`. The whole grind has been a **bounded set of swiftcc
closure-ABI walls**: on wasm, Swift closures with args/return passed to C `AG_SWIFT_CC(swift)`
fn-ptr params mislower → `signature_mismatch` trap; fix = per-function plain-C `*C` variant
(C++ `extern "C"` + header decl `#if defined(__wasi__)` + Swift `#if arch(wasm32)` routing
with a `@convention(c)` thunk + boxed/by-pointer context). LESSON (cost ~3 sessions): the
multi-child wall LOOKED like "memory corruption that moves with any code change" under
print-probing — it was actually one such swiftcc wall (`AGGraphReadCachedAttribute`, hit by the
layout engine). **Don't print-probe a moving wasm crash; use `wasmtime run -D debug-info=y -D
coredump=… -D max-backtrace=N` for a DWARF backtrace with NO guest source change** (frame 0
named the symbol directly). `wmemcheck` needs a wasmtime built with that feature (stock binary
lacks it). NEXT = PHASE 4 (DisplayList → Option-B CGContext drawer → wasi:canvas → Pixel 2 XL).

---
**(historical) Swift + OpenSwiftUI on wandr — web research 2026-06-13, pre-spike. Full
memo: `docs/swift-openswiftui-wandr-feasibility.md`.**

Verdict (SUPERSEDED — see upgrade above): **wait** — gated on upstream (Qt/Flutter class),
NOT "almost possible" (corrected after digging forums/issues 2026-06-13). The
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
