---
name: reference_dart_wasm_component_status
description: "Dart→WASM component viability for wandr. As of 2026-06 a real prototype path exists: simolus3/wasm.dart `wasm_tools` (witgen + dart2wasm-wrapper → components) on Dart 3.13, targeting wasmtime. WasmGC (needs GC host — we have it). Not tried on wandr yet; confirm reactor (not CLI/main) export."
metadata:
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-07-28T19:08:12.149Z
---

Checked 2026-07-28 (upstream re-survey, alongside Swift + Kotlin). **Dart went
from "JS-host only, no component story" to a working prototype component path** —
worth a future guest spike, not attempted on wandr yet.

**The path that now exists:**
- **Dart 3.13** will experimentally target **non-JS WASM embedders** (wasmtime/
  wasmer). The Dart team's stance ([dart-lang/sdk#56366](https://github.com/dart-lang/sdk/issues/56366),
  open, updated 2026-06-21; broader [#53884](https://github.com/dart-lang/sdk/issues/53884)):
  Component-Model support can live as a **third-party tool** on top of the
  non-JS `dart2wasm` target rather than in the SDK.
- **simolus3 built exactly that:** `wasm_tools` package —
  `github.com/simolus3/wasm.dart/tree/main/pkg/wasm_tools` (first version
  published 2026-06-21). *"can be used to compile Dart to WebAssembly
  components."* Flow: obtain a `.wit` → `dart run wasm_tools witgen -i
  definitions.wit` → implement the generated export interfaces → `dart run
  wasm_tools compile` (a `dart2wasm` wrapper that emits a **component**).
  Requires **Dart 3.13**; targets **wasmtime**.

**Fit with wandr (host↔guest = WIT + Component Model):** this is the FIRST
plausible Dart guest — WIT-driven, wasmtime-targeted, exactly our host's shape.
Before betting on it, confirm two things:
1. **WasmGC host.** Dart compiles via WasmGC (like Kotlin/Wasm), so it needs our
   GC-enabled host. We already set `config.wasm_gc(true)` /
   `wasm_function_references(true)` / `wasm_exceptions(true)`
   (`runtime/wandr-host/src/lib.rs`), and wasmtime 47 makes GC+exceptions default
   (see [[reference-wasmtime-version-status]] if written). So the host side is ready.
2. **Reactor vs CLI.** wandr guests are **reactors** (cdylib, no `main`,
   host-driven via WIT imports). Verify `wasm_tools` can emit a reactor-style
   component (implement imported interfaces + exported handlers), not just a
   CLI/`wasi:cli/run` main export. This is the same caveat that gates the native
   Kotlin P2 path (KT-87801) — see [[reference-kotlin-wasm-component-model-status]].

**Status: NOT tried on wandr.** Third-party + experimental + Dart-3.13-gated.
Re-check when `wasm_tools` stabilizes / Dart 3.13 ships stable, then a spike:
Dart 3.13 + wasm_tools → WIT bindings against our `wandr:*` contracts →
component → host. Would add a Dart / Flutter-adjacent guest lane.

Related: [[reference-kotlin-wasm-component-model-status]] (same WasmGC + reactor
questions), [[reference_swift_wasm_wasi_status]], [[reference_flutter_go_ui_wandr]],
`docs/wasm-component-language-support.md`.
