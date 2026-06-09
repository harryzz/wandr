// Hand-written Kotlin/Wasm bindings for the `wandr:markdown/renderer@0.1.0`
// WIT import. wit-bindgen 0.53.1 doesn't ship a Kotlin generator
// (despite skiko's headers claiming otherwise — that fork is private),
// so we author this by hand. Modeled on skiko's ComponentSupport.kt.
//
// Surface: just enough to call `render(source: string) -> document`
// from `main()` and read `blocks.len` from the returned record. The
// rest of the document tree is left in linear memory and ignored —
// the consumer is a one-shot smoke that exits after printing.
//
// Task 36 step 6 — see `tasks/36-cross-app-deps.md`.

@file:OptIn(UnsafeWasmMemoryApi::class, ComponentModelInternalApi::class)

package wandr.mdSmoke.markdown

import kotlin.wasm.*
import kotlin.wasm.unsafe.*

/// Canonical-ABI lowering of `render(source: string) -> document` on
/// the *import* side (caller-allocated return area). Asymmetric from
/// the export side, where the function returns a single i32 pointer
/// to a callee-allocated area:
///
///   exporter signature: `(param i32 i32) (result i32)`
///   importer signature: `(param i32 i32 i32) (result )`
///
/// Params: (source_ptr, source_len, return_area_ptr). The return area
/// is 8 bytes (a list lowers to ptr + len, both i32):
///   offset 0..3: blocks.ptr  (i32)
///   offset 4..7: blocks.len  (i32)
/// The component-model linker layer in wandr-host bridges the two.
@WasmImport("wandr:markdown/renderer@0.1.0", "render")
private external fun __wasm_import_render(
    sourcePtr: Int, sourceLen: Int, returnAreaPtr: Int,
)

/// `cabi_realloc` is the canonical-ABI's allocator hook. The dep calls
/// this to allocate space in our linear memory for the return value's
/// inner allocations (the `blocks` list and its nested data). Kotlin
/// stdlib provides `componentModelRealloc`; we just re-export it under
/// the canonical name. Must live in the application (not a library
/// klib) so it survives DCE.
@WasmExport
fun cabi_realloc(ptr: Int, oldSize: Int, align: Int, newSize: Int): Int =
    componentModelRealloc(ptr, oldSize, newSize)

/// Result of [render] — just the field we actually read in the smoke.
data class RenderResult(val returnAreaPtr: Int, val blocksLen: Int)

/// Lower the `source` string into linear memory, allocate the 8-byte
/// return area, call the dep's `render`, and read `blocks.len` from
/// offset 4. The dep-allocated `blocks` array and nested structures
/// stay in linear memory — left untouched (smoke exits).
fun render(source: String): RenderResult = withScopedMemoryAllocator { alloc ->
    val bytes = source.encodeToByteArray()
    val srcPtr = writeBytes(alloc, bytes)
    val retArea = alloc.allocate(8).address.toInt()   // 8-byte record (ptr+len)
    __wasm_import_render(srcPtr, bytes.size, retArea)
    val blocksLen = (retArea + 4).ptr.loadInt()
    RenderResult(retArea, blocksLen)
}

private fun writeBytes(alloc: MemoryAllocator, bytes: ByteArray): Int {
    val pointer = alloc.allocate(bytes.size)
    var cur = pointer
    bytes.forEach { cur.storeByte(it); cur += 1 }
    return pointer.address.toInt()
}

private val Int.ptr: Pointer get() = Pointer(this.toUInt())
