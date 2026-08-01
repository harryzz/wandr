# WASM dynamic linking & shared libraries — can a heavy framework be shipped once?

> **Scope.** Companion to [`shared-runtime-and-app-size.md`](shared-runtime-and-app-size.md).
> That doc asks "why are apps big and how do we share a runtime"; this one goes a
> layer down: **at the wasm *mechanism* level, what would it take to ship a heavy
> framework (Compose/skiko) once as a system shared library that thin apps link
> against — WITHOUT the WIT seam** (which can't express the app↔framework boundary).
> TL;DR: for **linear-memory** languages it's basically an engineering/adoption
> problem (the mechanism exists); for **WasmGC** (Kotlin/Compose) it needs new wasm
> spec capabilities first. Analysis captured 2026-08-01.

## The WIT seam can't express app↔framework

Components link at the WIT canonical-ABI seam — coarse functions, flat/handle
types. An app uses Compose through rich *Kotlin-language* API (`@Composable`
compiler-plugin functions, generics, closures, coroutines, deep object graphs).
None of that crosses a WIT boundary. So "share the framework" must happen at a
lower layer than WIT — i.e. **traditional linking / dynamic linking**, not
component composition. Hence this doc.

## wasm linking *is* traditional — but linear-memory only

`wasm-ld` (LLVM lld) is a real linker over real object files:
- wasm `.o` files carry a **`linking`** custom section (symbol table) + **`reloc.*`**
  relocation sections.
- Flags: `--export`/`--export-dynamic`, `--import-memory`, `--import-table`,
  `--allow-undefined`, `-shared`, `--experimental-pic`.
- **Static *and* dynamic linking.** Dynamic = the **Emscripten `MAIN_MODULE`/
  `SIDE_MODULE`** model: a shared `Memory` + `Table`, a **GOT** (imported
  `__memory_base`/`__table_base` globals), function pointers via `call_indirect`
  through the shared table, and `dlopen`/`dlsym` to load + relocate a side module
  at runtime.

**"Shared libraries for wasm" is real — for C/C++/Rust.** [SEDL] (component-model
Shared-Everything Dynamic Linking) is the standards-track formalization of this
exact scheme; it gained an executable spec test
(`test/linking/shared-everything-dynamic-linking.wast`) on 2026-07-10.

### The "fake vtable at link, real at runtime" idea = PLT/GOT

The natural proposal — hand the linker a placeholder it accepts, then fill the real
dispatch table at runtime — is literally **relocations + PLT/GOT**, how every
traditional dynamic linker works. In wasm terms it's `call_indirect` through a
shared table the runtime populates at instantiation; wasm **imports** already *are*
"linker sees a named placeholder, runtime binds the real one." **The runtime
binding primitive is not missing.**

## Why the JVM shares classes and wasm doesn't (by design)

| | JVM | WASM |
|---|---|---|
| Internal references | **symbolic** (constant pool, by name) | **numeric index** (`call 42`) |
| Cross-unit references | symbolic, same mechanism | **named imports/exports**, only at the module boundary |
| When bound | **lazily, per call site, at first use** | internal: compile/validate time; imports: instantiation |
| Who resolves | the **VM's class loader + verifier + resolver** | the engine's import binder (once, at instantiate) |
| Object model | one shared heap + type system, resolved by the VM | per-module (linear mem) or shared GC heap, no runtime class-linking by name |

wasm validates the *entire* module up front — fast, total, once — *because* it's
index-based and self-contained. That's a deliberate substrate tradeoff (validation
speed + security) in direct tension with fine-grained runtime late-binding. wasm
pushes late-binding to **host imports at instantiation** or to **the guest
language's own runtime**. The JVM *is* a managed late-binding runtime; wasm is a
lower-level target.

## The two walls (why it doesn't "just work" for Compose)

1. **The compiler must PRESERVE the boundary.** The trick needs call sites that are
   already indirect stubs with relocations left in. Whole-program compilers
   (Kotlin/Wasm, SwiftWasm, .NET NativeAOT) **inline + dead-code-eliminate across**
   the app↔framework boundary → the seam is *erased* (no symbols, calls inlined
   away). You cannot fake a vtable into an already-monolithic module. Fix = separate
   compilation with a boundary ABI (Kotlin **KT-82064**). **Partly addressed:**
   **KT-86919** (Fixed ~2026-06) adds `kotlin.wasm.compilationMode` with
   `multimodule-open-world` / `multimodule-closed-world` — the compiler can now emit
   a separate wasm file per klib (open-world = compiled independently). **But** it's
   motivated by build-time incrementality and wired via JS/ES-module glue, **not**
   distribution/dynamic-linking; it doesn't yet make Compose a cross-app shared lib,
   and it doesn't touch Wall #2.
2. **WasmGC has NO dynamic-linking mechanism at all.** Compose = Kotlin/Wasm →
   **WasmGC**: dispatch is **typed GC references** (`ref func` in struct fields +
   `call_ref`), types defined **per-module**. There's no linear-memory GOT to patch;
   cross-module GC type identity is the immature **type-imports** story; there's no
   typed-vtable relocation format. **The PLT/GOT trick does not port to GC.**

## The real axis is "heavy managed runtime + whole-program compile" — split two ways

- **Rust/C/C++ (linear memory):** small, compact, **and** dynamic-linkable → no
  problem. wandr's Rust guests (dioxus-canvas, Slint, Bluesky, Floem) live here.
- **WasmGC — Kotlin/Compose (Dart, Java):** big **and** no dynamic-linking mechanism
  = **both** walls. The hard case.
- **Heavy but *linear-memory* — Swift (ARC), .NET NativeAOT/Avalonia:** also big
  (Swift 65→191 MB, Avalonia ~40 MB) **but linear-memory** → the fake-vtable/side-
  module idea is **theoretically applicable**; blocked only by the toolchain whole-
  program-compiling, **not a wasm wall**.

### Swift — clearest proof it's toolchain-blocked, not wasm-blocked
Native Swift has a full dynamic-lib model: **`.swiftmodule`** (binary interface =
the "header"), **`.dylib`/`.so`** (dyld/ld.so), and **library-evolution
"resilience"** (`-enable-library-evolution`) — which accesses resilient types via
accessors + **runtime-computed layouts**, i.e. Swift **already ships the
fake-vtable/indirection idea natively**. But **SwiftWasm statically links
everything** into one `wasm32-wasip1` module — no `.so` equivalent, no dyld-for-wasm,
resilience not carried to wasm. That static bundling *is* the 65 MB. Forum/GitHub
check (2026-08-01): **no SwiftWasm dynamic-linking proposal in flight.**

### .NET — the strongest second candidate; it already *partly does this*
- **mono-wasm / Blazor** ships the mono runtime (`dotnet.wasm`) **once** + the app as
  loadable **`.dll` IL assemblies loaded at runtime** — the JVM-like "shared runtime
  + thin loadable units". Tradeoff: **interpreter** speed; Blazor AOT (IL→wasm) is
  faster but **~2× size** + monolithic (same dynamic-vs-AOT tradeoff as JS engines).
  → a real **existence proof** of shared-runtime + thin apps on wasm.
- **.NET uses Emscripten side modules** (real wasm dynamic linking) for native
  interop — active: dotnet/runtime **#123570** ("LibraryImport fails to resolve side
  modules on browser-wasm"), **#112984** (new hostfxr hosting API).
- **.NET migrating to WasmGC** for *size* (shed its own GC; community writeups claim
  ~40–60% smaller) — orthogonal to dynamic linking.
- **But wandr's Avalonia uses NativeAOT (ILC)** = the monolithic path, **not** the
  mono shared-runtime model. The capability exists in .NET; wandr didn't pick it.

## What it would take — the full matrix

### Linear memory (C/Rust; Swift-ARC, .NET-NativeAOT)

| | Exists | Needed | Effort |
|---|---|---|---|
| **Runtime** | shared Memory+Table, GOT, `call_indirect`, imports-by-name, PIC, Emscripten `dlopen` | a **standardized, engine-native** linker + **AOT-compatible** dynamic linking → **SEDL in wasmtime**; precompiled shared libs linked at load | **engineering** (no new wasm capability) |
| **Compiler** | `wasm-ld -shared`/`--experimental-pic` → C/C++/Rust emit side modules today | heavy toolchains (**SwiftWasm, .NET-NativeAOT**) must emit a **separable side module** + **preserve the boundary** (no whole-program inline) + agree on mem/table layout | **per-toolchain adoption** |

Existence proofs: Emscripten side modules (C/C++); **.NET/mono-wasm** (shared runtime + loadable IL).

### WasmGC (Kotlin/Compose, Dart, Java)

| | Exists | Needed | Effort |
|---|---|---|---|
| **Runtime** | shared GC heap; GC refs cross boundaries; `call_ref` dispatch | ① **cross-module GC type identity** (type-imports / structural type sharing — immature); ② a **relocation/linking model for typed GC refs** (no GOT equivalent); ③ **engine dynamic-linking for GC modules** | **spec + research** |
| **Compiler** | Kotlin/Wasm emits WasmGC. **NEW: multi-module compilation shipped** — **KT-86919** (Fixed ~2026-06): `kotlin.wasm.compilationMode = monolith \| multimodule-open-world \| multimodule-closed-world`; each klib → a separate wasm(+mjs) file, incl. an **open-world "total module independence"** mode. | Repurpose that from **build-time** to **distribution**: cross-app shared libs, a stable **GC boundary ABI**, and it still doesn't cross the **runtime** type-linking wall. KGP umbrella **KT-82064 still In Progress** (exports KT-81595, closed-world KGP KT-84108 Open). | **toolchain — building block landed, but aimed elsewhere; gated on the runtime half** |

Existence proof: **none for dynamic linking** — but the compiler can now *emit* separate WasmGC modules (motivated by faster/incremental builds + JS-module wiring, **not** cross-app shared libs).

## Verdict + the critical dependency

- **Linear memory:** both halves basically **exist** (wasm-ld/PIC/GOT/dlopen + SEDL);
  what's left is **wiring + adoption** — SEDL in the runtime, and Swift/.NET-AOT
  choosing to emit side modules. Runtime and compiler pieces are **independent and
  both present** → wireable today.
- **WasmGC:** the **runtime/spec half doesn't exist yet**, and the **compiler half
  (KT-82064) is gated on it**. Strict ordering: **spec → runtime → compiler**. This
  is exactly why Compose is stuck while Rust guests aren't.

## wandr implications

- **Rust guests:** small/shareable — prefer them where size matters (a point in favor
  of the Floem/dioxus/Slint lanes).
- **Compose (WasmGC):** near-term the only lever is the **framework-base zygote**
  (share the runtime in *RAM*, [`shared-runtime-and-app-size.md`](shared-runtime-and-app-size.md));
  on-disk dedup waits on WasmGC dynamic linking. **Not actionable.**
- **Swift / .NET:** toolchain-blocked but **not** wasm-wall-blocked; a real (if
  upstream) engineering path exists.

## Tracking

- **SEDL** — `WebAssembly/component-model`, `design/mvp/examples/SharedEverythingDynamicLinking.md`
  + `test/linking/shared-everything-dynamic-linking.wast` (spec test, 2026-07-10).
- **Kotlin KT-86919** — *Fixed ~2026-06* — `kotlin.wasm.compilationMode` (monolith /
  multimodule-open-world / multimodule-closed-world): separate wasm+mjs per klib. The
  first real crack in Wall #1 (compiler can emit separate WasmGC modules) — but
  build-time-motivated + JS-wired, not a distribution/shared-lib mechanism yet.
- **Kotlin KT-82064** — multi-module compilation *Gradle-plugin* support (In Progress) —
  the umbrella; sub-issues for exports (KT-81595) / closed-world KGP (KT-84108) still Open.
- **WasmGC type-imports** — the spec gap for cross-module GC type identity.

[SEDL]: https://github.com/WebAssembly/component-model/blob/main/design/mvp/examples/SharedEverythingDynamicLinking.md

## See also
- [`shared-runtime-and-app-size.md`](shared-runtime-and-app-size.md) — the app-size / zygote-COW story.
- Memory: `[[reference_wasm_dynamic_linking_shared_libs]]`, `[[reference_kotlin_wasm_component_model_status]]`.
