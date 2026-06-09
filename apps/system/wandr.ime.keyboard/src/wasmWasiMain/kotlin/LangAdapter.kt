// Language plugin adapter (task 49 step 5). Calls each declared
// `wandr.lang.*` plugin's exported `lang.get-info` + `lang.get-layout`
// once at IME startup, lifts the returned data through the canonical
// ABI, and converts each plugin into a `KeyboardLayout` ready for the
// 🌐-cycle. Mirrors the lift recipe from
// `wandr-app/src/wasmWasiMain/kotlin/EmojiImports.kt` /
// `MarkdownImports.kt` — wit-bindgen has no Kotlin generator
// (see `feedback_wit_bindgen_no_kotlin_generator`), so the bindings
// are hand-rolled.
//
// **MVP shape**: the IME hardcodes the set of known plugins (bg, fr)
// here. Each lang plugin uses its own WIT package name
// (`wandr:keyboard-lang-bg`, `wandr:keyboard-lang-fr`) because two
// dependencies cannot share the same `linker.instance(name)` entry —
// `wire_dep_into_linker` would collide. Adding a new language → add
// a `package wandr:keyboard-lang-<id>` WIT, add an `import` line in
// `wit/wandr-ime-keyboard.wit`, and append a `Loader` entry here.
//
// Canonical-ABI shapes:
//   info       = 20 bytes  (name@0 string=8, locale@8 string=8,
//                           is-rtl@16 u8, +3 pad to align 4)
//   key-def    = 20 bytes  (display@0 string=8, code-point@8 u32,
//                           key-id@12 u32, width@16 f32)
//   list<T>    =  8 bytes  (ptr+len)
//   string     =  8 bytes  (ptr+len)
//
//   get-info()        -> info   →  import sig: (retArea: i32)
//   get-layout(bool)  -> layout →  import sig: (shifted: i32, retArea: i32)
//                                  layout = { rows: list<key-row> } @ 8 B
//                                  key-row = list<key-def>           @ 8 B

@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
    kotlin.wasm.ExperimentalWasmInterop::class,
)

package testapp

import kotlin.wasm.*
import kotlin.wasm.unsafe.*
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

// ── Lifted Kotlin types ──────────────────────────────────────────────

data class LangInfo(val name: String, val locale: String, val isRtl: Boolean)
data class LangKeyDef(
    val display:   String,
    val codePoint: Int,
    val keyId:     Int,
    val width:     Float,
)

// ── @WasmImport declarations (one set per known plugin) ──────────────

@WasmImport("wandr:keyboard-lang-bg/lang@0.1.0", "get-info")
private external fun __bg_get_info(retArea: Int)
@WasmImport("wandr:keyboard-lang-bg/lang@0.1.0", "get-layout")
private external fun __bg_get_layout(shifted: Int, retArea: Int)

@WasmImport("wandr:keyboard-lang-fr/lang@0.1.0", "get-info")
private external fun __fr_get_info(retArea: Int)
@WasmImport("wandr:keyboard-lang-fr/lang@0.1.0", "get-layout")
private external fun __fr_get_layout(shifted: Int, retArea: Int)

// ── Canonical-ABI lift helpers ───────────────────────────────────────

private const val INFO_SIZE   = 20
private const val LAYOUT_SIZE = 8
private const val KEYDEF_SIZE = 20

private fun liftInfoVia(call: (Int) -> Unit): LangInfo {
    // Required leading freeAll — without it the prior frame's
    // realloc-allocated memory is still live and `withScopedMemoryAllocator`
    // throws `Can't create new allocators while realloc-allocated
    // memory is not freed`. See [[wasi-realloc-allocator-pollution]].
    freeAllComponentModelReallocAllocatedMemory()
    return withScopedMemoryAllocator { alloc ->
        val retArea = alloc.allocate(INFO_SIZE).address.toInt()
        call(retArea)
        val info = LangInfo(
            name   = liftString(retArea),
            locale = liftString(retArea + 8),
            isRtl  = Pointer((retArea + 16).toUInt()).loadByte().toInt() != 0,
        )
        freeAllComponentModelReallocAllocatedMemory()
        info
    }
}

private fun liftLayoutVia(call: (Int) -> Unit): List<List<LangKeyDef>> {
    freeAllComponentModelReallocAllocatedMemory()
    return withScopedMemoryAllocator { alloc ->
        val retArea = alloc.allocate(LAYOUT_SIZE).address.toInt()
        call(retArea)
        val rowsPtr = retArea.loadI32()
        val rowsLen = (retArea + 4).loadI32()
        val rows = List(rowsLen) { i ->
            val rowField = rowsPtr + i * 8
            val keysPtr  = rowField.loadI32()
            val keysLen  = (rowField + 4).loadI32()
            List(keysLen) { j ->
                val base = keysPtr + j * KEYDEF_SIZE
                LangKeyDef(
                    display   = liftString(base),
                    codePoint = (base + 8).loadI32(),
                    keyId     = (base + 12).loadI32(),
                    width     = Float.fromBits((base + 16).loadI32()),
                )
            }
        }
        freeAllComponentModelReallocAllocatedMemory()
        rows
    }
}

private fun liftString(base: Int): String {
    val ptr = base.loadI32()
    val len = (base + 4).loadI32()
    if (len == 0) return ""
    val bytes = ByteArray(len)
    for (i in 0 until len) bytes[i] = Pointer((ptr + i).toUInt()).loadByte()
    return bytes.decodeToString()
}

private fun Int.loadI32(): Int = Pointer(this.toUInt()).loadInt()

// ── Public bridge ────────────────────────────────────────────────────

object LangAdapter {

    /** Static registry of known plugins. Order matters: defines the
     *  🌐-cycle sequence after the built-in English layout. */
    private val plugins: List<Loader> = listOf(
        Loader(
            id        = "bg",
            getInfo   = { liftInfoVia(::__bg_get_info) },
            getLayout = { shifted -> liftLayoutVia { __bg_get_layout(if (shifted) 1 else 0, it) } },
        ),
        Loader(
            id        = "fr",
            getInfo   = { liftInfoVia(::__fr_get_info) },
            getLayout = { shifted -> liftLayoutVia { __fr_get_layout(if (shifted) 1 else 0, it) } },
        ),
    )

    /** Called once at IME composition. Returns one `KeyboardLayout`
     *  per successfully-loaded plugin (failures swallowed + logged so
     *  one broken plugin doesn't kill the whole IME). */
    fun loadAllLangPlugins(): List<KeyboardLayout> {
        val loaded = plugins.mapNotNull { it.toKeyboardLayout() }
        WitCanvas.Import.logMessage(
            "LangAdapter: loaded ${loaded.size} plugin(s): " +
                loaded.joinToString { it.name }
        )
        return loaded
    }

    private data class Loader(
        val id:        String,
        val getInfo:   () -> LangInfo,
        val getLayout: (Boolean) -> List<List<LangKeyDef>>,
    ) {
        fun toKeyboardLayout(): KeyboardLayout? = runCatching {
            val info     = getInfo()
            val unshift  = getLayout(false)
            val shifted  = getLayout(true)
            ImeKeyboardDefaults.wrapLanguageLayout(
                name              = info.name,
                letterRows        = unshift.map { row -> row.map { it.toKeyDef() } },
                shiftedLetterRows = shifted.map { row -> row.map { it.toKeyDef() } },
            )
        }.getOrElse { t ->
            WitCanvas.Import.logMessage(
                "LangAdapter: plugin '$id' failed: ${t::class.simpleName} ${t.message}"
            )
            null
        }
    }

    private fun LangKeyDef.toKeyDef(): KeyDef = KeyDef(
        display = display,
        action  = KeyAction.Send(codePoint, keyId),
        width   = if (width <= 0f) 1f else width,
    )
}
