// Hand-written Kotlin/Wasm binding for the `war:emoji/picker@0.1.0`
// WIT import. wit-bindgen has no Kotlin generator
// (see [[wit-bindgen-no-kotlin-generator]]); modeled on
// `MarkdownImports.kt`'s full-tree lift pattern.
//
// Task 40 — see tasks/40-emoji-picker.md.
//
// Canonical-ABI shapes:
//   `list-all() -> list<emoji>` lowers on the IMPORT side to
//      params: (return_area_ptr)
//      return area: 8 bytes (list = ptr+len)
//   `emoji = record { glyph: string, name: string, category: string }`
//   = three strings @ 8 bytes each, align 4 → 24 bytes per record.

@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
    kotlin.wasm.ExperimentalWasmInterop::class,
)

package testapp.emoji

import kotlin.wasm.*
import kotlin.wasm.unsafe.*

data class Emoji(val glyph: String, val name: String, val category: String)

@WasmImport("war:emoji/picker@0.1.0", "list-all")
private external fun __wasm_import_emoji_list_all(returnAreaPtr: Int)

private const val EMOJI_RECORD_SIZE = 24  // 3 strings × 8 bytes, align 4

/// Call `list-all()` and lift the returned list<emoji> into Kotlin
/// data classes. Memory backing the lifted strings stays in the dep's
/// realloc allocator until the next WIT import call frees it; we copy
/// each string out via decodeToString() so the Kotlin objects don't
/// hold stale pointers.
fun listAllEmojis(): List<Emoji> = withScopedMemoryAllocator { alloc ->
    val retArea = alloc.allocate(8).address.toInt()
    __wasm_import_emoji_list_all(retArea)
    val ptr = retArea.loadI32()
    val len = (retArea + 4).loadI32()
    List(len) { i ->
        val base = ptr + i * EMOJI_RECORD_SIZE
        Emoji(
            glyph = liftString(base),
            name = liftString(base + 8),
            category = liftString(base + 16),
        )
    }
}

private fun liftString(stringFieldBase: Int): String {
    val ptr = stringFieldBase.loadI32()
    val len = (stringFieldBase + 4).loadI32()
    if (len == 0) return ""
    val bytes = ByteArray(len)
    for (i in 0 until len) {
        bytes[i] = Pointer((ptr + i).toUInt()).loadByte()
    }
    return bytes.decodeToString()
}

private fun Int.loadI32(): Int = Pointer(this.toUInt()).loadInt()
