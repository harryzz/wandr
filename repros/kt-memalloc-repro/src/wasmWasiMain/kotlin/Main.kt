@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
)

import kotlin.wasm.unsafe.Pointer
import kotlin.wasm.unsafe.componentModelRealloc
import kotlin.wasm.unsafe.freeAllComponentModelReallocAllocatedMemory
import kotlin.wasm.unsafe.withScopedMemoryAllocator

// KT-86415 — canonical-ABI `realloc` use-after-free, with a per-call
// reclamation probe so the three stdlib variants are distinguishable.
//
// A component-model runtime can use the exported `realloc`
// (`componentModelRealloc`) as a general allocator for LONG-LIVED
// storage — the wasmtime WASI preview1 adapter does exactly this for
// its `State`. Kotlin's wit-bindgen fork assumes `realloc` is only
// short-lived copy-buffer scratch and calls
// `freeAllComponentModelReallocAllocatedMemory()` aggressively between
// WIT calls, freeing the long-lived block out from under its holder.
//
// This repro measures TWO things, so a fix can be judged on both:
//   [UAF]     does a long-lived realloc block survive a freeAll +
//             a new scope?  (intact = no use-after-free)
//   [reclaim] is per-call realloc memory actually reclaimed across
//             freeAll cycles?  (b2 reuses b1 = no leak)
//
// Expected results by stdlib variant:
//   stock 2.4.0-RC          : BUG     — UAF; per-call reclaim works
//   our destroy() patch     : PARTIAL — UAF fixed, but per-call leaks
//   Tier 2 (persistent      : PASS    — UAF fixed AND per-call reclaimed
//     reallocAllocator +
//     watermark freeAll)

private const val BLOCK = 65_536
private const val SMALL = 4_096

private var longLivedPtr = 0
private var readBack = ""
private var b1 = 0
private var b2 = 0

fun main() {
    // 1. Long-lived realloc block + sentinel — models the adapter's State.
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
    freeAllComponentModelReallocAllocatedMemory()   // first freeAll

    // 2. A fresh scope allocates + writes — the use-after-free probe.
    withScopedMemoryAllocator { alloc ->
        val q = Pointer(alloc.allocate(BLOCK).address.toInt().toUInt())
        (q + 0).storeByte(0xAA.toByte())
        (q + 1).storeByte(0xBB.toByte())
        (q + 2).storeByte(0xCC.toByte())
        (q + 3).storeByte(0xDD.toByte())
    }

    // 3. Per-call reclamation probe — two realloc/freeAll cycles. If
    //    per-call realloc memory is reclaimed, b2 reuses b1's address.
    b1 = componentModelRealloc(originalPtr = 0, originalSize = 0, newSize = SMALL)
    freeAllComponentModelReallocAllocatedMemory()
    b2 = componentModelRealloc(originalPtr = 0, originalSize = 0, newSize = SMALL)
    freeAllComponentModelReallocAllocatedMemory()

    // Read the long-lived block back through the still-held pointer.
    val p = Pointer(longLivedPtr.toUInt())
    readBack = buildString {
        for (i in 0 until 4) {
            if (i > 0) append(",")
            append((p + i).loadByte().toInt().and(0xff).toString(16).padStart(2, '0'))
        }
    }

    val uafOk = readBack == "55,66,77,44"
    val reclaimOk = b1 == b2
    println("KT-86415 — realloc use-after-free + per-call reclamation")
    println("  [UAF]     long-lived block @ $longLivedPtr; sentinel [55,66,77,44]; read back [$readBack]")
    println("            => " + if (uafOk) "intact — no use-after-free"
                                  else "USE-AFTER-FREE — long-lived block overwritten")
    println("  [reclaim] per-call realloc: b1 @ $b1, b2 @ $b2")
    println("            => " + if (reclaimOk) "b2 reuses b1 — per-call memory reclaimed"
                                  else "b2 past b1 — per-call memory LEAKED")
    val verdict = when {
        uafOk && reclaimOk  -> "PASS — no use-after-free and no leak"
        uafOk && !reclaimOk -> "PARTIAL — use-after-free fixed, but per-call memory leaks"
        !uafOk && reclaimOk -> "BUG — use-after-free (per-call reclaim works)"
        else                -> "BUG — use-after-free and leak"
    }
    println("  verdict: $verdict")
}
