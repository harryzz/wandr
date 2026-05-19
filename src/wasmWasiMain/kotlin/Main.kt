@file:OptIn(kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class)

import kotlin.wasm.unsafe.Pointer
import kotlin.wasm.unsafe.withScopedMemoryAllocator

// Minimal Kotlin/Wasm 2.4.0-RC repro of the
// `ScopedMemoryAllocator.destroy()` bug.
//
// `destroy()` does NOT advance the parent allocator's
// `availableAddress`. The bytes used by a child scope become
// "available" again from the parent's perspective. A sibling scope
// opened from the same parent reuses the SAME address range — even
// if external code (the WASI preview1 adapter, a foreign component,
// etc.) still has a live pointer into the destroyed child's memory.
//
// This is the load-bearing piece of `feedback_wasi_adapter_state_corruption`
// on wart: the WASI adapter's State block, allocated via cabi_realloc
// (= componentModelRealloc → backed by ScopedMemoryAllocator), gets
// overwritten by a subsequent unrelated `withScopedMemoryAllocator`.
//
// EXPECTED OUTPUT (with the bug present, today's Kotlin):
//   scope A allocate(65536) -> ptr A0=...
//   scope A done
//   scope B allocate(8)     -> ptr B0=A0   ← reuses scope A's start address!
//   B0 lies inside scope A's used range? true
//
// EXPECTED OUTPUT (after fixing destroy() to bump parent.availableAddress):
//   scope A allocate(65536) -> ptr A0=...
//   scope A done
//   scope B allocate(8)     -> ptr B0=A0+65536  ← advanced past A's range
//   B0 lies inside scope A's used range? false

fun main() {
    var aPtr = 0
    var aSize = 65_536
    withScopedMemoryAllocator { alloc ->
        aPtr = alloc.allocate(aSize).address.toInt()
        println("scope A allocate($aSize) -> ptr A0=$aPtr (0x${aPtr.toUInt().toString(16)})")
        // Write a sentinel at start so we can observe overwrites later.
        val p = Pointer(aPtr.toUInt())
        (p + 0).storeByte(0x55.toByte())
        (p + 1).storeByte(0x66.toByte())
        (p + 2).storeByte(0x77.toByte())
        (p + 3).storeByte(0x44.toByte())
    }
    println("scope A done")

    // scope A's bytes are still in linear memory (destroy doesn't zero).
    // Read the sentinel to confirm:
    val ap = Pointer(aPtr.toUInt())
    val sentinelHex = buildString {
        append("[")
        for (i in 0 until 4) {
            if (i > 0) append(", ")
            append("0x")
            append((ap + i).loadByte().toInt().and(0xff).toString(16).padStart(2, '0'))
        }
        append("]")
    }
    println("scope A first 4 bytes still readable: $sentinelHex (expected [0x55, 0x66, 0x77, 0x88])")

    var bPtr = 0
    withScopedMemoryAllocator { alloc ->
        bPtr = alloc.allocate(8).address.toInt()
        println("scope B allocate(8)     -> ptr B0=$bPtr (0x${bPtr.toUInt().toString(16)})")
        // Write a different pattern at scope B's start.
        val p = Pointer(bPtr.toUInt())
        (p + 0).storeByte(0xAA.toByte())
        (p + 1).storeByte(0xBB.toByte())
        (p + 2).storeByte(0xCC.toByte())
        (p + 3).storeByte(0xDD.toByte())
    }
    println("scope B done")

    val overlaps = bPtr in aPtr until (aPtr + aSize)
    println("B0 lies inside scope A's used range? $overlaps (this is the bug if true)")

    // Re-read scope A's sentinel. Scope B's write should have clobbered it if overlap is true.
    val sentinelHex2 = buildString {
        append("[")
        for (i in 0 until 4) {
            if (i > 0) append(", ")
            append("0x")
            append((ap + i).loadByte().toInt().and(0xff).toString(16).padStart(2, '0'))
        }
        append("]")
    }
    println("scope A first 4 bytes after scope B: $sentinelHex2")
    if (sentinelHex2 != sentinelHex) {
        println("    ^ corrupted by scope B")
    }
}
