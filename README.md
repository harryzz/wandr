# kt-memalloc-repro

Minimal standalone Kotlin/Wasm 2.4.0-RC reproducer for
[**KT-86415**](https://youtrack.jetbrains.com/issue/KT-86415) — a
**use-after-free of canonical-ABI `realloc` memory**.

> A component-model runtime can use the exported `realloc`
> (`componentModelRealloc` on the Kotlin side) as a general allocator
> for **long-lived** storage. Kotlin's `wit-bindgen` fork assumes
> `realloc` is only ever short-lived copy-buffer scratch — as the
> [Canonical ABI](https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md)
> describes — and so calls `freeAllComponentModelReallocAllocatedMemory()`
> aggressively between WIT calls. After that free, the long-lived
> block is reused by the next `withScopedMemoryAllocator` and its
> contents are silently overwritten.

## Framing note (read this)

An earlier version of this repro framed the bug as
*"`ScopedMemoryAllocator.destroy()` does not propagate the child's
`availableAddress` to the parent."* **That framing is wrong** — and
was corrected by JetBrains (Jim Teichgräber) on KT-86415:

- `ScopedMemoryAllocator` is **correct**. Scoped addresses are *meant*
  to be invalid outside their scope; comparing addresses across scopes
  is meaningless.
- The actual bug is a **classic use-after-free**: the runtime holds
  `realloc`-allocated memory with effectively static storage duration,
  and Kotlin's `freeAll` frees it out from under the holder.
- Who is responsible is **spec-ambiguous** — `CanonicalABI.md` neither
  states its listed `realloc` uses are exhaustive nor forbids others.
  JetBrains is deciding whether to support `realloc` as a general
  long-lived allocator. KT-86415 state: *Investigating*.

This repro and README are reframed accordingly: it now demonstrates
the use-after-free directly (sentinel corruption), not an
address-comparison.

## What it shows

`Main.kt` reproduces the failure with no skia / Compose / Android:

1. Inside a `withScopedMemoryAllocator`, call `componentModelRealloc`
   for a block, write a sentinel `55 66 77 44`, keep the pointer —
   this models the WASI preview1 adapter's long-lived `State`.
2. `freeAllComponentModelReallocAllocatedMemory()` — exactly what
   every Kotlin `wit-bindgen` WIT call does between invocations.
3. Open a new `withScopedMemoryAllocator` and allocate — this reuses
   the just-freed address range and writes `AA BB CC DD`.
4. Read the long-lived block back through the still-held pointer.

```
$ wasmtime run …               # stock 2.4.0-RC stdlib
KT-86415 — canonical-ABI realloc use-after-free
  long-lived realloc block @ 8; sentinel written = [55,66,77,44]
  after freeAll + one new withScopedMemoryAllocator: read back = [aa,bb,cc,dd]
  => USE-AFTER-FREE: long-lived realloc memory was reused and overwritten
```

The long-lived block — written before the `freeAll` — comes back
holding the *new* scope's bytes. Anything still using the original
pointer (the adapter's `State`) is now reading another allocation's
data.

## Build + run

Requires Kotlin 2.4.0-RC (or newer), `wasm-tools`, `wasmtime` 44+, and
the WASI Preview 1 component adapter from the wasmtime tree (built once
with `cargo build -p wasi-preview1-component-adapter --target wasm32-unknown-unknown --release`).

```bash
./gradlew compileProductionExecutableKotlinWasmWasi

wasm-tools component new \
    build/compileSync/wasmWasi/main/productionExecutable/kotlin/kt-memalloc-repro.wasm \
    --adapt <path-to>/wasi_snapshot_preview1.wasm \
    -o /tmp/repro.wasm

wasmtime run --wasm gc=y --wasm function-references=y --wasm exceptions=y \
    --wasi preview2 /tmp/repro.wasm
```

## On the "fix"

This repo previously proposed a `destroy()` patch that propagates a
child scope's high-water mark to its parent (see
[`kt-86415-fix-completeness.md`](kt-86415-fix-completeness.md), written
*before* the JetBrains response). JetBrains **rejected it as a fix**:
never-freeing just turns the use-after-free into a read of immutable
memory that "works" until the first repeated allocation — *"not a
solution to use-after-free, just defining UB conveniently (and making
everything a memory leak)"* — and it is fragile with multiple parallel
child allocators at the same scope.

So there is no clean stdlib-side patch yet. The real fix is one of:

- **adapter-side** — the WASI preview1 component adapter must not keep
  long-lived `State` in canonical-ABI `realloc` memory; or
- **upstream Kotlin** decides to support `realloc` as a general
  long-lived allocator and stops calling `freeAll` so aggressively.

That decision (KT-86415, *Investigating*) determines where the fix
belongs. The downstream project that hit this ships a pragmatic
stopgap — the rejected patch *plus* a self-heal in its WASI-adapter
fork — but that is a workaround, not a resolution.

## Context

Found while diagnosing a SIGILL on Android in a Compose Multiplatform
+ wasmtime + WASI-adapter project. The WASI adapter's static `State`
block, allocated once via `cabi_realloc` at cold init, was being
silently overwritten by an unrelated marshalling buffer — the
use-after-free above, manifesting as adapter-state corruption.
