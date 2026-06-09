// Hand-written Kotlin/Wasm binding for the `wandr:fonts/loader@0.1.0`
// WIT import. wit-bindgen has no Kotlin generator
// (see [[wit-bindgen-no-kotlin-generator]]); modeled on the existing
// MarkdownImports / EmojiImports lifts.
//
// Task 41 — see tasks/41-system-fonts.md.
//
// Only `list-all() -> list<font-info>` is bound today. `load(family,
// style) -> option<list<u8>>` is on the WIT contract but unused by
// MarkdownCard — the host renders fonts directly via /system/fonts/
// paths in `canvas_impl.rs::family_alias_paths`. Add a binding for
// `load` when a consumer actually wants raw bytes.
//
// font-info record (24 bytes, align 4): three strings @ 8 bytes each
// (family, style, path).

@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
    kotlin.wasm.ExperimentalWasmInterop::class,
)

package testapp.fonts

import kotlin.wasm.*
import kotlin.wasm.unsafe.*

data class FontInfo(val family: String, val style: String, val path: String)

@WasmImport("wandr:fonts/loader@0.1.0", "list-all")
private external fun __wasm_import_fonts_list_all(returnAreaPtr: Int)

private const val FONT_INFO_SIZE = 24  // 3 strings × 8 bytes, align 4

fun listAllFonts(): List<FontInfo> = withScopedMemoryAllocator { alloc ->
    val retArea = alloc.allocate(8).address.toInt()
    __wasm_import_fonts_list_all(retArea)
    val ptr = retArea.loadI32()
    val len = (retArea + 4).loadI32()
    List(len) { i ->
        val base = ptr + i * FONT_INFO_SIZE
        FontInfo(
            family = liftString(base),
            style  = liftString(base + 8),
            path   = liftString(base + 16),
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
