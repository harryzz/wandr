// Export-record spike guest: receives `wandr:spike/handler.key-event`
// (record with TWO strings) from the host and returns a checksum the host
// verifies. The host lowered both strings into OUR linear memory via
// cabi_realloc before either export body runs — the historically-unsafe
// host→guest direction (feedback_wasi_cabi_realloc_export_block).
//
// `onKey` uses the official Kotlin/wit-bindgen ordering (freeAll → lift →
// scoped allocations). `onKeyLateLift` deliberately scribbles via the scoped
// allocator BEFORE lifting — the positive control: if arg memory is reused
// after freeAll, this one must corrupt.

@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.ExperimentalWasmInterop::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
)

import kotlin.wasm.WasmExport
import kotlin.wasm.unsafe.*

@WasmExport
fun cabi_realloc(ptr: Int, oldSize: Int, align: Int, newSize: Int): Int =
    componentModelRealloc(ptr, oldSize, newSize)

private fun loadBytes(addr: Int, len: Int): ByteArray {
    val p = Pointer(addr.toUInt())
    return ByteArray(len) { i -> (p + i).loadByte() }
}

private fun fnv1a(acc0: UInt, bytes: ByteArray): UInt {
    var acc = acc0
    for (b in bytes) {
        acc = (acc xor b.toUByte().toUInt()) * 16777619u
    }
    return acc
}

private fun checksum(code: ByteArray, text: ByteArray, mods: Int, repeatFlag: Int): Int {
    var h = 2166136261u
    h = fnv1a(h, code)
    h = fnv1a(h, text)
    h = fnv1a(
        h,
        byteArrayOf(
            (mods and 0xFF).toByte(),
            ((mods shr 8) and 0xFF).toByte(),
            ((mods shr 16) and 0xFF).toByte(),
            ((mods ushr 24) and 0xFF).toByte(),
        ),
    )
    h = fnv1a(h, byteArrayOf(if (repeatFlag != 0) 1 else 0))
    return h.toInt()
}

/// Scribble a sentinel over a fresh scoped allocation the same size as the
/// (just-freed) realloc region holding the args — if that region is reused
/// by the scoped allocator, this overwrites the argument strings.
private fun scribble(size: Int) {
    withScopedMemoryAllocator { a ->
        val n = size + 64
        var p = a.allocate(n)
        var i = 0
        while (i < n) {
            p.storeByte(0x5A)
            p += 1
            i += 1
        }
    }
}

// record key-event { code: string, text: string, mods: u32, repeat: bool }
// flattens to (ptr,len, ptr,len, i32, i32); result u32 → single flat i32.

@WasmExport("wandr:spike/handler@0.1.0#on-key")
fun onKey(codePtr: Int, codeLen: Int, textPtr: Int, textLen: Int, mods: Int, repeatFlag: Int): Int {
    freeAllComponentModelReallocAllocatedMemory()
    val code = loadBytes(codePtr, codeLen)   // lift FIRST (pure reads)
    val text = loadBytes(textPtr, textLen)
    scribble(codeLen + textLen)              // then scoped-alloc hammering
    return checksum(code, text, mods, repeatFlag)
}

@WasmExport("wandr:spike/handler@0.1.0#on-key-late-lift")
fun onKeyLateLift(codePtr: Int, codeLen: Int, textPtr: Int, textLen: Int, mods: Int, repeatFlag: Int): Int {
    freeAllComponentModelReallocAllocatedMemory()
    scribble(codeLen + textLen)              // WRONG order: scribble first…
    val code = loadBytes(codePtr, codeLen)   // …then lift (positive control)
    val text = loadBytes(textPtr, textLen)
    return checksum(code, text, mods, repeatFlag)
}

fun main() {
    // Never invoked — the component is used as a reactor (handler exports
    // only); binaries.executable() just needs an entry point to compile.
}
