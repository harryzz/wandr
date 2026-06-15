---
name: reference_ruby_wandr
description: Ruby on wandr feasibility — wasip2 component build + custom WIT both exist (js gem); blocked on UI layer
metadata: 
  node_type: memory
  type: reference
  originSessionId: da762aa1-b790-4e3d-9728-2a3c5d37bf19
---

Researched 2026-06-15 (web + ruby.wasm repo source). Can a Ruby guest render on
wandr via `wasi:canvas` like Slint/Avalonia/dioxus? Memo =
`docs/ruby-wandr-feasibility.md`.

**Verdict: viable-but-DIY, blocked on the UI layer — NOT the toolchain** (softer
than Swift/Qt). The Component-Model + custom-WIT plumbing Ruby needs is REAL and
demonstrated in-tree, not unproven.

CORRECTION: earlier I said "ruby.wasm = wasip1 only / no component model" by
trusting the docs page — WRONG. Repo shows a shipped wasip2 build. Read source,
not rendered docs ([[feedback_read_source_first]]).

What the source shows:
- ruby.wasm SHIPS `wasm32-unknown-wasip2` (`builders/.../Dockerfile` = env only;
  `Rakefile` `enable_component_model: true`; npm `ruby-head-wasm-wasip2`). CRuby
  C build is shared for both previews (`crossruby.rb` `/^wasm32-unknown-wasi/`).
- Base output = a `wasi:cli` COMMAND component (run script/REPL); `wasi-preset-args`
  bakes argv at the wasip1 MODULE level (not the component).
- THE FIND: the `js` gem (`packages/gems/js/`) is a hand-rolled custom-WIT bridge
  — `wit/world.wit` (`world ext { import js-runtime; export ruby-runtime; }`),
  implemented via `ext/js/witapi-core.c` + wit-bindgen-c `bindgen/ext.c` +
  embedded `ext_component_type.o`. A C extension lifting/lowering the canonical
  ABI against the Ruby C-API. PROVES custom WIT export/import from Ruby works.

Two real gaps: (1) no generalized `componentize-ruby` (js gem is hand-written,
JS-specific, and shaped "host drives VM" — inverse of a reactive guest export);
(2) NO Skia-backed Ruby UI framework at all (no Compose/Slint/Avalonia analog).

To make Ruby a wandr guest: write a "wandr bridge" C-extension gem modeled on the
js gem (export on-frame/on-pointer → rb_funcall; import wasi:canvas/audio →
native methods), settle Asyncify-vs-reactor ([[project_wandr_step_executor]]
problem in CRuby), and hand-build the entire UI over wasi:canvas. Effort ≈
componentize-py but with an in-repo template. Best off-the-shelf use today =
the command-component form (run Ruby scripts). Revisit if a generic
componentize-ruby or any Skia Ruby UI lands. Compare [[reference_swift_openswiftui_wandr]],
[[reference_qt_wandr]], [[reference_flutter_go_ui_wandr]], [[reference_slint_wasip2]].
