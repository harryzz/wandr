# kt-memalloc-repro

Minimal standalone Kotlin/Wasm 2.4.0-RC reproducer for a bug in
`kotlin.wasm.unsafe.ScopedMemoryAllocator`:

> **`ScopedMemoryAllocator.destroy()` does not propagate the child's
> `availableAddress` back to the parent.** When the child is destroyed,
> the parent's bump pointer is still wherever it was before the child
> opened, so a sibling scope opened from the same parent reuses the
> *same* address range — overwriting bytes the destroyed child wrote.

This breaks `componentModelRealloc` (the Component Model canonical-ABI
realloc, added in [KT-65030] / Kotlin 2.4.0-Beta1) for long-lived
allocations, because `componentModelRealloc` is built on top of
`ScopedMemoryAllocator` and the WASI Preview 1 component adapter (and
similar hosts) expects `cabi_realloc(null, 0, ...)` to return memory
that stays valid for the program's lifetime.

[KT-65030]: https://youtrack.jetbrains.com/issue/KT-65030

## What it shows

```
scope A allocate(65536) -> ptr A0=0
scope A done
scope A first 4 bytes still readable: [0x73, 0x63, 0x6f, 0x70]
    ^ NOT our 0x55 0x66 0x77 0x44 sentinel — another scope (from the
      intervening println's internal buffer) already overwrote it.

scope B allocate(8)     -> ptr B0=0
scope B done
B0 lies inside scope A's used range? true (this is the bug)
scope A first 4 bytes after scope B: [0x42, 0x30, 0x20, 0x6c]
    ^ corrupted by scope B
```

Every `withScopedMemoryAllocator` block's first allocation returns
address `0` regardless of what prior scopes wrote there.

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

## Proposed fix

In `libraries/stdlib/wasm/src/kotlin/wasm/unsafe/MemoryAllocation.kt`:

```kotlin
internal fun destroy() {
    destroyed = true
    parent?.let { p ->
        p.suspended = false
        if (availableAddress > p.availableAddress) {
            p.availableAddress = availableAddress
        }
    }
}
```

(`availableAddress` visibility needs to change from `private` to
`internal` for the parent to read+write its own copy from `destroy()`.)

This propagates the child's high-water mark up; memory becomes
monotonic within a parent scope. A nicer fix would be to back
`componentModelRealloc` with a separate non-scoped allocator pool that
doesn't share state with `withScopedMemoryAllocator` at all.

## Context

Found while diagnosing a SIGILL on Android in a Compose Multiplatform
+ wasmtime + WASI-adapter project. The WASI adapter's static `State`
block, allocated once via `cabi_realloc` at cold init, was being
silently overwritten by an unrelated audio-test marshalling buffer —
because both ended up at the same parent-relative address after a
`freeAllComponentModelReallocAllocatedMemory` ↔
`withScopedMemoryAllocator` toggle.
