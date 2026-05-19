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

The repro matches the actual failure pattern in downstream code:

1. An outer `withScopedMemoryAllocator` block is active.
2. Inside it, `componentModelRealloc` is called for a long-lived block.
3. The outer block ends; `reallocAllocator` is still set.
4. `freeAllComponentModelReallocAllocatedMemory()` is called.
5. A new `withScopedMemoryAllocator` block opens and allocates — and
   overlaps with the long-lived block.

```
$ wasmtime run …               # stock 2.4.0-RC stdlib
outerScope probe(8)              -> ptr=0
componentModelRealloc(65536)     -> longLivedPtr=8
longLived range = [8, 65544)
freeAll done
newScope probe(8)                -> ptr=8         ← SAME as longLivedPtr!
newScope.allocate(65536)         -> ptr=16
newScope OVERLAPS longLivedPtr?  true  (BUG if true)
longLivedPtr first 4 bytes:      [55,66,77,44]    ← sentinel still readable,
                                                    but only because newScope
                                                    didn't store to that range
```

After applying the proposed fix (see `build.gradle.kts` for how to
switch to a locally-published patched stdlib):

```
newScope probe(8)                -> ptr=65544     ← past long-lived range
newScope.allocate(65536)         -> ptr=65552
newScope OVERLAPS longLivedPtr?  false            ← no overlap
```

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

```diff
-    private var availableAddress = startAddress
+    @PublishedApi
+    internal var availableAddress = startAddress
...
     internal fun destroy() {
         destroyed = true
-        parent?.suspended = false
+        parent?.let { p ->
+            p.suspended = false
+            if (availableAddress > p.availableAddress) {
+                p.availableAddress = availableAddress
+            }
+        }
     }
```

This propagates the child's high-water mark up to the parent. Memory
becomes monotonic within a parent scope. A nicer fix would be to back
`componentModelRealloc` with a separate non-scoped allocator pool that
doesn't share state with `withScopedMemoryAllocator` at all, but the
~10-line change above is the minimum.

**Empirically verified** with a locally-built patched stdlib
(2.4.255-SNAPSHOT). See the toggle in `build.gradle.kts` to switch
between stock and patched.

## Context

Found while diagnosing a SIGILL on Android in a Compose Multiplatform
+ wasmtime + WASI-adapter project. The WASI adapter's static `State`
block, allocated once via `cabi_realloc` at cold init, was being
silently overwritten by an unrelated audio-test marshalling buffer —
because both ended up at the same parent-relative address after a
`freeAllComponentModelReallocAllocatedMemory` ↔
`withScopedMemoryAllocator` toggle.
