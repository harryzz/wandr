// Hand-written Kotlin/Wasm binding for the `war:markdown/renderer@0.1.0`
// WIT import — full document-tree lift (task 36 visuals v2). Walks
// every block + run + style synchronously inside the import's scoped
// allocator, returns immutable Kotlin data classes the Compose card
// then renders as proper styled text.
//
// Canonical-ABI layouts (component-model spec): every record is align
// to its largest field; every variant is `disc_byte + padding +
// max_payload`; every list is `ptr + len` (8 bytes); every string is
// `ptr + len` (8 bytes); every option<T> is `disc + padding + T`.
//
// Layout sizes (all align 4):
//   run                 = 28  (text@0 string, styles@8 list, link-url@16 option<string>)
//   list<run>           =  8  (ptr+len)
//   heading-block       = 12  (level@0 u8, runs@4 list)
//   code-block          = 20  (language@0 option<string>, text@12 string)
//   list-item           =  8  (blocks@0 list<simple-block>)
//   ordered-list-block  = 12  (start@0 u32, items@4 list)
//   simple-block        = 24  (disc@0 u8, payload@4..max 20 — code-block)
//   block               = 24  (disc@0 u8, payload@4..max 20 — code-block)
//   option<string>      = 12  (disc@0 u8, ptr@4, len@8)
//
// Block discriminants (WIT case order):
//   0 paragraph(list<run>)         3 bullet-list(list<list-item>)
//   1 heading(heading-block)       4 ordered-list(ordered-list-block)
//   2 code-block(code-block)       5 block-quote(list<simple-block>)
//                                  6 thematic-break
//
// Simple-block discriminants:
//   0 paragraph  1 heading  2 code-block  3 thematic-break
//
// Inline-style enum (u8): 0 emphasis, 1 strong, 2 code.

@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
    kotlin.wasm.ExperimentalWasmInterop::class,
)

package testapp.markdown

import kotlin.wasm.*
import kotlin.wasm.unsafe.*

// ── Lifted Kotlin types ──────────────────────────────────────────────

data class Document(val blocks: List<Block>)

sealed interface Block {
    data class Paragraph(val runs: List<Run>) : Block
    data class Heading(val level: Int, val runs: List<Run>) : Block
    data class CodeBlock(val language: String?, val text: String) : Block
    data class BulletList(val items: List<MdListItem>) : Block
    data class OrderedList(val start: Int, val items: List<MdListItem>) : Block
    data class BlockQuote(val blocks: List<SimpleBlock>) : Block
    data object ThematicBreak : Block
}

sealed interface SimpleBlock {
    data class Paragraph(val runs: List<Run>) : SimpleBlock
    data class Heading(val level: Int, val runs: List<Run>) : SimpleBlock
    data class CodeBlock(val language: String?, val text: String) : SimpleBlock
    data object ThematicBreak : SimpleBlock
}

data class MdListItem(val blocks: List<SimpleBlock>)

enum class InlineStyle { Emphasis, Strong, Code }

data class Run(
    val text: String,
    val styles: Set<InlineStyle>,
    val linkUrl: String?,
)

// ── WIT import + lift entry point ────────────────────────────────────

@WasmImport("war:markdown/renderer@0.1.0", "render")
private external fun __wasm_import_render(
    sourcePtr: Int, sourceLen: Int, returnAreaPtr: Int,
)

/// Call the cross-app `render` import and walk the entire returned
/// document tree synchronously. Returns immutable Kotlin data classes;
/// the dep-allocated linear memory is no longer needed after this fn.
fun renderDocument(source: String): Document = withScopedMemoryAllocator { alloc ->
    val bytes = source.encodeToByteArray()
    val srcPtr = writeBytes(alloc, bytes)
    val retArea = alloc.allocate(8).address.toInt()
    __wasm_import_render(srcPtr, bytes.size, retArea)
    // retArea layout: [0..4] blocks.ptr, [4..8] blocks.len
    val blocksPtr = retArea.loadI32()
    val blocksLen = (retArea + 4).loadI32()
    Document(blocks = liftListBlocks(blocksPtr, blocksLen))
}

// ── Block-level lifts ────────────────────────────────────────────────

private const val BLOCK_SIZE = 24
private const val SIMPLE_BLOCK_SIZE = 24
private const val RUN_SIZE = 28
private const val HEADING_BLOCK_SIZE = 12
private const val CODE_BLOCK_SIZE = 20
private const val LIST_ITEM_SIZE = 8
private const val ORDERED_LIST_BLOCK_SIZE = 12

private fun liftListBlocks(ptr: Int, len: Int): List<Block> =
    List(len) { i -> liftBlock(ptr + i * BLOCK_SIZE) }

private fun liftBlock(base: Int): Block {
    val disc = base.loadU8()
    val payload = base + 4 // padding to align 4
    return when (disc) {
        0 -> Block.Paragraph(runs = liftRunListAt(payload))
        1 -> liftHeadingBlock(payload).let { Block.Heading(it.first, it.second) }
        2 -> liftCodeBlock(payload).let { Block.CodeBlock(it.first, it.second) }
        3 -> Block.BulletList(items = liftListItemListAt(payload))
        4 -> {
            val start = payload.loadI32()
            val items = liftListItemListAt(payload + 4)
            Block.OrderedList(start = start, items = items)
        }
        5 -> Block.BlockQuote(blocks = liftSimpleBlockListAt(payload))
        6 -> Block.ThematicBreak
        else -> error("liftBlock: unknown discriminant $disc")
    }
}

private fun liftListSimpleBlocks(ptr: Int, len: Int): List<SimpleBlock> =
    List(len) { i -> liftSimpleBlock(ptr + i * SIMPLE_BLOCK_SIZE) }

private fun liftSimpleBlock(base: Int): SimpleBlock {
    val disc = base.loadU8()
    val payload = base + 4
    return when (disc) {
        0 -> SimpleBlock.Paragraph(runs = liftRunListAt(payload))
        1 -> liftHeadingBlock(payload).let { SimpleBlock.Heading(it.first, it.second) }
        2 -> liftCodeBlock(payload).let { SimpleBlock.CodeBlock(it.first, it.second) }
        3 -> SimpleBlock.ThematicBreak
        else -> error("liftSimpleBlock: unknown discriminant $disc")
    }
}

// ── Record lifts (return tuples to avoid duplicating Block.X / SimpleBlock.X) ──

/// Returns (level, runs). Caller wraps into Block.Heading or SimpleBlock.Heading.
private fun liftHeadingBlock(base: Int): Pair<Int, List<Run>> {
    val level = base.loadU8()
    val runs = liftRunListAt(base + 4) // align to 4 after u8
    return level to runs
}

/// Returns (language, text). Caller wraps into Block.CodeBlock or SimpleBlock.CodeBlock.
private fun liftCodeBlock(base: Int): Pair<String?, String> {
    val language = liftOptionString(base)        // option<string> @ 0..12
    val text = liftString(base + 12)             // string @ 12..20
    return language to text
}

private fun liftListItem(base: Int): MdListItem =
    MdListItem(blocks = liftSimpleBlockListAt(base))

// ── Run + inline-style ────────────────────────────────────────────────

private fun liftRunListAt(listFieldBase: Int): List<Run> {
    val ptr = listFieldBase.loadI32()
    val len = (listFieldBase + 4).loadI32()
    return List(len) { i -> liftRun(ptr + i * RUN_SIZE) }
}

private fun liftRun(base: Int): Run {
    val text = liftString(base)                  // string @ 0..8
    val stylesPtr = (base + 8).loadI32()
    val stylesLen = (base + 12).loadI32()
    val styles = mutableSetOf<InlineStyle>()
    for (i in 0 until stylesLen) {
        when ((stylesPtr + i).loadU8()) {
            0 -> styles += InlineStyle.Emphasis
            1 -> styles += InlineStyle.Strong
            2 -> styles += InlineStyle.Code
            else -> { /* unknown style — drop */ }
        }
    }
    val linkUrl = liftOptionString(base + 16)    // option<string> @ 16..28
    return Run(text = text, styles = styles, linkUrl = linkUrl)
}

private fun liftSimpleBlockListAt(listFieldBase: Int): List<SimpleBlock> {
    val ptr = listFieldBase.loadI32()
    val len = (listFieldBase + 4).loadI32()
    return liftListSimpleBlocks(ptr, len)
}

private fun liftListItemListAt(listFieldBase: Int): List<MdListItem> {
    val ptr = listFieldBase.loadI32()
    val len = (listFieldBase + 4).loadI32()
    return List(len) { i -> liftListItem(ptr + i * LIST_ITEM_SIZE) }
}

// ── Primitives ───────────────────────────────────────────────────────

/// option<T> layout: disc@0 (u8), padding, payload@align(T).
/// For option<string>: disc@0, ptr@4, len@8 — total 12 bytes.
private fun liftOptionString(base: Int): String? {
    val disc = base.loadU8()
    return if (disc == 0) null else liftString(base + 4)
}

/// string lift: read [ptr, len) bytes as UTF-8.
private fun liftString(stringFieldBase: Int): String {
    val ptr = stringFieldBase.loadI32()
    val len = (stringFieldBase + 4).loadI32()
    if (len == 0) return ""
    val bytes = ByteArray(len)
    for (i in 0 until len) {
        bytes[i] = (ptr + i).loadI8()
    }
    return bytes.decodeToString()
}

// Pointer helpers — `loadI32` etc. on a raw Int address. Saves typing
// `(addr).ptr.loadInt()` everywhere.
private fun Int.loadI32(): Int   = Pointer(this.toUInt()).loadInt()
private fun Int.loadI8(): Byte   = Pointer(this.toUInt()).loadByte()
private fun Int.loadU8(): Int    = Pointer(this.toUInt()).loadByte().toInt() and 0xFF

private fun writeBytes(alloc: MemoryAllocator, bytes: ByteArray): Int {
    val pointer = alloc.allocate(bytes.size)
    var cur = pointer
    bytes.forEach { cur.storeByte(it); cur += 1 }
    return pointer.address.toInt()
}
