# TeaVM 0.16.0-SNAPSHOT patches — JS-free WasmGC (task 113 spike, 2026-06-16)

Applied to a `konsoletyper/teavm` clone, these remove the JavaScript dependency
from TeaVM's WasmGC output so a pure-Java guest compiles to a module importing
only the TeaVM floor (`env::memory`, `teavmDate`, `teavmMemory`) — **no
`teavmJso`, no `wasm:js-string`** — that instantiates under wasmtime and whose
`@Export`ed functions are callable from a non-JS host.

- **01-throwable-dejso.patch** — `TThrowable`: the WasmGC stack-trace path used
  `JSString`/`LazyStackSupplier extends JSObject`. Replaced with an empty trace
  (test/unconditional). This is what pulled `JSString` reachably (via every
  exception). *Clean version: gate on a WASI sub-mode, keep a non-JS stack capture.*
- **02-jsoplugin-gate.patch** — `JSOPlugin`: gate `WasmGCJso.install(...)` behind
  `-Dteavm.wasmgc.nojso=true`, removing the JSO codegen + the module initializer
  (`WasmGCJsoCommonGenerator.stringToJs` → js-string) for non-JS builds.

## Reproduce
1. clone teavm @ 0.16.0-SNAPSHOT, `git apply` both patches.
2. `./gradlew :classlib:publishToMavenLocal :jso:impl:publishToMavenLocal -x test`
3. in `../` (the spike): `mvn -Dteavm.wasmgc.nojso=true package` → `target/wasm/spike.wasm`
4. `cd stub-host && cargo run` → host calls `packed_len(n)`, prints correct results.

## Still TODO (task 113 M1/M4/M5)
- Package as a proper `WEBASSEMBLY_GC_WASI` target/mode (gate the Throwable patch
  too; don't affect browser WasmGC).
- Floor → WASI: `teavmDate.currentTimeMillis`→`clock_time_get`,
  `teavmConsole.putcharStdout`→`fd_write` (restore stdout), `teavmMemory` heap
  self-managed; then no host stubs needed.
- P1→P2 adapter → component + a real custom WIT export (vs the raw core-func export).
