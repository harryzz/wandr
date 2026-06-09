---
name: kotlin-wasm-println-throws-under-wasmtime
description: "Kotlin/Wasm + wasmtime CLI with command-adapter — println() or any meaningful main() body throws \"thrown Wasm exception\"; empty main() works"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d9451151-9116-4c95-a45d-8758673104ce
---

Under `wasmtime run --wasm gc --wasm function-references --wasm exceptions --wasm component-model <composed.wasm>` against a Kotlin/Wasm 2.4.0-RC component wrapped with the command-variant preview1 adapter (`wasi_snapshot_preview1.command.wasm`):

- Empty `fun main() {}` — runs + exits clean (exit code 0).
- `fun main() { println("hi") }` — `Error: thrown Wasm exception`.
- Same throw for any `@WasmImport` call, even when the call is inside a `try { … } catch (t: Throwable) { … }` in main() (the throw happens BEFORE main() runs — likely during module init / Kotlin static initializers).

The composed component STRUCTURE validates fine — `wasm-tools component wit` shows the correct import/export shape; the dep wiring + canonical-ABI signatures are correct. The bug is at runtime — possibly Kotlin's standard-library init throws an exception that gets lowered to a wasm `throw` instruction, OR the command adapter has an interaction issue with Kotlin's preview1 `fd_write` imports.

**Why:** task 36 step 6 hit this on the dev box. Composed `md-smoke-component.wasm` + `markdown_renderer.wasm` via `wac plug`; runtime trips on any non-trivial main(). Blocks dev-box validation of the cross-app dep wiring round-trip.

**On-device update (2026-05-26, task 36 step 7):** Confirmed the throw is module-init level and unconditional, NOT specific to `wasmtime run`. Reproduced via `wandr-host --run-once` on Pixel 2 XL with wandr-host's `wasi_stderr` routing fd 2 to logcat — same `thrown Wasm exception` from `call_run`. Even empty `fun main() {}` (built + installed + invoked via the full install pipeline) throws identically. So:
- Bug is NOT about println.
- Bug is NOT about stderr destination.
- Bug fires before any guest user code runs (likely Kotlin's static init / `__wasm_call_ctors` chain).
- wandr-host's wasi_stderr does NOT help — don't expect it to.

**How to apply:**

- Don't assume `println()` in Kotlin/Wasm prints under `wasmtime run` or any wasi:cli host — bisect with `fun main() {}` first to confirm the build is structurally fine before debugging output paths.
- For observable output in Kotlin/Wasm guests, use the wandr-app pattern: `@WasmImport` a host-provided log function (skiko has `WitCanvas.Import.logMessage`); don't rely on the WASI stdio path.
- The throw happens at module init, so reordering main() body or adding try/catch around the call site doesn't surface a Kotlin Throwable — diagnose by isolating which Kotlin static initializer / `__wasm_call_ctors` step throws (wasmtime debug APIs / wasm-tools dump of the start function).
- For task-validation use cases that just need a one-shot `wasi:cli/command` consumer, sidestep the whole bug by writing the consumer in Rust on `wasm32-wasip2` — see [[rust-component-as-cli-smoke]].
- The Kotlin smoke still validates EVERYTHING up to `Command::instantiate` (linker correctness, dep wiring, deserialize, instantiate succeed) — the throw is at `call_run` time. So it's useful as a "instantiate-only" validator even with the bug present.

Related: [[wit-bindgen-no-kotlin-generator]] (forced the hand-written bindings here), [[canonical-abi-import-export-asymmetry]] (the @WasmImport shape this debugged), [[task-36-step-7-pending]] (deferred step 7's primary blocker).
