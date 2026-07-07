# wit-bindgen async codegen probe

Proves the wandr toolchain can do **native Component-Model async** end-to-end —
the M1 gate for `tasks/115-signal-transport-wasip3-async.md` (retire
`wandr-step-executor`). Verified 2026-07-07.

## Toolchain (wandr's pinned versions)

- `wit-bindgen` 0.53.1 (crate + CLI) — `async` is a **default** feature
- `rustc` 1.95.0, target `wasm32-wasip2`
- `wasmtime` 46.0.0, `wasm-tools` 1.245.1

## What it proves (4 steps, all ✅)

1. **Codegen** — `wit-bindgen rust --async all` on a WIT with `async func` +
   `future<T>` + `stream<T>` emits `pub async fn` (imports and exports) +
   `FutureReader/Writer` + `StreamReader/Writer`. (See "regenerate" below.)
2. **Compile + link** — this crate (`src/lib.rs`, an `async fn run` export) builds
   for `wasm32-wasip2` and **links `wit_bindgen::rt::async_support`**.
3. **Component** — the build is a valid component.
4. **Run** — wasmtime drives the async export to completion, correct result.

## Reproduce

```bash
# build + run the async export (expect: 42)
cargo build --release --target wasm32-wasip2
wasmtime run -W component-model-async=y \
  --invoke 'run(41)' \
  target/wasm32-wasip2/release/wit_bindgen_async_probe.wasm      # -> 42

# inspect future<T>/stream<T> codegen from a fuller WIT:
cat > /tmp/x.wit <<'WIT'
package t:x;
interface i { c: async func()->u32; h: func()->future<bool>; r: func()->stream<u8>; }
world w { export i; }
WIT
wit-bindgen rust --async all --out-dir /tmp/xout /tmp/x.wit
grep -E 'async fn|FutureReader|StreamReader' /tmp/xout/*.rs
```

## Note

Async needed only `-W component-model-async=y`, **not** `-Sp3` — the async
export / future / stream ABI is orthogonal to importing WASI 0.3 interfaces.
The p3 `wasi:tls`/`wasi:sockets` wiring is the *next* thing M1 proves (this probe
covers only the toolchain/async-ABI gate).
