---
name: reference_flutter_go_ui_wandr
description: "Flutter on wandr = blocked on toolchain TODAY (dart2wasm JS-env-only) but GATE CLOSING (deep dive 2026-06-13): standalone/non-JS is ACTIVE upstream (dart-lang/sdk#53884, in-tree standalone_platform.dill, narrow blockers, Dart team soliciting a Rust/wasmtime integration); architecture fits BEST (web_ui dart:ui→CanvasKit binding seam retargets to wasi:canvas, host-shaped Paragraph = our paragraph iface); Go = NO skia-backed UI exists; memo = docs/flutter-wandr-feasibility.md"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**Flutter/Dart + Go UI — analyzed 2026-06-11 (survey complete: Slint
shipped / Avalonia feasible / Qt no / Flutter gated / Go n/a). Full memo:
`docs/flutter-wandr-feasibility.md`.**

- **Flutter: best architecture, no toolchain.** Flutter Web's `dart:ui` is
  implemented IN DART (engine/lib/web_ui) forwarding canvas calls to
  CanvasKit — Google ships "the skiko trick" already; retarget = JS-interop
  → my:skiko-gfx imports. Text is HOST-shaped (Paragraph/SkParagraph) =
  wandr's existing `paragraph` interface, no glyph verbs needed. dart2wasm
  emits WasmGC (our wasmtime already enables gc for Kotlin) BUT only runs
  in JS environments — core libs import JS builtins; "doesn't support
  wasmtime" per Dart docs. **UPDATE 2026-06-12: dart:ui → wasi:canvas 0.0.2 mapping DONE**
(flutter memo §dart:ui): SceneBuilder independently re-derives the
`scene` interface; the check drove three R1 breaking-class amendments
into REDESIGN-0.0.2 (29-mode blend enum union — the Compose binding was
silently degrading 10 modes to src-over!, text shadows = list, gradient
local transforms). Contract is Flutter-ready; only the toolchain gate
remains. Native-engine-embedder shape rejected (fat C++ runtime beside
wandr, bypasses sandbox + skiko-gfx). If the gate flips, Flutter jumps the
port queue.
- **DEEP DIVE 2026-06-13 (corrects "not our fight / nonexistent"): the gate
  is ACTIVELY CLOSING.** The live issue is **dart-lang/sdk#53884** (support
  non-JS wasm runtimes), NOT the dormant #56366 (component model). Findings:
  (1) JS dependency is an ENUMERATED tractable list (event loop, printing,
  timers, weak maps/finalizers, stack traces, double→String, regex, math,
  strings) — Dart team plan = impl-in-Dart / link-wasm / import-from-host;
  (2) in-tree **dart2wasm_standalone_platform.dill** + stringref variant
  already exist (standalone platform being built) but NOT shipped in
  releases, NO public wasmtime proof-of-life yet; (3) narrow named blockers:
  dart2wasm emits LEGACY try/catch EH that wasmtime rejects → must move to
  exnref EH (now in all browsers); ship the .dill; (4) strings via
  wasm:js-string builtins OR stringref/in-wasm variant; (5) @simolus3:
  achievable, only stack-traces+weak-maps degrade standalone; (6) **Dart
  team soliciting a RUST integration crate = wandr-shaped host.** Component
  model not strictly needed — once standalone runs in wasmtime, wandr path =
  Kotlin pipeline (standalone module + hand-rolled WIT bindings + `component
  new --adapt`). wandr's wasmtime already does WasmGC+exnref+func-refs (the
  Kotlin AOT flags) so it's PRE-POSITIONED to load dart2wasm standalone
  output the moment it emits exnref. Verdict shifts: "wait + watch closely,"
  not "don't." Status TODAY still = not shipped (docs say browser-only,
  updated Dec 2025). Rendering nuance: both web renderers bundle Skia-IN-wasm
  (CanvasKit/skwasm) → retarget the dart:ui→CanvasKit BINDING seam, not a
  remote-skia path.
- **Go: no skia-backed UI library exists** (the user looked and was right):
  go-skia = dead cgo experiment; unison = real Skia-via-cgo toolkit but
  desktop-only and cgo can't target wasm; Gio/Fyne/Cogent grew bespoke
  renderers (Go culture avoids cgo → no skia ecosystem). If a Go guest is
  ever wanted: study **Gio** (pure Go, browser-wasm proven,
  go-text/typesetting = pure-Go shaper → the task-100 draw-glyphs model
  like [[reference_slint_wasip2]]); friction = integrated GPU renderer +
  big-Go=wasip1+adapter / TinyGo-unsupported. No demand → parked.
- Full six-way lineup table in the memo (Compose/dioxus/Slint/Avalonia/
  Qt/Flutter).
