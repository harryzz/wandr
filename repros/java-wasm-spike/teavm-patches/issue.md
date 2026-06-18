# Design proposal: a JS-free WasmGC target for WASI / component-model hosts (`WEBASSEMBLY_GC_WASI`)

> Draft for konsoletyper/teavm. **Not yet posted** — review before submitting.
> Reference branch: `harryzz/teavm:wasmgc-wasi-poc` (one commit, `cdc6471`, off
> upstream `master`).

## Summary

I'd like to add a sibling to the `WEBASSEMBLY_GC` target that emits a WasmGC
module with **no JavaScript dependency** — no JSO, no `wasm:js-string`, no
`teavmConsole`/`teavmDate`/`teavmMemory` host imports — so pure-Java code runs on
a **non-JS host** (e.g. wasmtime) and, after the standard preview1→preview2
adapter, as a **WebAssembly component**. Today TeaVM's WasmGC output assumes a JS
embedder, so there is no path from Java to a JS-free WasmGC/WASI module.

Before opening a PR I'd like your guidance on the **shape** (new `TeaVMTargetType`
vs. a flag on `WasmGCTarget`) and naming. A working proof-of-concept is linked
above; details below.

## Motivation

I'm running guest UIs/logic on a `wasmtime` + component-model host (no JS engine
anywhere). Kotlin/Wasm, Rust, and C# already target it; I'd like Java to as well.
The existing `WEBASSEMBLY_GC` target is browser-oriented and pulls in a JS
runtime, so its output can't instantiate without a JS embedder. The **C backend
is already TeaVM's non-JS WasmGC-adjacent sibling** (char-array `String`, no JSO,
WASI via `tests.wasi`), which suggests a JS-free WasmGC target is a natural,
additive addition rather than a mutation of the browser target.

Key observation (so this is smaller than it looks): **`java.lang.String` needs no
work** — on WasmGC `getPlatformTags()` is `{WEBASSEMBLY_GC}`, so `isJavaScript()`
is false and `TString` is already char-array-backed. The JS imports come from the
**JSO plugin** plus **`Throwable`** (JS stack capture via `JSString`) and a couple
of runtime floor functions — not from String.

## What the PoC does

A single, additive, **browser-unchanged** change (verified: the plain
`WEBASSEMBLY_GC` target still emits `teavmJso`×N + `wasm:js-string`×7 +
`teavmDate`, byte-for-byte behavior preserved).

**Target plumbing**
- `Platforms.WEBASSEMBLY_GC_WASI` + `@PlatformMarker
  PlatformDetector.isWebAssemblyGCWasi()` (lets classlib branch at compile time).
- `TeaVMTargetType.WEBASSEMBLY_GC_WASI`; `TeaVMTool` constructs the WasmGC target
  in WASI mode (`WasmGCTarget.setWasi(true)`, added to `TeaVMWasmGCHost`).
- `WasmGCTarget` in WASI mode: adds `WEBASSEMBLY_GC_WASI` to the platform tags,
  drops the imported `env`/`memory` and **exports** the linear memory instead,
  makes the heap globals internal consts, and threads a `wasi` flag through
  `WasmGCIntrinsics` → `SystemIntrinsic`.
- `JSOPlugin` skips `WasmGCJso.install` when `isWasi()`; `TThrowable` skips JS
  stack capture under WASI (empty trace for now — see open questions).

**A real `wasi_snapshot_preview1` floor** (the module's *only* host imports)
- **stdout/stderr** → `WasmGCSupport.putChar*` route to `fd_write` (gated on
  `isWebAssemblyGCWasi()`); browser keeps `teavmConsole`.
- **wall clock** → `System.currentTimeMillis` → `WasmGCSupport.currentTimeMillis`
  via `clock_time_get`. `SystemIntrinsic` emits the call under `wasi`; the helper
  is linked on demand by a `methodReached` listener in `WasmGCDependencies` (only
  when `System.currentTimeMillis` is actually reached). Browser keeps `teavmDate`.
- **heap growth** stays the `memory.grow` intrinsic; only `notifyHeapResized` is
  gated off (self-managed, no host callback).

## Evidence

End-to-end, pure Java with `<targetType>WEBASSEMBLY_GC_WASI</targetType>`:

1. Core module imports **exactly** `wasi_snapshot_preview1.{fd_write,
   clock_time_get}`; nothing else.
2. `wasm-tools component new --adapt …preview1-adapter` → a valid **WASI 0.2
   component** importing `wasi:cli/{stdout,stderr}`, `wasi:io/*`,
   `wasi:clocks/*` and exporting custom WIT verbs.
3. Runs under `wasmtime`: a `System.out.println` guest prints over
   `wasi:cli/stdout`; `System.currentTimeMillis` returns a real epoch; an
   exported `s32->s32` function returns correct results.
4. Verified on real hardware (aarch64 / Android via a wasmtime host) — same wasm.

(There's also a small allocation/GC stress guest that exercises the floor; it
surfaced that wasmtime's deferred-RC WasmGC collector reclaims acyclic garbage
but not cycles — orthogonal to this proposal, just a note that the floor is
exercised under load.)

## Design questions (the reason for an issue before a PR)

1. **New `TeaVMTargetType` vs. a flag.** The PoC adds a `WEBASSEMBLY_GC_WASI`
   enum value that reuses `WasmGCTarget` with `setWasi(true)`. Would you prefer a
   first-class target type, or a `hostType: JS | WASI` (or `embedding:`) option on
   the existing WasmGC target with one `TeaVMTargetType`? This affects the public
   maven/gradle surface, so I'd rather agree before writing it.
2. **Naming.** `WEBASSEMBLY_GC_WASI`? `WEBASSEMBLY_GC_STANDALONE`? Something that
   leaves room for non-WASI non-JS hosts later?
3. **Floor scope.** Is `fd_write` + `clock_time_get` (+ self-managed heap) the
   right minimal floor to upstream first, with `args`/`environ`/`fd_read`/exit
   added incrementally? Should it crib the C backend's WASI layer more directly?
4. **Entry point.** The WASI target currently emits no `_start`; guests are driven
   via an exported function. Do you want a `_start`/`_initialize` reactor-or-command
   convention wired to `mainClass` for this target?
5. **`Throwable` stack traces.** Empty under WASI in the PoC. Worth a non-JS
   WasmGC capture (the C backend has a precedent), or leave minimal initially?

## Out of scope for the first PR (follow-ups)

Gradle-plugin support for the new target; a WASI-WasmGC CI test; richer floor
(args/env/fs/exit); non-empty stack traces.

## Compatibility

Additive only. The browser `WEBASSEMBLY_GC` path is untouched and re-verified
(same JS imports, same generated shape). All WASI behavior is gated behind the new
platform tag / `wasi` flag.

I'm happy to split the PoC commit into reviewable pieces (target plumbing → floor)
and rebase on `master`. Guidance on (1)/(2) would unblock that.
