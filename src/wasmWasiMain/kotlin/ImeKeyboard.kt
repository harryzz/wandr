// war.ime.keyboard — task 47 step 3c follow-up. Port of wart-app's
// WasiSoftKeyboard to the dedicated-guest IME, swapping the
// onKey(KeyEvent) callback for direct Keyboard.Import.sendKeyEvent
// calls (the IME's outbound WIT path → arbiter → focused-pid).
//
// Five layouts: English QWERTY (+ Shifted), Bulgarian Cyrillic
// (+ Shifted), Symbols, Symbols2, Emoji. Shift + layout-cycle +
// switch-layout all work. Autorepeat omitted (parent file's
// comment documents why the original disabled it).
//
// The "Hide" key is currently a logged no-op — step 4 will wire it
// to a new "request-hide" WIT verb that forwards to the arbiter's
// overlay-clear. For now the IME stays up until the focused editor
// loses focus (auto-tied via detach-editor in step 3c).

@file:OptIn(androidx.compose.ui.InternalComposeUiApi::class)

package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas
import org.jetbrains.skiko.wasi.wit.Keyboard as WitKeyboard

// ─── AKEYCODE_* constants ────────────────────────────────────────────
//
// Mapped from frameworks/base/core/java/android/view/KeyEvent.java.
// Used as the `key-id` arg to sendKeyEvent so wart-host's
// dispatch_key_v2 sees the same keycode a hardware key press would
// generate. Printable characters that don't have a clean AKEYCODE_*
// equivalent fall back to keyId=0; codePoint carries the actual char.

private const val AKEYCODE_0:        Int = 7
private const val AKEYCODE_A:        Int = 29
private const val AKEYCODE_DEL:      Int = 67   // backspace
private const val AKEYCODE_ENTER:    Int = 66
private const val AKEYCODE_SPACE:    Int = 62
private const val AKEYCODE_COMMA:    Int = 55
private const val AKEYCODE_PERIOD:   Int = 56

/** Map a printable char to a sensible AKEYCODE; 0 for "no specific keycode". */
private fun akeycodeFor(c: Char): Int = when (c) {
    in 'a'..'z' -> AKEYCODE_A + (c - 'a')
    in 'A'..'Z' -> AKEYCODE_A + (c - 'A')
    in '0'..'9' -> AKEYCODE_0 + (c - '0')
    ' '         -> AKEYCODE_SPACE
    ','         -> AKEYCODE_COMMA
    '.'         -> AKEYCODE_PERIOD
    else        -> 0
}

/** Fire-and-forget a down + up pair via the IME WIT verb. */
private fun sendKey(codePoint: Int, keyId: Int) {
    WitKeyboard.Import.sendKeyEvent(codePoint.toUInt(), keyId.toUInt(), 0u) // down
    WitKeyboard.Import.sendKeyEvent(codePoint.toUInt(), keyId.toUInt(), 1u) // up
}

// ─── Layout model ────────────────────────────────────────────────────

/** Width weight for a key relative to the other keys in its row. */
typealias KeyWidth = Float

/** What pressing one key emits + which layout-switch it triggers, if any. */
sealed interface KeyAction {
    /** Send a key event (text input). `codePoint` is the actual Unicode
     *  scalar; `keyId` is the AKEYCODE_* the host should treat this as.
     *  For printable characters with no specific AKEYCODE, keyId = 0
     *  and the host falls back to codePoint-only insertion. */
    data class Send(val codePoint: Int, val keyId: Int) : KeyAction

    /** Toggle the shift state. */
    data object Shift : KeyAction

    /** Switch to a named layout. */
    data class SwitchLayout(val targetLayoutName: String) : KeyAction

    /** Cycle through the "language" layouts in declaration order (🌐). */
    data object CycleLanguage : KeyAction

    /** Hide the keyboard. Currently logged + no-op; step 4 wires to
     *  a request-hide WIT verb forwarding to arbiter overlay-clear. */
    data object Hide : KeyAction
}

/** One key definition. */
data class KeyDef(
    val display: String,
    val action: KeyAction,
    val width: KeyWidth = 1f,
)

/** A full keyboard layout. `shiftedRows` is optional; if null, shift is a no-op. */
data class KeyboardLayout(
    val name: String,
    /** Set false for symbol / emoji layouts so the 🌐 modifier skips them. */
    val isLanguage: Boolean = true,
    val rows: List<List<KeyDef>>,
    val shiftedRows: List<List<KeyDef>>? = null,
)

// ─── Built-in layouts ────────────────────────────────────────────────

object ImeKeyboardDefaults {

    /** Character codepoint of `c` plus a sensible AKEYCODE_*. */
    private fun letter(c: Char): KeyDef =
        KeyDef(c.toString(), KeyAction.Send(c.code, akeycodeFor(c)))

    /** Codepoint of the first character (handles surrogate-pair emoji). */
    private fun firstCodePoint(s: String): Int {
        val c0 = s[0]
        if (c0.isHighSurrogate() && s.length >= 2) {
            val c1 = s[1]
            if (c1.isLowSurrogate()) {
                return 0x10000 + ((c0.code - 0xD800) shl 10) + (c1.code - 0xDC00)
            }
        }
        return c0.code
    }

    /** Printable codepoint that isn't a standard letter — digits, punctuation, emoji. */
    private fun text(s: String, codePoint: Int = firstCodePoint(s)): KeyDef {
        val keyId = if (s.length == 1) akeycodeFor(s[0]) else 0
        return KeyDef(s, KeyAction.Send(codePoint, keyId))
    }

    private val backspace = KeyDef("⌫", KeyAction.Send(0, AKEYCODE_DEL),   width = 1.5f)
    private val space     = KeyDef(" ", KeyAction.Send(32, AKEYCODE_SPACE), width = 4.0f)
    private val enter     = KeyDef("⏎", KeyAction.Send(0, AKEYCODE_ENTER), width = 1.5f)
    private val hide      = KeyDef("⌄", KeyAction.Hide,                    width = 1f)

    /** Top digits row (1-9, 0) — shared across language layouts. */
    private val digitsRow: List<KeyDef> = listOf(
        "1","2","3","4","5","6","7","8","9","0",
    ).map { text(it) }

    /** English QWERTY (US layout). 5 rows. */
    val English: KeyboardLayout = KeyboardLayout(
        name = "English",
        rows = listOf(
            digitsRow,
            listOf("q","w","e","r","t","y","u","i","o","p").map { letter(it[0]) },
            listOf("a","s","d","f","g","h","j","k","l").map { letter(it[0]) },
            buildList {
                add(KeyDef("⇧", KeyAction.Shift, width = 1.5f))
                addAll(listOf("z","x","c","v","b","n","m").map { letter(it[0]) })
                add(backspace)
            },
            listOf(
                KeyDef("123",  KeyAction.SwitchLayout("Symbols"),  width = 1.3f),
                KeyDef("🌐",   KeyAction.CycleLanguage,            width = 1f),
                KeyDef("😀",   KeyAction.SwitchLayout("Emoji"),    width = 1f),
                space.copy(width = 3.4f),
                text("."),
                enter,
                hide,
            ),
        ),
        shiftedRows = listOf(
            digitsRow,
            listOf("Q","W","E","R","T","Y","U","I","O","P").map { letter(it[0]) },
            listOf("A","S","D","F","G","H","J","K","L").map { letter(it[0]) },
            buildList {
                add(KeyDef("⇧", KeyAction.Shift, width = 1.5f))
                addAll(listOf("Z","X","C","V","B","N","M").map { letter(it[0]) })
                add(backspace)
            },
            listOf(
                KeyDef("123",  KeyAction.SwitchLayout("Symbols"),  width = 1.3f),
                KeyDef("🌐",   KeyAction.CycleLanguage,            width = 1f),
                KeyDef("😀",   KeyAction.SwitchLayout("Emoji"),    width = 1f),
                space.copy(width = 3.4f),
                text(","),
                enter,
                hide,
            ),
        ),
    )

    /** Bulgarian Cyrillic — example of how to add another language. */
    val Bulgarian: KeyboardLayout = KeyboardLayout(
        name = "Български",
        rows = listOf(
            digitsRow,
            listOf("я","в","е","р","т","ъ","у","и","о","п").map { text(it) },
            listOf("а","с","д","ф","г","х","й","к","л").map { text(it) },
            buildList {
                add(KeyDef("⇧", KeyAction.Shift, width = 1.5f))
                addAll(listOf("з","ь","ц","ж","б","н","м").map { text(it) })
                add(backspace)
            },
            listOf(
                KeyDef("123",  KeyAction.SwitchLayout("Symbols"), width = 1.3f),
                KeyDef("🌐",   KeyAction.CycleLanguage,           width = 1f),
                KeyDef("😀",   KeyAction.SwitchLayout("Emoji"),   width = 1f),
                space.copy(width = 3.4f),
                text("."),
                enter,
                hide,
            ),
        ),
        shiftedRows = listOf(
            digitsRow,
            listOf("Я","В","Е","Р","Т","Ъ","У","И","О","П").map { text(it) },
            listOf("А","С","Д","Ф","Г","Х","Й","К","Л").map { text(it) },
            buildList {
                add(KeyDef("⇧", KeyAction.Shift, width = 1.5f))
                addAll(listOf("З","Ь","Ц","Ж","Б","Н","М").map { text(it) })
                add(backspace)
            },
            listOf(
                KeyDef("123",  KeyAction.SwitchLayout("Symbols"), width = 1.3f),
                KeyDef("🌐",   KeyAction.CycleLanguage,           width = 1f),
                KeyDef("😀",   KeyAction.SwitchLayout("Emoji"),   width = 1f),
                space.copy(width = 3.4f),
                text(","),
                enter,
                hide,
            ),
        ),
    )

    /** Digits + common punctuation. */
    val Symbols: KeyboardLayout = KeyboardLayout(
        name = "Symbols",
        isLanguage = false,
        rows = listOf(
            digitsRow,
            listOf("-","/",":",";","(",")","$","&","@","\"").map { text(it) },
            buildList {
                add(KeyDef("#+=", KeyAction.SwitchLayout("Symbols2"), width = 1.5f))
                addAll(listOf(".",",","?","!","'").map { text(it) })
                add(backspace)
            },
            listOf(
                KeyDef("ABC", KeyAction.SwitchLayout("English"), width = 1.5f),
                KeyDef("🌐",  KeyAction.CycleLanguage,           width = 1f),
                space.copy(width = 4f),
                enter,
                hide,
            ),
        ),
    )

    /** More symbols (second symbols page). */
    val Symbols2: KeyboardLayout = KeyboardLayout(
        name = "Symbols2",
        isLanguage = false,
        rows = listOf(
            listOf("[","]","{","}","#","%","^","*","+","=").map { text(it) },
            listOf("_","\\","|","~","<",">","€","£","¥","·").map { text(it) },
            buildList {
                add(KeyDef("123", KeyAction.SwitchLayout("Symbols"), width = 1.5f))
                addAll(listOf(".",",","?","!","'").map { text(it) })
                add(backspace)
            },
            listOf(
                KeyDef("ABC", KeyAction.SwitchLayout("English"), width = 1.5f),
                KeyDef("🌐",  KeyAction.CycleLanguage,           width = 1f),
                space.copy(width = 4f),
                enter,
                hide,
            ),
        ),
    )

    /** Starter emoji layout. Glyph rendering depends on skiko-wasm-wasi
     *  having a color emoji font loaded — current builds may render some
     *  as boxes; that's a separate skia COLR/CBDT gap. */
    val Emoji: KeyboardLayout = KeyboardLayout(
        name = "Emoji",
        isLanguage = false,
        rows = listOf(
            listOf("😀","😁","😂","🤣","😊","😍","😎","🤔","😢","😡").map { text(it) },
            listOf("👍","👎","👏","🙏","💪","✌️","👌","🤝","👀","🔥").map { text(it) },
            listOf("❤️","💔","💯","✨","⭐","🌟","🎉","🎂","🍕","☕").map { text(it) },
            listOf(
                KeyDef("ABC", KeyAction.SwitchLayout("English"), width = 2f),
                backspace.copy(width = 2f),
                space.copy(width = 3f),
                enter.copy(width = 2f),
                hide,
            ),
        ),
    )

    /** All built-in layouts in display order. */
    fun layouts(): List<KeyboardLayout> = listOf(English, Bulgarian, Symbols, Symbols2, Emoji)
}

// ─── Composable ──────────────────────────────────────────────────────

@Composable
fun ImeKeyboard(
    layouts: List<KeyboardLayout> = ImeKeyboardDefaults.layouts(),
    initialLayoutName: String = "English",
) {
    var layoutName by remember { mutableStateOf(initialLayoutName) }
    var shifted by remember { mutableStateOf(false) }

    val layout = layouts.firstOrNull { it.name == layoutName } ?: layouts.first()
    val rows = if (shifted && layout.shiftedRows != null) layout.shiftedRows else layout.rows

    val languageLayoutNames = remember(layouts) {
        layouts.filter { it.isLanguage }.map { it.name }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .padding(4.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        for (row in rows) {
            Row(
                modifier = Modifier.fillMaxWidth().weight(1f),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                for (keyDef in row) {
                    KeyButton(
                        modifier = Modifier.weight(keyDef.width).fillMaxHeight(),
                        display = keyDef.display,
                        isModifier = keyDef.action !is KeyAction.Send,
                        shiftActive = shifted && keyDef.action is KeyAction.Shift,
                    ) {
                        when (val a = keyDef.action) {
                            is KeyAction.Send -> {
                                sendKey(a.codePoint, a.keyId)
                                // Auto-unshift after one character (typical IME behavior).
                                if (shifted) shifted = false
                            }
                            KeyAction.Shift -> shifted = !shifted
                            is KeyAction.SwitchLayout -> {
                                layoutName = a.targetLayoutName
                                shifted = false
                            }
                            KeyAction.CycleLanguage -> {
                                val curIdx = languageLayoutNames.indexOf(layoutName)
                                if (languageLayoutNames.isNotEmpty()) {
                                    val next = (curIdx + 1).coerceAtLeast(0) % languageLayoutNames.size
                                    layoutName = languageLayoutNames[next]
                                    shifted = false
                                }
                            }
                            KeyAction.Hide -> {
                                WitCanvas.Import.logMessage(
                                    "ime: Hide tapped — step 4 will wire request-hide WIT verb"
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun KeyButton(
    display: String,
    isModifier: Boolean,
    shiftActive: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    // pointerInput(Unit) below runs ONCE and never restarts. The gesture
    // detector captures `onClick`, but recompositions that change Shift
    // need the CURRENT recomposition's lambda — wrap with
    // rememberUpdatedState so the captured State<> is stable but the
    // value reflects the latest recomposition.
    val currentOnClick by rememberUpdatedState(onClick)
    val bg = when {
        shiftActive -> MaterialTheme.colorScheme.primary
        isModifier  -> MaterialTheme.colorScheme.surface
        else        -> MaterialTheme.colorScheme.surfaceContainerHighest
    }
    val fg = when {
        shiftActive -> MaterialTheme.colorScheme.onPrimary
        isModifier  -> MaterialTheme.colorScheme.onSurface
        else        -> MaterialTheme.colorScheme.onSurface
    }
    Box(
        modifier = modifier
            .background(bg, RoundedCornerShape(8.dp))
            // Raw pointerInput + awaitEachGesture rather than
            // Modifier.clickable / detectTapGestures. (clickable installs
            // a FocusableNode that, in the in-canvas case, stole focus
            // from the focused BasicTextField. In the dedicated-guest
            // IME, focus is in another process, so clickable is harmless
            // — but keeping the same pattern means a single audit-able
            // input path across both keyboards.)
            .pointerInput(Unit) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    val up = waitForUpOrCancellation()
                    if (up != null) currentOnClick()
                }
            }
            .padding(4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = display,
            color = fg,
            fontSize = if (isModifier) 14.sp else 18.sp,
            fontWeight = if (isModifier) FontWeight.SemiBold else FontWeight.Normal,
        )
    }
}
