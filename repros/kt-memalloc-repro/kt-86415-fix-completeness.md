# KT-86415 fix completeness analysis

A walkthrough of *why* the first version of the `kt-memalloc-repro`
didn't trip the proposed patch, and whether the patch fully closes
the bug or has a residual edge case worth flagging upstream.

Filed against: <https://youtrack.jetbrains.com/issue/KT-86415/>
Reproducer:    <https://codeberg.org/harryzz/kt-memalloc-repro>
Patch:         `~/xl/kotlin` (locally built, published to mavenLocal as
               `kotlin-stdlib-wasm-wasi:2.4.255-SNAPSHOT`)


## The proposed patch (recap)

In `libraries/stdlib/wasm/src/kotlin/wasm/unsafe/MemoryAllocation.kt`:

```diff
-    private var availableAddress = startAddress
+    @PublishedApi
+    internal var availableAddress = startAddress
…
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

i.e. when a child scope is destroyed, propagate its high-water mark
up to the parent so the parent's `availableAddress` no longer
"forgets" the bytes the child used.


## The first repro: two sequential top-level scopes

```kotlin
withScopedMemoryAllocator { alloc -> alloc.allocate(65_536) }   // scope A, top-level
withScopedMemoryAllocator { alloc -> alloc.allocate(8)     }   // scope B, top-level
```

Both scopes are **top-level** — their `parent` is `null`. After scope
A exits, A's allocated bytes become reclaimable, and scope B's
allocator happily starts back at address 0. The first repro reported
"overlap" as if this were the bug.

**But that's not a bug — that's the documented contract of the API:**

```kotlin
// from Kotlin/Wasm stdlib:
//
// WARNING! Addresses allocated inside the [block] function become invalid
// after exiting the function.
public inline fun <T> withScopedMemoryAllocator(...)
```

Reading from `aPtr` after scope A exits is *undefined behavior by
design*. The fact that scope B's first allocation reuses address 0
isn't a bug — it's the whole point of "scoped". So my `destroy()`
patch was correctly a no-op for it: there's nothing to fix.


## The real bug: componentModelRealloc nested in an outer scope

The second repro matches the real downstream failure pattern:

```kotlin
withScopedMemoryAllocator { outer ->         // ← parent scope
    outer.allocate(8)
    componentModelRealloc(0, 0, 65_536)      // creates a CHILD of outer
    // …
}
// outer is destroyed.
freeAllComponentModelReallocAllocatedMemory()   // destroys the realloc-child
withScopedMemoryAllocator { newScope ->         // ← would overlap with bug
    newScope.allocate(65_536)
}
```

The contract violation here is different: **`componentModelRealloc`
is documented as returning memory that outlives the call** (the
canonical ABI's realloc; the host writes into it and reads it later).
Yet the underlying `ScopedMemoryAllocator` is invalidating it like an
ephemeral scope. The semantics of the two functions are
incompatible, but their implementations share the same allocator
class.

The chain that makes the patch work for this case:

1. `componentModelRealloc` creates the realloc-allocator scope as
   `outer.createChild()`. Both `outer` and the realloc-child are
   alive at this point.
2. `outer` block exits. `outer.destroy()`: `parent` is null so the
   `parent?.let { … }` is a no-op. But `outer` is now marked
   `destroyed = true`. The realloc-child is NOT destroyed.
3. `freeAll()` runs:
   - `reallocAllocator.destroy()`. With the patch, this bumps
     `outer.availableAddress` from `8` to `65544` (the realloc-child's
     high-water mark).
   - `currentAllocator = reallocAllocator.parent` (= the destroyed
     `outer` object — still in memory, just flagged destroyed).
   - `reallocAllocator = null`.
4. `newScope`'s `createAllocatorInTheNewScope()` does
   `currentAllocator?.createChild()`. `createChild()` does **not**
   check whether `this` is destroyed, so it reads `outer.availableAddress`
   (= `65544` after the patch) and creates a child of the destroyed
   outer starting at `65544`. No overlap.

Empirically verified by toggling `useKt86415Patch` in the repro's
`build.gradle.kts`:

| stdlib                                | longLivedPtr | newScope.allocate ptr | overlaps? |
|---|---|---|---|
| stock 2.4.0-RC                        | 8            | 16                    | **true** ← bug |
| patched 2.4.255-SNAPSHOT              | 8            | 65552                 | **false** ← fix |


## The residual edge case

If `componentModelRealloc` is ever called with **no active outer
`withScopedMemoryAllocator` scope**, the patch does *not* save us:

```kotlin
componentModelRealloc(0, 0, 65_536)   // currentAllocator is null → top-level realloc-allocator
freeAll()                              // destroys it. parent=null, patch is a no-op.
withScopedMemoryAllocator { /* fresh top-level at 0 */ }
```

In step 2, `reallocAllocator.parent` is `null`, so:
- The patch's `parent?.let { … }` doesn't fire.
- `freeAll` sets `currentAllocator = null` (because that's what
  `reallocAllocator.parent` is).
- The next `withScopedMemoryAllocator` makes a fresh top-level at
  `startAddress = 0`. Overlap is possible again.

**Is this case real?** Probably not in current Kotlin/Wasm code:
the Kotlin runtime's canonical-ABI plumbing seems to always invoke
`componentModelRealloc` inside some active scope (a WIT binding's
`withScopedMemoryAllocator`, or stdlib's println's
`withScopedMemoryAllocator`). But it's a hole in the fix that
JetBrains may want to plug — for example with a static
"last-top-level-high-water" variable:

```kotlin
private var lastTopLevelHighWater: Int = 0

internal fun createAllocatorInTheNewScope(): ScopedMemoryAllocator {
    check(reallocAllocator == null) { … }
    val allocator = currentAllocator?.createChild()
        ?: ScopedMemoryAllocator(lastTopLevelHighWater, parent = null)
    currentAllocator = allocator
    return allocator
}

internal fun destroy() {
    destroyed = true
    parent?.let { p ->
        p.suspended = false
        if (availableAddress > p.availableAddress) p.availableAddress = availableAddress
    } ?: run {
        if (availableAddress > lastTopLevelHighWater) lastTopLevelHighWater = availableAddress
    }
}
```

This makes memory monotonic *globally* (not per-parent). That's a
bigger semantic change — once a top-level scope writes bytes, they're
gone forever to ALL future top-level allocators, not just to its own
parent's children. The Kotlin team should weigh in on whether that's
acceptable for the canonical-ABI use case (it's probably fine —
componentModelRealloc users want monotonic anyway), or whether a
cleaner architectural fix would be to back `componentModelRealloc`
with a separate, non-scoped allocator pool entirely.


## Why our project's self-heal works regardless

Independent of any upstream fix, our downstream WASI preview1
adapter fork (`~/wart/wasmtime-src/crates/wasi-preview1-component-adapter/`)
ships a self-heal in `State::with`: when the magic sentinels at the
start/end of the adapter's State block don't match `MAGIC` (= "ugh!"),
it re-initializes the State in place before the assert fires. That
keeps the adapter robust against any allocator-semantics bug
upstream — Kotlin's, or hypothetically any other host that implements
`cabi_realloc` as a scope-reset allocator.

The minimum fix to ship upstream (the `destroy()` patch) is enough
for our project — every `componentModelRealloc` call we hit in
practice happens inside a wrapping scope, so my patch's parent
propagation works. The residual edge case (componentModelRealloc
called with no active outer scope) is academic for our codebase but
worth flagging on the YouTrack issue.
