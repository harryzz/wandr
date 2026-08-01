---
name: reference_wasm_dynamic_linking_shared_libs
description: "Why wandr guests can't share a heavy framework (Compose) as a system lib on-disk, at the wasm mechanism level. wasm linking IS traditional (wasm-ld, sections, PIC/GOT, dlopen) but ONLY for linear-memory (C/Rust). WasmGC (Kotlin/Compose) has NO dynamic-linking mechanism = the deep wall. Swift/.NET are heavy-but-linear-memory = toolchain-blocked, not wasm-wall-blocked. Extends docs/shared-runtime-and-app-size.md."
metadata:
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-08-01T11:32:38.698Z
---

Deep-dive 2026-08-01 (follow-on to `docs/shared-runtime-and-app-size.md`): CAN a
heavy guest framework be shipped once as a shared system library so thin apps
don't re-bundle it — WITHOUT the WIT seam (which can't express app↔framework)?
Answer at the wasm-mechanism level.

## wasm linking IS traditional — but linear-memory only
`wasm-ld` (LLVM lld) is a real linker: wasm `.o` files carry `linking` (symbol
table) + `reloc.*` sections; flags `--export`/`--import-memory`/`--import-table`/
`--allow-undefined`/`-shared`/`--experimental-pic`; static AND dynamic linking.
**Dynamic linking exists** = the Emscripten `MAIN_MODULE`/`SIDE_MODULE` model:
shared `Memory` + `Table`, a **GOT** (imported `__memory_base`/`__table_base`
globals), function pointers via `call_indirect` through the shared table,
`dlopen`/`dlsym` at runtime. **SEDL** (component-model Shared-Everything Dynamic
Linking) is the formalization of this; gained an executable spec test
(`test/linking/shared-everything-dynamic-linking.wast`, WebAssembly/component-model
2026-07-10). So "shared libraries for wasm" is REAL — for C/C++/Rust.

## The "fake vtable at link, real at runtime" idea = PLT/GOT, and it works (linear mem)
The natural proposal (supply a placeholder the linker accepts, fill the real
dispatch table at runtime, no WIT) is literally **relocations + PLT/GOT** — how
traditional dynamic linkers work, and what Emscripten side-modules do. In wasm
terms it's `call_indirect` through a shared table the runtime populates at
instantiation; wasm **imports** ARE "linker sees a named placeholder, runtime
binds the real one." So the RUNTIME primitive is NOT missing.

## Two walls stop it for the frameworks that actually hurt
1. **Compiler must PRESERVE the boundary.** The trick needs call sites to already
   be indirect stubs with relocations left in. Whole-program compilers (Kotlin/Wasm,
   SwiftWasm, .NET NativeAOT) **inline + DCE across the app↔framework boundary** →
   the seam is erased; you can't fake a vtable into an already-monolithic module
   (no symbols, calls inlined away). Fix = separate compilation w/ a boundary ABI =
   Kotlin **KT-82064** (multi-module, In Progress). Toolchain half.
2. **WasmGC has NO dynamic-linking mechanism at all.** Compose = Kotlin/Wasm →
   **WasmGC**: dispatch is **typed GC references** (`ref func` in struct fields +
   `call_ref`), types defined PER-MODULE. No linear-memory GOT to patch; cross-module
   GC type identity = the immature **type-imports/rec-group** story; no shipping
   typed-vtable-injection format. The PLT/GOT trick **does not port to GC**. Deep wall.

## The real axis is NOT "GC" — it's "heavy managed runtime + whole-program compile"
- **Rust/C/C++ (linear mem):** small, compact, AND dynamic-linkable → NO problem.
  wandr's Rust guests (dioxus-canvas, Slint, Bluesky, Floem) live here.
- **WasmGC — Kotlin/Compose (Dart/Java):** big AND no dynamic-linking mechanism =
  BOTH walls. The hard case. On-disk dedup blocked on WasmGC dynamic linking.
- **Heavy but LINEAR-MEMORY — Swift (ARC), .NET NativeAOT/Avalonia:** also big
  (Swift 65→191MB, Avalonia ~40MB) BUT linear-memory → the fake-vtable/side-module
  idea is THEORETICALLY applicable; blocked only by the toolchain whole-program-
  compiling, **not a wasm wall.**

## Forum/GitHub research (2026-08-01)
- **Swift: NOTHING in flight.** Native Swift `.dylib`/`.so`/library-evolution is
  heavily discussed on forums.swift.org, but **no SwiftWasm dynamic-linking
  proposal exists** — SwiftWasm stays static-only. Confirms toolchain-blocked, no
  active bridge.
- **.NET = the strongest second candidate — it ALREADY partly does this.**
  - **mono-wasm / Blazor model** ships the mono runtime (`dotnet.wasm`) ONCE + the
    app as loadable **`.dll` IL assemblies loaded at runtime** — the JVM-like
    "shared runtime + thin loadable units" the user was reaching for. Tradeoff =
    **interpreter** speed; Blazor AOT (IL→wasm) is faster but **~2× size** +
    monolithic (the same dynamic-vs-AOT tradeoff as JS engines). So .NET is a
    partial EXISTENCE PROOF of shared-runtime + thin apps on wasm.
  - **.NET uses Emscripten side modules** (real wasm dynamic linking) for native
    interop — active: dotnet/runtime **#123570** "LibraryImport fails to resolve
    side modules on browser-wasm" (open, 2026-07). Plus **#112984** new hostfxr
    hosting-API design (runtime/assembly resolution).
  - **.NET migrating to WasmGC** for SIZE (shed its own GC → community writeups
    claim ~40-60% smaller bundles) — orthogonal to dynamic linking.
  - **BUT wandr's Avalonia uses NativeAOT (ILC)** = the MONOLITHIC path (~40MB,
    self-contained, no shared runtime) — NOT the mono/IL shared-runtime model.
    So the shared-runtime capability EXISTS in .NET but wandr didn't pick it
    (NativeAOT chosen for perf/self-containment/no-JS-host). See [[reference_avalonia_wandr]].

## Swift = clearest proof it's toolchain-blocked, not wasm-blocked
Native Swift has a FULL dynamic-lib model: **`.swiftmodule`** (binary interface =
the "header"), **`.dylib`/`.so`** (shared libs, dyld/ld.so), and **library-evolution
"resilience"** (`-enable-library-evolution`, `@frozen`) — which accesses resilient
types through accessors + **runtime-computed layouts**, i.e. **Swift already ships
the fake-vtable/indirection idea natively**. But **SwiftWasm statically links
everything** (stdlib + Foundation + framework) into ONE `wasm32-wasip1` module —
no `.so` equivalent, no dyld-for-wasm, resilience not carried to wasm output. That
static bundling IS the 65MB. The capability exists on both sides (Swift dynamic
libs + wasm side-modules); nobody bridged them in SwiftWasm.

## wandr implication
- Rust guests: small/shareable, done — prefer them where size matters.
- Compose (WasmGC): near-term only lever = **framework-base zygote** (share runtime
  in RAM, `docs/shared-runtime-and-app-size.md`); on-disk dedup waits on WasmGC
  dynamic linking (deepest gap). NOT actionable.
- Swift/.NET: toolchain-blocked; theoretically fixable without a wasm-wall.
- JVM contrast: JVM shares classes because bytecode is **symbolic + late-bound** with
  a VM-provided resolver; wasm is **index + early-bound**, late-binding only at the
  coarse import boundary — a deliberate substrate tradeoff (fast total validation).

Related: [[reference_kotlin_wasm_component_model_status]] (KT-82064 multi-module),
[[reference_wasi_webgpu_gfx]], [[reference_floem_wandr_candidate]],
[[project_app_lifecycle_and_packaging]] (zygote), `docs/shared-runtime-and-app-size.md`.
