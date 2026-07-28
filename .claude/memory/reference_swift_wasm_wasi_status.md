---
name: reference_swift_wasm_wasi_status
description: "Swift/Wasm WASI toolchain status (distinct from our OpenSwiftUI app work). As of 2026-07 Swift is still WASI Preview 1 only; WASI 0.2 / Component Model = future work in the official vision. WIT support is experimental via WasmKit `wit-tool` (not the compiler). No native-component escape from the P1 adapter — our Swift guests stay on wasm32-wasip1 + adapter."
metadata:
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-07-28T19:08:29.335Z
---

Checked 2026-07-28 (upstream re-survey). This tracks the **Swift toolchain's
WASI/Component-Model status**, separate from our app-level Swift/OpenSwiftUI
port ([[reference_swift_openswiftui_wandr]]).

**Conclusion: no toolchain-level change for wandr — Swift stays exactly where our
guests already run** (`wasm32-wasip1` through the same P1 reactor adapter as
Kotlin). Swift gives no native wasip2/component shortcut yet.

From Swift's official vision (`github.com/swiftlang/swift-evolution/blob/main/visions/webassembly.md`):
- **WASI Preview 1 only.** "all patches necessary for basic Wasm and WASI Preview
  1 support have been merged" (since Mar 2024). **WASI 0.2 / Preview 2 and full
  Component Model are explicitly FUTURE WORK**: "Continue work on Wasm Component
  Model support in Swift as the Component Model proposal is stabilized. Ensure
  that future versions of WASI are available to Swift developers."
- **WIT support is experimental and NOT in the compiler** — it lives in
  **WasmKit's `wit-tool`** subcommand: generate `.wit` from Swift decls and Swift
  bindings from `.wit`. Native single-step component production is aspirational.
  **`wit-tool` is the thing to WATCH** (seed of a real Swift→WIT path).
- SwiftWasm ships frequent DEVELOPMENT-SNAPSHOTs (latest 2026-07-11); last tagged
  release still **6.1** (Apr 2025). The WASI SDK is upstreamed into
  swiftlang/swift.

**For wandr:** our Swift blockers are our OWN, not toolchain-WASI gaps — the
live blocker is the device `pow` SIGILL in aarch64-AOT codegen during animations
(see [[reference_swift_openswiftui_wandr]]), plus the swift-foundation WASI
FileManager bug ([[reference_swift_foundation_wasi_filemanager_bug]]). Re-check
`wit-tool` + the vision's Component-Model line before any Swift guest-pipeline
rework; nothing actionable until Swift emits components natively.

Related: [[reference_swift_openswiftui_wandr]],
[[reference_swift_foundation_wasi_filemanager_bug]],
[[reference-kotlin-wasm-component-model-status]] (parallel P1-adapter situation),
[[reference_dart_wasm_component_status]].
