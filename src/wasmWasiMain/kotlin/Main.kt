@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
)

import kotlin.wasm.unsafe.Pointer
import kotlin.wasm.unsafe.componentModelRealloc
import kotlin.wasm.unsafe.freeAllComponentModelReallocAllocatedMemory
import kotlin.wasm.unsafe.withScopedMemoryAllocator

// KT-86415 — use-after-free of canonical-ABI `realloc` memory on
// Kotlin/Wasm.
//
// A component-model runtime can use the exported `realloc`
// (`componentModelRealloc`) as a general allocator for LONG-LIVED
// storage. The wasmtime WASI preview1 component adapter does exactly
// this for its `State` block. Kotlin's wit-bindgen fork, however,
// assumes `realloc` is only ever short-lived copy-buffer scratch — as
// the Canonical ABI describes — and so calls
// `freeAllComponentModelReallocAllocatedMemory()` aggressively between
// WIT calls. After that free, the long-lived block is handed back out
// by the next `withScopedMemoryAllocator` and its contents are
// clobbered — a classic use-after-free.
//
// NOTE: this is *not* a `ScopedMemoryAllocator` bug — scoped addresses
// are correctly invalid outside their scope. See README.md for the
// JetBrains analysis on KT-86415.
//
// Flow — no skia / Compose / Android needed:
//   1. componentModelRealloc a block; write a sentinel; keep the ptr
//      (models the adapter's long-lived State).
//   2. freeAllComponentModelReallocAllocatedMemory()  — what every
//      Kotlin wit-bindgen WIT call does.
//   3. open a withScopedMemoryAllocator and allocate — reuses the
//      just-freed address range.
//   4. read the block back through the kept ptr — sentinel gone.

private const val BLOCK = 65_536
private var longLivedPtr = 0
private var readBack = ""

fun main() {
    // 1. Long-lived allocation via the canonical-ABI realloc export.
    //    componentModelRealloc must run inside an active scope; the
    //    block it returns is meant to outlive that scope.
    withScopedMemoryAllocator { _ ->
        longLivedPtr = componentModelRealloc(
            originalPtr = 0, originalSize = 0, newSize = BLOCK,
        )
        val p = Pointer(longLivedPtr.toUInt())
        (p + 0).storeByte(0x55.toByte())
        (p + 1).storeByte(0x66.toByte())
        (p + 2).storeByte(0x77.toByte())
        (p + 3).storeByte(0x44.toByte())
    }

    // 2. Kotlin's wit-bindgen frees all realloc memory between calls.
    freeAllComponentModelReallocAllocatedMemory()

    // 3. The next scoped allocation reuses the freed range.
    withScopedMemoryAllocator { alloc ->
        val q = Pointer(alloc.allocate(BLOCK).address.toInt().toUInt())
        (q + 0).storeByte(0xAA.toByte())
        (q + 1).storeByte(0xBB.toByte())
        (q + 2).storeByte(0xCC.toByte())
        (q + 3).storeByte(0xDD.toByte())
    }

    // 4. Read the long-lived block through the still-held pointer.
    val p = Pointer(longLivedPtr.toUInt())
    readBack = buildString {
        for (i in 0 until 4) {
            if (i > 0) append(",")
            append((p + i).loadByte().toInt().and(0xff).toString(16).padStart(2, '0'))
        }
    }

    // reallocAllocator is null again — safe to print.
    println("KT-86415 — canonical-ABI realloc use-after-free")
    println("  long-lived realloc block @ $longLivedPtr; sentinel written = [55,66,77,44]")
    println("  after freeAll + one new withScopedMemoryAllocator: read back = [$readBack]")
    if (readBack == "55,66,77,44") {
        println("  => sentinel intact — no use-after-free observed")
    } else {
        println("  => USE-AFTER-FREE: long-lived realloc memory was reused and overwritten")
    }
}
