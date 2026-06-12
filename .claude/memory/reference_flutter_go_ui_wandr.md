---
name: reference_flutter_go_ui_wandr
description: "Flutter on wandr = blocked on toolchain (dart2wasm is JS-env-only WasmGC; WASI = open proposal dart-lang/sdk#56366) though architecture fits BEST (web_ui = dart:ui-over-remote-skia precedent, host-shaped Paragraph = our paragraph iface); Go = NO skia-backed UI exists (unison/cgo is desktop-only; Gio is the non-skia study target); memo = docs/flutter-wandr-feasibility.md"
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
remains. **Gate = dart-lang/sdk#56366** (`-t wasi`,
  open/unimplemented). Native-engine-embedder shape rejected (fat C++
  runtime beside wandr, bypasses sandbox + skiko-gfx). If the gate flips,
  Flutter jumps the port queue.
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
