@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
)

import kotlin.wasm.unsafe.Pointer
import kotlin.wasm.unsafe.componentModelRealloc
import kotlin.wasm.unsafe.freeAllComponentModelReallocAllocatedMemory
import kotlin.wasm.unsafe.withScopedMemoryAllocator

// Repro that matches the wart-app failure pattern. The actual flow is:
//
//  (a) An outer `withScopedMemoryAllocator` block is active when
//      `componentModelRealloc` is called for a long-lived block (in
//      wart this is the WASI preview1 adapter's State allocation,
//      triggered indirectly via wasiRandomGet -> random_get ->
//      State::new -> cabi_realloc inside a println scope).
//  (b) The outer block ends. `reallocAllocator` is still set.
//  (c) Code later calls `freeAllComponentModelReallocAllocatedMemory()`
//      defensively (every WIT binding does this to satisfy the
//      "Can't create new allocators while realloc-allocated memory
//      is not freed" check in `createAllocatorInTheNewScope`).
//  (d) A new `withScopedMemoryAllocator` block opens. With the bug
//      its first allocation overwrites the long-lived block.
//
// Run with stock 2.4.0-RC stdlib: see overlap = true.
// Run with patched stdlib (destroy() propagates availableAddress to
// parent): see overlap = false.

private var capturedLongLivedPtr: Int = 0
private var capturedOuterProbePtr: Int = 0
private var capturedNewProbePtr: Int = 0
private var capturedNewAllocPtr: Int = 0
private var capturedFirstFour: String = ""

fun main() {
    val longLivedSize = 65_536

    // (a) Outer scope active. componentModelRealloc creates a child.
    //     Note: we cannot call println in this section while
    //     reallocAllocator is non-null — it would throw.
    withScopedMemoryAllocator { outerAllocator ->
        capturedOuterProbePtr = outerAllocator.allocate(8).address.toInt()
        capturedLongLivedPtr = componentModelRealloc(
            originalPtr = 0,
            originalSize = 0,
            newSize = longLivedSize,
        )
        // Write a sentinel at longLivedPtr.
        val p = Pointer(capturedLongLivedPtr.toUInt())
        (p + 0).storeByte(0x55.toByte())
        (p + 1).storeByte(0x66.toByte())
        (p + 2).storeByte(0x77.toByte())
        (p + 3).storeByte(0x44.toByte())
    }
    // (b) Outer block ended. reallocAllocator still set.

    // (c) Defensive freeAll.
    freeAllComponentModelReallocAllocatedMemory()

    // (d) New scope. With the bug, overlaps longLivedPtr.
    withScopedMemoryAllocator { newAllocator ->
        capturedNewProbePtr = newAllocator.allocate(8).address.toInt()
        capturedNewAllocPtr = newAllocator.allocate(longLivedSize).address.toInt()
        val q = Pointer(capturedNewAllocPtr.toUInt())
        (q + 0).storeByte(0xAA.toByte())
        (q + 1).storeByte(0xBB.toByte())
        (q + 2).storeByte(0xCC.toByte())
        (q + 3).storeByte(0xDD.toByte())
    }

    // Read what's at longLivedPtr now.
    val p = Pointer(capturedLongLivedPtr.toUInt())
    capturedFirstFour = buildString {
        for (i in 0 until 4) {
            if (i > 0) append(",")
            append((p + i).loadByte().toInt().and(0xff).toString(16).padStart(2, '0'))
        }
    }

    // Now safe to print — reallocAllocator is null.
    println("outerScope probe(8)              -> ptr=$capturedOuterProbePtr")
    println("componentModelRealloc($longLivedSize)   -> longLivedPtr=$capturedLongLivedPtr")
    println("longLived range = [$capturedLongLivedPtr, ${capturedLongLivedPtr + longLivedSize})")
    println("freeAll done")
    println("newScope probe(8)                -> ptr=$capturedNewProbePtr")
    println("newScope.allocate($longLivedSize) -> ptr=$capturedNewAllocPtr")
    val overlaps = capturedLongLivedPtr < capturedNewAllocPtr + longLivedSize &&
        capturedNewAllocPtr < capturedLongLivedPtr + longLivedSize
    println("newScope OVERLAPS longLivedPtr?  $overlaps  (BUG if true)")
    println("longLivedPtr first 4 bytes:      [$capturedFirstFour]")
    println("  expected w/ fix:               [55,66,77,44]  (sentinel intact)")
    println("  expected w/ bug:               [aa,bb,cc,dd]  (overwritten)")
}
