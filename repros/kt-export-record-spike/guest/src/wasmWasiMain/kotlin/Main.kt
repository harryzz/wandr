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

// ── indirect-args spill case ──────────────────────────────────────────────
// big-event flattens to 19 core params (> MAX_FLAT_PARAMS=16): the host
// passes ONE i32 pointer to an args area it cabi_realloc'd in OUR memory.
// Canonical-ABI layout of the area (single record arg):
//   0..63   8 × (ptr: u32, len: u32)   the strings
//   64      mods: u32
//   72      ts: u64                    (align 8)
//   80      repeat: u8

private fun longLeBytes(v: Long): ByteArray =
    ByteArray(8) { i -> ((v ushr (8 * i)) and 0xFF).toByte() }

private fun intLeBytes(v: Int): ByteArray =
    ByteArray(4) { i -> ((v ushr (8 * i)) and 0xFF).toByte() }

private fun liftBig(argsPtr: Int): Pair<Array<ByteArray>, Triple<Int, Long, Int>> {
    val base = Pointer(argsPtr.toUInt())
    val strings = Array(8) { i ->
        val ptr = (base + i * 8).loadInt()
        val len = (base + i * 8 + 4).loadInt()
        loadBytes(ptr, len)
    }
    val mods = (base + 64).loadInt()
    val ts = (base + 72).loadLong()
    val repeatFlag = (base + 80).loadByte().toInt() and 0xFF
    return Pair(strings, Triple(mods, ts, repeatFlag))
}

private fun bigChecksum(strings: Array<ByteArray>, mods: Int, ts: Long, repeatFlag: Int): Int {
    var h = 2166136261u
    for (s in strings) h = fnv1a(h, s)
    h = fnv1a(h, intLeBytes(mods))
    h = fnv1a(h, longLeBytes(ts))
    h = fnv1a(h, byteArrayOf(if (repeatFlag != 0) 1 else 0))
    return h.toInt()
}

private fun totalLen(strings: Array<ByteArray>): Int {
    var n = 0
    for (s in strings) n += s.size
    return n
}

@WasmExport("wandr:spike/handler@0.1.0#on-big")
fun onBig(argsPtr: Int): Int {
    freeAllComponentModelReallocAllocatedMemory()
    val (strings, rest) = liftBig(argsPtr)       // lift FIRST (pure reads)
    scribble(totalLen(strings) + 96)             // then scoped-alloc hammering
    return bigChecksum(strings, rest.first, rest.second, rest.third)
}

@WasmExport("wandr:spike/handler@0.1.0#on-big-late-lift")
fun onBigLateLift(argsPtr: Int): Int {
    // WRONG order, kept trap-free: read the ptr/len table + scalars first
    // (so the pointers stay sane), scribble, THEN read the string bytes —
    // models a binding that interleaves allocation with arg copying. A
    // scribble before even the table read would corrupt the pointers
    // themselves and trap on a wild load; same bug, louder failure.
    freeAllComponentModelReallocAllocatedMemory()
    val base = Pointer(argsPtr.toUInt())
    val ptrs = IntArray(8) { i -> (base + i * 8).loadInt() }
    val lens = IntArray(8) { i -> (base + i * 8 + 4).loadInt() }
    val mods = (base + 64).loadInt()
    val ts = (base + 72).loadLong()
    val repeatFlag = (base + 80).loadByte().toInt() and 0xFF
    var n = 96
    for (l in lens) n += l
    scribble(n)                                   // clobber area + string bytes
    val strings = Array(8) { i -> loadBytes(ptrs[i], lens[i]) }  // LATE reads
    return bigChecksum(strings, mods, ts, repeatFlag)
}

fun main() {
    // Never invoked — the component is used as a reactor (handler exports
    // only); binaries.executable() just needs an entry point to compile.
}
