@file:OptIn(androidx.compose.ui.InternalComposeUiApi::class)

package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * In-canvas soft keyboard for wasi/Compose, modeled on `egui_keyboard`'s
 * approach: pure-Kotlin composable, no JNI, no system IME, no Binder.
 *
 * Architecture goals:
 *   * **Layout-pluggable**. Built-in: English QWERTY + Shifted + Symbols
 *     + a starter Emoji page. Adding another (Cyrillic, Greek, Hebrew,
 *     etc.) is just defining one more `KeyboardLayout`.
 *   * **Output via Compose KeyEvent.** Caller passes an `onKey` callback;
 *     wire it to `realScene.sendKeyEvent(...)` so the focused
 *     `BasicTextField(state: TextFieldState, ...)` receives the typed
 *     character through the same path used by the hardware-keyboard
 *     `WasiInput.setKeyHandler`.
 *   * **No system services.** The keyboard lives entirely inside our
 *     own surface and our own composition.
 *
 * Extensibility:
 *   * Pass `layouts = listOf(MyLayout1, MyLayout2, …)` to add more
 *     language layouts; the 🌐 modifier cycles through them in
 *     declaration order. Emoji can be split into multiple pages
 *     (`Emoji-Faces`, `Emoji-Animals`, …) — each is just another
 *     KeyboardLayout in the list.
 */

/** Width weight for a key relative to the other keys in its row. */
typealias KeyWidth = Float

/** What pressing one key emits + which layout-switch it triggers, if any. */
sealed interface KeyAction {
    /** Send a Compose KeyEvent (text input). */
    data class Send(val key: Key, val codePoint: Int) : KeyAction

    /** Toggle the shift state. */
    data object Shift : KeyAction

    /** Switch to a named layout. */
    data class SwitchLayout(val targetLayoutName: String) : KeyAction

    /** Cycle through the "language" layouts in `WasiSoftKeyboard.layouts` (the 🌐 key). */
    data object CycleLanguage : KeyAction

    /** Hide the keyboard. */
    data object Hide : KeyAction
}

/** One key definition. */
data class KeyDef(
    val display: String,
    val action: KeyAction,
    val width: KeyWidth = 1f,
    /**
     * If true, holding this key fires the action repeatedly: one fire at
     * 500ms, then every 50ms until release. Typical for ⌫/space/enter on
     * Android/iOS keyboards. Untouched for letters (most users don't expect
     * 'aaaaaaaa' from holding 'a' on a soft keyboard).
     */
    val autorepeat: Boolean = false,
)

/** A full keyboard layout. `shiftedRows` is optional; if null, shift is a no-op. */
data class KeyboardLayout(
    val name: String,
    /** Set this to false for symbol / emoji layouts so the 🌐 modifier skips them. */
    val isLanguage: Boolean = true,
    val rows: List<List<KeyDef>>,
    val shiftedRows: List<List<KeyDef>>? = null,
)

/** Built-in layouts. Apps can prepend / append their own to customize. */
object WasiSoftKeyboardDefaults {

    private fun letter(c: Char, key: Key = Key(c.code.toLong())): KeyDef =
        KeyDef(c.toString(), KeyAction.Send(key, c.code))

    /**
     * Codepoint of the first character (handles surrogate-pair emoji where
     * the first Char's `.code` is just the high surrogate; we need the
     * UTF-32 codepoint). For grapheme clusters like "❤️" (heart + variation
     * selector) we use the BASE codepoint only — the cluster is rendered
     * by the display string but only the base codepoint is sent via
     * KeyEvent.codePoint, which is fine for text-field commit.
     */
    private fun firstCodePoint(s: String): Int {
        val c0 = s[0]
        if (c0.isHighSurrogate() && s.length >= 2) {
            val c1 = s[1]
            if (c1.isLowSurrogate()) {
                // UTF-16 surrogate pair → UTF-32 codepoint (manual math; the
                // `Char.toCodePoint(...)` helper is internal in Kotlin/Wasm
                // stdlib).
                return 0x10000 + ((c0.code - 0xD800) shl 10) + (c1.code - 0xDC00)
            }
        }
        return c0.code
    }

    /** A printable codepoint that isn't a standard letter (digits, punctuation, emoji…). */
    private fun text(s: String, codePoint: Int = firstCodePoint(s)): KeyDef =
        KeyDef(s, KeyAction.Send(Key(codePoint.toLong()), codePoint))

    // Autorepeat temporarily disabled. The earlier implementation produced
    // freezes / "delete whole edit box" symptoms on press-and-hold (race
    // between the gesture's UP detector and a separately-launched repeat
    // coroutine on the single Kotlin dispatcher thread; rewriting it as a
    // single coroutine inside the gesture scope didn't recover. Needs a
    // different strategy — likely a frame-loop-based check ("if pointer
    // still down at this frame and DOWN was >500ms ago, fire").
    private val backspace = KeyDef("⌫", KeyAction.Send(Key(8), 0), width = 1.5f)
    private val space     = KeyDef(" ", KeyAction.Send(Key(32), 32), width = 4.0f)
    private val enter     = KeyDef("⏎", KeyAction.Send(Key(13), 13), width = 1.5f)
    private val hide      = KeyDef("⌄", KeyAction.Hide,              width = 1f)

    /** Top digits row (1-9, 0) — shared across language layouts. */
    private val digitsRow: List<KeyDef> = listOf(
        "1","2","3","4","5","6","7","8","9","0",
    ).map { text(it) }

    /** English QWERTY (US layout). 5 rows: digits, QWERTY, ASDFGHJKL, ⇧ZXCVBNM⌫, modifiers. */
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
                KeyDef(".",    KeyAction.Send(Key(46), 46),         width = 1f),
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
                KeyDef(",",    KeyAction.Send(Key(44), 44),         width = 1f),
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
                KeyDef(".",    KeyAction.Send(Key(46), 46),        width = 1f),
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
                KeyDef(",",    KeyAction.Send(Key(44), 44),        width = 1f),
                enter,
                hide,
            ),
        ),
    )

    /** Digits + common punctuation + math. */
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

    /**
     * Starter emoji layout — a small picker. To grow this, just add more rows
     * or split into multiple emoji pages (Faces / Animals / Food / Objects).
     * Note: glyph rendering depends on whether skiko-wasm-wasi has a color
     * emoji font loaded. If glyphs appear as boxes, emoji rendering is the
     * separate gap to close (Skia COLR/CBDT + Noto Color Emoji TTF embedding).
     */
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

/**
 * In-canvas soft keyboard.
 *
 * @param onKey callback fired for every Send-action key. Wire to
 *   `realScene.sendKeyEvent(...)` to deliver into the focused TextField.
 * @param layouts ordered list of layouts. The 🌐 modifier cycles through
 *   those with `isLanguage = true`; modifier keys switch directly by name.
 * @param initialLayoutName which layout to show on first composition.
 * @param height total keyboard height. The English / Bulgarian layouts now
 *   have 5 rows (digits + 3 letter rows + modifier row), Symbols/Emoji have
 *   4. Default leaves enough room for the taller layouts.
 * @param onHide optional dismissal callback (e.g., when a future ⌄ key fires).
 */
@Composable
fun WasiSoftKeyboard(
    onKey: (KeyEvent) -> Unit,
    layouts: List<KeyboardLayout> = WasiSoftKeyboardDefaults.layouts(),
    initialLayoutName: String = "English",
    height: Dp = 300.dp,
    onHide: (() -> Unit)? = null,
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
            .fillMaxWidth()
            .height(height)
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
                        autorepeat = keyDef.autorepeat,
                    ) {
                        when (val a = keyDef.action) {
                            is KeyAction.Send -> {
                                // Down + Up for each tap so the focused
                                // BasicTextField sees a complete press.
                                onKey(KeyEvent(a.key, KeyEventType.KeyDown, a.codePoint))
                                onKey(KeyEvent(a.key, KeyEventType.KeyUp,   a.codePoint))
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
                            KeyAction.Hide -> onHide?.invoke()
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
    autorepeat: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    // pointerInput(Unit) below runs ONCE and never restarts. Any lambda the
    // gesture detector captures here freezes to the FIRST recomposition's
    // value. We route every callable through `rememberUpdatedState` so the
    // captured `State<>` reference is stable but the value reflects the
    // CURRENT recomposition — this is what makes Shift flip the case
    // correctly across taps.
    val currentOnClick by androidx.compose.runtime.rememberUpdatedState(onClick)
    val currentAutorepeat by androidx.compose.runtime.rememberUpdatedState(autorepeat)
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
            // Use raw pointerInput + awaitEachGesture rather than
            // Modifier.clickable / detectTapGestures. clickable installs a
            // FocusableNode that steals focus from the focused BasicTextField
            // (synthetic KeyEvents we then dispatch would have no recipient).
            // awaitEachGesture also lets us implement autorepeat for ⌫/␣/⏎
            // by launching a repeating coroutine on DOWN and cancelling on UP.
            .pointerInput(Unit) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    if (currentAutorepeat) {
                        // Single-coroutine autorepeat: no scope.launch.
                        // The previous design used scope.launch + delay,
                        // which produced two coroutines racing on the
                        // wasmWasi Kotlin runtime's lone thread (the
                        // launched repeat job + the gesture's UP detector)
                        // — cancel() didn't take effect until the
                        // coroutine next suspended, by which time it had
                        // already enqueued the next fire via the
                        // dispatcher. Net effect: "too fast / freeze".
                        // Here every fire runs in the SAME pointer-input
                        // coroutine and we never proceed past one fire
                        // until withTimeoutOrNull resumes us, so there is
                        // no concurrent state to race.
                        val initialUp = withTimeoutOrNull(500L) {
                            waitForUpOrCancellation()
                        }
                        if (initialUp != null) {
                            // Short tap (< 500ms) → fire once on release.
                            currentOnClick()
                            return@awaitEachGesture
                        }
                        // Held past 500ms → autorepeat.
                        var fires = 0
                        while (fires < 64) {
                            currentOnClick()
                            fires++
                            val anotherUp = withTimeoutOrNull(500L) {
                                waitForUpOrCancellation()
                            }
                            if (anotherUp != null) return@awaitEachGesture
                        }
                    } else {
                        val up = waitForUpOrCancellation()
                        if (up != null) currentOnClick()
                    }
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
