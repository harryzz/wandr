---
name: rust-component-as-cli-smoke
description: When you need a one-shot wasi:cli/command WASM consumer to validate host plumbing, write the consumer in Rust on wasm32-wasip2 — Kotlin/Wasm has a pre-existing module-init throw with the command adapter
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d9451151-9116-4c95-a45d-8758673104ce
---

When you need a one-shot `wasi:cli/command`-shaped guest to smoke-test host-side WASM plumbing (cross-app dep wiring, `Command::instantiate` paths, anything that invokes `wasi_cli_run().call_run(store)`), write the consumer as a Rust binary on `wasm32-wasip2`, not Kotlin/Wasm.

**Why:** Kotlin/Wasm 2.4.0-RC + the WASI command adapter throws "thrown Wasm exception" at module init unconditionally. Confirmed on-device in task 36 step 7 (2026-05-26): even `fun main() {}` (empty body) throws under `wart-host --run-once`. wart-host's `wasi_stderr` routing to logcat doesn't make a difference vs `wasmtime run` — the throw isn't stderr-related, it's at module init before `main()` runs. See [[kotlin-wasm-println-throws-under-wasmtime]].

Rust on `wasm32-wasip2`:

- `fn main()` gives implicit wasi:cli/command shape (exports `wasi:cli/run.run`).
- `wit_bindgen::generate!({ world: "...", path: "wit", generate_all })` brings in the import side of any WIT.
- Build with `cargo build --target wasm32-wasip2 --release` — output is already a component, no `wasm-tools component embed`/`new` needed.
- `wasmtime compile --target aarch64-linux-android --wasm component-model --wasm gc --wasm function-references --wasm exceptions` for the device cwasm.

The reference implementation lives at `md-smoke-rust/` in the wart repo — copy that crate as the starting template.

**Concrete shape:**

```
md-smoke-rust/
  Cargo.toml          # [[bin]] name=md_smoke_rust + wit-bindgen = "0.46"
  src/main.rs         # wit_bindgen::generate! + fn main() { call_dep(); exit(N) }
  wit/
    smoke.wit         # `world consumer { import some:thing/iface@x.y.z; }`
    deps/
      thing/iface.wit # symlink or copy from ../wit/iface.wit
```

**How to apply:**

- For any future cross-app-dep validation, Rust testing of a new host CLI mode, or component-model smoke that just needs to "call into the host and exit cleanly" — start from `md-smoke-rust/`'s Cargo.toml + main.rs + wit/ layout.
- Don't lose time bisecting Kotlin/Wasm command-adapter throws when the validation goal is *host plumbing*; Kotlin's bug is module-init level and orthogonal. Use Rust to bypass; defer the Kotlin throw to its own investigation if/when a Kotlin CLI consumer actually matters.
- The Kotlin smoke still has value as a "does the install/load/linker layer reach `Command::instantiate` cleanly" test, since the throw happens AFTER `Command::instantiate` returns. Don't delete the Kotlin smoke; keep both.

**Related:** [[kotlin-wasm-println-throws-under-wasmtime]], [[task-36-step-7-pending]], [[wit-bindgen-no-kotlin-generator]] (explains why the Kotlin side needs hand-written bindings in the first place).
