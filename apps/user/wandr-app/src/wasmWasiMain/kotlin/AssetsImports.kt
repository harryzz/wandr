// Hand-written Kotlin/Wasm binding for the `my:skiko-gfx/assets@0.1.0`
// WIT import. wit-bindgen has no Kotlin generator (see
// [[wit-bindgen-no-kotlin-generator]]), and Kotlin/Wasm's stdlib has
// no filesystem APIs (no fd_read / path_open wrappers as of Kotlin
// 2.4), so this hand-written host-verb is the practical path for
// reading bundled data files.
//
// Task 38 — see tasks/38-wandrpkg-assets.md.

@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
    kotlin.wasm.ExperimentalWasmInterop::class,
)

package testapp.assets

import kotlin.wasm.*
import kotlin.wasm.unsafe.*

/// `read(name: string) -> option<list<u8>>`. Importer-side canonical
/// ABI: caller-allocated return area.
///   params: (name_ptr, name_len, return_area_ptr)
///   return area: 12 bytes (option discriminant + padding + list[ptr+len])
///     [0] disc (u8): 0 = none, 1 = some
///     [4..8] list.ptr (i32)
///     [8..12] list.len (i32)
@WasmImport("my:skiko-gfx/assets@0.1.0", "read")
private external fun __wasm_import_assets_read(
    namePtr: Int, nameLen: Int, returnAreaPtr: Int,
)

/// Read a file from the bundle's `assets/` dir. Returns `null` for
/// missing/unsafe/io-error (host logs the cause). `name` is relative
/// to the assets root; `..` traversal is rejected host-side.
fun readAsset(name: String): ByteArray? = withScopedMemoryAllocator { alloc ->
    val nameBytes = name.encodeToByteArray()
    val namePtr = writeBytes(alloc, nameBytes)
    val retArea = alloc.allocate(12).address.toInt()
    __wasm_import_assets_read(namePtr, nameBytes.size, retArea)
    val disc = Pointer(retArea.toUInt()).loadByte().toInt() and 0xFF
    if (disc == 0) {
        null
    } else {
        val ptr = Pointer((retArea + 4).toUInt()).loadInt()
        val len = Pointer((retArea + 8).toUInt()).loadInt()
        val out = ByteArray(len)
        for (i in 0 until len) {
            out[i] = Pointer((ptr + i).toUInt()).loadByte()
        }
        out
    }
}

private fun writeBytes(alloc: MemoryAllocator, bytes: ByteArray): Int {
    val pointer = alloc.allocate(bytes.size)
    var cur = pointer
    bytes.forEach { cur.storeByte(it); cur += 1 }
    return pointer.address.toInt()
}
