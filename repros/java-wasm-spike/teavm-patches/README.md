# TeaVM `WEBASSEMBLY_GC_WASI` target — JS-free WasmGC for non-JS/WASI hosts

`WEBASSEMBLY_GC_WASI.patch` — productionized patch on `konsoletyper/teavm`
(0.16.0-SNAPSHOT) adding a first-class WasmGC target/mode that emits a
**self-contained, JS-free** module (zero imports) for non-JS (WASI / component-
model) hosts, which `wasm-tools` turns into a **component with a WIT interface**
callable from wasmtime. **Browser `WEBASSEMBLY_GC` output is unchanged** (verified).

Lives on the fork branch **`harryzz/teavm:wasmgc-wasi-poc`** (one clean commit off
upstream master) — PR-ready. PR compare:
`https://github.com/konsoletyper/teavm/compare/master...harryzz:teavm:wasmgc-wasi-poc`

## What the patch does (11 files, all gated; browser path intact)
- **API:** `TeaVMTargetType.WEBASSEMBLY_GC_WASI` (maven `<targetType>`), a
  `Platforms.WEBASSEMBLY_GC_WASI` tag + `PlatformDetector.isWebAssemblyGCWasi()`,
  `WasmGCTarget.setWasi`/`isWasi()` (added to `TeaVMWasmGCHost`), `TeaVMTool`
  wiring, `wasi` threaded into `WasmGCIntrinsics`/`SystemIntrinsic`.
- **WASI behavior (gated on the flag / tag):** JSO plugin skipped (no `teavmJso`,
  no `wasm:js-string`); `Throwable` → empty stack trace; module-defined memory +
  internal heap globals (no `env`/`teavmMemory` imports); `notifyHeapResized` +
  `currentTimeMillis` → no-op/0 placeholder.

## Verified (2026-06-18)
- `<targetType>WEBASSEMBLY_GC_WASI</targetType>` (no `-D` flag) → spike module
  imports **0**, ~11 KB → `wasm-tools component embed wit/ + component new` →
  `world root { export packed-len: func(septets: s32) -> s32 }` → `../stub-host`
  `comp-host` calls it over the WIT → `1,3,5,7,9`.
- Plain `<targetType>WEBASSEMBLY_GC</targetType>` still emits the JS-coupled module
  (`teavmJso`×13, `wasm:js-string`×7, …) — **no browser regression.**

## Reproduce
1. clone `harryzz/teavm` (branch `wasmgc-wasi-poc`) — or `git apply WEBASSEMBLY_GC_WASI.patch` on a 0.16 clone.
2. `./gradlew publishToMavenLocal :tools:core:publishToMavenLocal :tools:maven:plugin:publishToMavenLocal -x test`
3. `mvn package` (in `../`, pom uses `<targetType>WEBASSEMBLY_GC_WASI</targetType>`).
4. `wasm-tools component embed wit target/wasm/spike.wasm -o e.wasm && wasm-tools component new e.wasm -o c.wasm`
5. `cd stub-host && cargo run --bin comp-host -- ../target/wasm/spike.component.wasm`

## Still TODO before upstream PR
- Real WASI floor: `currentTimeMillis`→`clock_time_get`, restore stdout
  (`putcharStdout`→`fd_write`), real self-managed heap growth (for allocating guests).
- A non-empty WasmGC stack trace for WASI (optional; empty is acceptable to start).
- Gradle plugin support for the new target (maven done); a wasi-WasmGC CI test.
- Open a TeaVM design issue to align on the target-vs-flag shape before PR.
