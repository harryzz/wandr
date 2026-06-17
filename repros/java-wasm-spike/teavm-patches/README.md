# TeaVM 0.16.0-SNAPSHOT patches — JS-free WasmGC → WASI component (task 113)

5 patches on a `konsoletyper/teavm` clone that make TeaVM's WasmGC backend emit a
**fully self-contained, JS-free module** (zero imports) for pure-Java guests, which
`wasm-tools` turns into a **WASM component with a WIT interface** that a non-JS host
(wasmtime) calls directly. All gated on **`-Dteavm.wasmgc.nojso=true`** (browser
WasmGC unaffected); the Throwable + Heap ones are unconditional-but-safe.

| # | file | change |
|---|---|---|
| 01 | `classlib/.../java/lang/TThrowable.java` | WasmGC stack-trace was `JSString`/`LazyStackSupplier extends JSObject` (reachable from every exception). → empty trace. |
| 02 | `jso/impl/.../JSOPlugin.java` | gate `WasmGCJso.install(...)` behind `teavm.wasmgc.nojso` → no JSO codegen / `stringToJs` initializer / `wasm:js-string`. |
| 03 | `core/.../backend/wasm/WasmGCTarget.java` | gate: `env::memory` import → module-defined memory; `teavmMemory.heapOffset`/`maxSize` imported globals → internal const globals. |
| 04 | `core/.../backend/wasm/intrinsics/SystemIntrinsic.java` | gate: `teavmDate.currentTimeMillis` import → a local fn returning `f64 0` (placeholder; real `wasi clock_time_get` later). |
| 05 | `core/.../runtime/heap/Heap.java` | `teavmMemory.notifyHeapResized` import → no-op (JS loader's default was a no-op too). |

## Result (verified 2026-06-17)
Pure-Java spike (`../src/.../Spike.java`, `@Export(name="packed-len")`):
- imports = **0**, size ~11 KB (was ~36 KB JS-coupled).
- `wasm-tools component embed wit + component new` → a **valid component**:
  `world root { export packed-len: func(septets: s32) -> s32; }`
- `../stub-host` (`comp-host` bin) instantiates the **component** via wasmtime's
  component model and calls `packed-len` over the WIT → correct results
  (1,3,5,7,9). **Full Java→JS-free-WasmGC→component→WIT→host-call chain works.**

## Reproduce
1. clone teavm @ 0.16.0-SNAPSHOT; `git apply` all 5 patches.
2. `./gradlew publishToMavenLocal -x test` (or `:core:`, `:classlib:`, `:jso:impl:`).
3. `mvn -Dteavm.wasmgc.nojso=true package` (in `../`) → `target/wasm/spike.wasm`.
4. `wasm-tools component embed wit target/wasm/spike.wasm -o e.wasm && wasm-tools component new e.wasm -o c.wasm`
5. `cd stub-host && cargo run --bin comp-host -- ../target/wasm/spike.component.wasm`

## Still TODO (productionization, not blockers)
- Real WASI floor when guests need it: `currentTimeMillis`→`clock_time_get`,
  restore stdout `putcharStdout`→`fd_write` (dropped with the crude JSO gate),
  proper self-managed heap growth (the const heapOffset is fine for non-allocating
  guests; real allocation needs the C-backend heap-init logic).
- Package as a first-class `WEBASSEMBLY_GC_WASI` `TeaVMTargetType` (vs the
  `-D` flag) and **upstream to TeaVM** (fills a real gap: Java→WasmGC+WASI).
- Allocating guests + the wandr P1→P2 adapter path for guests that DO import wasi.
