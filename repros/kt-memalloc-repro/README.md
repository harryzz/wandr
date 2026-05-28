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

`Main.kt` runs with no skia / Compose / Android and measures **two**
things, so a fix can be judged on both:

- **[UAF]** — `componentModelRealloc` a block, write a sentinel
  `55 66 77 44` (models the WASI adapter's long-lived `State`),
  `freeAllComponentModelReallocAllocatedMemory()`, then open a new
  `withScopedMemoryAllocator` and write `AA BB CC DD`. Read the
  long-lived block back: sentinel intact = no use-after-free.
- **[reclaim]** — two `realloc`/`freeAll` cycles allocate small blocks
  `b1` then `b2`. `b2 == b1` means per-call realloc memory is actually
  reclaimed; `b2 > b1` means it leaks.

A correct fix must pass *both*. Results across three stdlib builds:

| stdlib | [UAF] | [reclaim] | verdict |
|--------|-------|-----------|---------|
| stock 2.4.0-RC | overwritten (`aa,bb,cc,dd`) | `b1==b2` | **BUG** — use-after-free |
| `destroy()` parent-bump patch | intact | `b1=131072, b2=135168` | **PARTIAL** — UAF fixed, per-call leaks |
| Tier 2: persistent realloc allocator + watermark `freeAll` | intact | `b1==b2==65536` | **PASS** |

Stock reclaims per-call memory but has the use-after-free. The
`destroy()` patch fixes the UAF but stops reclaiming *anything* —
JetBrains' "makes everything a memory leak." Only Tier 2 does both.

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

## On the fix

### The rejected patch — `destroy()` parent-bump

This repo originally proposed making `ScopedMemoryAllocator.destroy()`
propagate the child scope's high-water mark to its parent (see
[`kt-86415-fix-completeness.md`](kt-86415-fix-completeness.md), written
*before* the JetBrains response). JetBrains **rejected it**: never
reusing an address turns the use-after-free into a read of immutable
memory that "works" until the first repeated allocation — *"not a
solution to use-after-free, just defining UB conveniently (and making
everything a memory leak)"*. The `[reclaim]` column above is that leak.

### Tier 2 — persistent realloc allocator + watermark `freeAll`

A stdlib-side change (`kotlin/wasm/unsafe/MemoryAllocation.kt`) that
passes both probes:

- `reallocAllocator` is **not** destroyed/nulled by `freeAll`; it
  persists for the process lifetime.
- The first `freeAll` records a **watermark** — the realloc high-water
  mark at that point. The adapter's long-lived `State` sits below it.
- Every later `freeAll` rewinds the realloc allocator to the watermark:
  per-call memory above it is reclaimed; the long-lived block below is
  preserved.
- Scoped allocators opened while `reallocAllocator` is alive become its
  children, so they sit *above* realloc memory and can't collide — the
  `destroy()` parent-bump is no longer needed and is reverted.

The catch: the watermark is a **heuristic** — "realloc memory live at
the first `freeAll` is permanent." It holds when the adapter sets up
`State` before the guest's first marshalling cycle (true for the WASI
preview1 adapter), but a single monotonic watermark cannot represent a
long-lived allocation made *after* per-call traffic starts. The fully
general fix is a per-call realloc arena (a wit-bindgen codegen change),
or — JetBrains' actual recommendation — adapter-side: don't keep
`State` in canonical-ABI `realloc` memory at all.

KT-86415 (*Investigating*) decides whether the stdlib should support
`realloc` as a long-lived allocator at all — which is what determines
whether a Tier-2-style change is acceptable upstream.

## Context

Found while diagnosing a SIGILL on Android in a Compose Multiplatform
+ wasmtime + WASI-adapter project. The WASI adapter's static `State`
block, allocated once via `cabi_realloc` at cold init, was being
silently overwritten by an unrelated marshalling buffer — the
use-after-free above, manifesting as adapter-state corruption.
