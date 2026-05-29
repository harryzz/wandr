// war.ime.keyboard — task 47 step 3c follow-up. Port of wart-app's
// WasiSoftKeyboard to the dedicated-guest IME, swapping the
// onKey(KeyEvent) callback for direct Keyboard.Import.sendKeyEvent
// calls (the IME's outbound WIT path → arbiter → focused-pid).
//
// Built-in layouts: English QWERTY (+ Shifted), Numeric, Phone,
// Email, Url, Password, Symbols, Symbols2, Emoji. Additional
// languages load at startup from `war.lang.*` plugins via the
// `war:keyboard-lang/lang` WIT contract (task 49 step 5). Shift +
// layout-cycle + switch-layout all work. Autorepeat omitted (parent
// file's comment documents why the original disabled it).
//
// The "Hide" key sends ESC to the focused editor, reusing its
// lose-focus dismiss path (→ notify-editor-detached → the arbiter hides
// the overlay). Same effect as tapping outside a text field. No bespoke
// "request-hide" WIT verb / arbiter overlay-clear command needed.

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

// ─── Compose Key constants ───────────────────────────────────────────
//
// Numeric IDs match upstream Compose's webMain hard-codes for
// `Key.Backspace`, `Key.Enter`, etc. — the same values the host's
// winit branch sends through `on-key-event-v2` (see wart-host
// lib.rs:512-528) so a guest can pass `key-id` straight into
// `Key(keyId.toLong())` without a translation table. NOT the same
// as Android's AKEYCODE_* (Backspace=67, Enter=66 there) — the
// dispatch_key_v2 path doesn't translate, so the IME must send
// what the guest expects.
//
// For printable characters (letters / digits / punctuation), `key-id`
// is 0 and `code-point` carries the char (matching the winit `_ => 0`
// fallback). The guest's BasicTextField inserts based on code-point.

private const val KEY_BACKSPACE: Int = 8
private const val KEY_TAB:       Int = 9
private const val KEY_ENTER:     Int = 13
private const val KEY_ESCAPE:    Int = 27
private const val KEY_SPACE:     Int = 32

/** Map a printable char to a sensible key-id; 0 for "letters / digits /
 *  punctuation" (the guest uses code-point for those). */
private fun keyIdFor(c: Char): Int = when (c) {
    ' ' -> KEY_SPACE
    else -> 0
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

    /** Hide the keyboard — sends ESC to the focused editor, reusing its
     *  lose-focus dismiss path (→ notify-editor-detached → arbiter hides
     *  the overlay). No bespoke hide verb needed. */
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

    /** Character codepoint of `c` plus a Compose-compatible key-id
     *  (0 for letters — the field uses code-point). */
    private fun letter(c: Char): KeyDef =
        KeyDef(c.toString(), KeyAction.Send(c.code, keyIdFor(c)))

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
        val keyId = if (s.length == 1) keyIdFor(s[0]) else 0
        return KeyDef(s, KeyAction.Send(codePoint, keyId))
    }

    // For Enter, send code-point=10 ('\n') alongside KEY_ENTER so the
    // focused field can insert a newline (multi-line BasicTextField) OR
    // dispatch its IME action (single-line). Pure key-id=13 with
    // code-point=0 leaves singleLine fields without text to insert and
    // they trigger imeAction defaults — usually a focus reset / submit
    // — which is what the user saw as "cursor jumps to the beginning,
    // no newline". The '\n' codepoint gives multi-line fields the char
    // they need.
    private val backspace = KeyDef("⌫", KeyAction.Send(0,  KEY_BACKSPACE), width = 1.5f)
    private val space     = KeyDef(" ", KeyAction.Send(32, KEY_SPACE),     width = 4.0f)
    private val enter     = KeyDef("⏎", KeyAction.Send(10, KEY_ENTER),     width = 1.5f)
    // Wider tap target (the right-edge Hide key was easy to miss / hit
    // Enter instead — it was only weight 1 next to Enter's 1.5).
    private val hide      = KeyDef("⌄", KeyAction.Hide,                    width = 1.9f)

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

    // Bulgarian + French moved out to `war.lang.bg/` + `war.lang.fr/`.
    // They load at IME startup via the `war:keyboard-lang-*/lang`
    // plugin contracts (task 49 step 5, `LangAdapter.kt`).

    /** Wrap a plugin's 3 rows of letter keys with the IME-uniform
     *  digits / shift / utility rows. Plugin contract: returns N rows
     *  of letters; this code prepends the shared digit row, flanks
     *  the LAST plugin row with ⇧ + ⌫, and appends the standard
     *  utility row (123 / 🌐 / 😀 / space / `.` / ⏎ / ⌄). Behavior
     *  identical to the original inline Bulgarian / English layouts. */
    fun wrapLanguageLayout(
        name:              String,
        letterRows:        List<List<KeyDef>>,
        shiftedLetterRows: List<List<KeyDef>>,
    ): KeyboardLayout {
        fun wrap(rows: List<List<KeyDef>>, bottomPunct: KeyDef): List<List<KeyDef>> {
            val letterTopRows = rows.dropLast(1).ifEmpty { listOf(emptyList()) }
            val lastRow       = rows.lastOrNull().orEmpty()
            return buildList {
                add(digitsRow)
                addAll(letterTopRows)
                add(buildList {
                    add(KeyDef("⇧", KeyAction.Shift, width = 1.5f))
                    addAll(lastRow)
                    add(backspace)
                })
                add(listOf(
                    KeyDef("123", KeyAction.SwitchLayout("Symbols"), width = 1.3f),
                    KeyDef("🌐",  KeyAction.CycleLanguage,            width = 1f),
                    KeyDef("😀",  KeyAction.SwitchLayout("Emoji"),    width = 1f),
                    space.copy(width = 3.4f),
                    bottomPunct,
                    enter,
                    hide,
                ))
            }
        }
        return KeyboardLayout(
            name        = name,
            rows        = wrap(letterRows,        text(".")),
            shiftedRows = wrap(shiftedLetterRows, text(",")),
        )
    }

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

    // ── Editor-driven layouts (task 49 step 2) ─────────────────────────
    //
    // Picked automatically by `pickLayout` when the focused editor's
    // input-type matches. `isLanguage = false` so the 🌐 cycle skips
    // them — they're not user-cyclable.

    /** Numeric keypad. Picked for `input-type = number`. 4 rows × 4 keys,
     *  all width=1 so columns line up vertically — using backspace.copy
     *  / enter.copy to override the default 1.5 width those carry in
     *  the alphabetic layouts. */
    val Numeric: KeyboardLayout = KeyboardLayout(
        name = "Numeric",
        isLanguage = false,
        rows = listOf(
            listOf(text("1"), text("2"), text("3"), backspace.copy(width = 1f)),
            listOf(text("4"), text("5"), text("6"), enter.copy(width = 1f)),
            listOf(text("7"), text("8"), text("9"), text(".")),
            listOf(
                text("-"),
                text("0"),
                text(","),
                KeyDef("ABC", KeyAction.SwitchLayout("English"), width = 1f),
            ),
        ),
    )

    /** Phone keypad. Picked for `input-type = phone`. ITU-T E.161
     *  layout with `*` / `#` / `+`. Same uniform width=1 grid as
     *  Numeric. */
    val Phone: KeyboardLayout = KeyboardLayout(
        name = "Phone",
        isLanguage = false,
        rows = listOf(
            listOf(text("1"), text("2"), text("3"), backspace.copy(width = 1f)),
            listOf(text("4"), text("5"), text("6"), enter.copy(width = 1f)),
            listOf(text("7"), text("8"), text("9"), text("+")),
            listOf(
                text("*"),
                text("0"),
                text("#"),
                KeyDef("ABC", KeyAction.SwitchLayout("English"), width = 1f),
            ),
        ),
    )

    /** Email layout — English QWERTY + dedicated `@` key + `.com`
     *  suffix on the modifier row. Picked for `input-type = email`. */
    val Email: KeyboardLayout = KeyboardLayout(
        name = "Email",
        isLanguage = false,
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
                text("@"),
                space.copy(width = 2.7f),
                text("."),
                KeyDef(".com", KeyAction.Send(46, 0),               width = 1.6f),
                enter,
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
                text("@"),
                space.copy(width = 2.7f),
                text("."),
                KeyDef(".com", KeyAction.Send(46, 0),               width = 1.6f),
                enter,
            ),
        ),
    )

    /** URL layout — like Email but `/` instead of `@`. Picked for
     *  `input-type = url`. */
    val Url: KeyboardLayout = KeyboardLayout(
        name = "Url",
        isLanguage = false,
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
                text("/"),
                text("."),
                space.copy(width = 2.7f),
                KeyDef(".com", KeyAction.Send(46, 0),               width = 1.6f),
                enter,
            ),
        ),
        shiftedRows = null,
    )

    /** Password layout — English QWERTY shape, no autocorrect markers
     *  (future), no `🌐`/`😀` to avoid leaking input via cycles. Picked
     *  for `input-type = password`. */
    val Password: KeyboardLayout = KeyboardLayout(
        name = "Password",
        isLanguage = false,
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
                space.copy(width = 5.4f),
                text("."),
                enter,
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
                space.copy(width = 5.4f),
                text(","),
                enter,
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

    /** All built-in layouts in display order. The user-cyclable ones
     *  (English only — additional languages load from `war.lang.*`
     *  plugins per task 49 step 5) come first; editor-driven layouts
     *  (Numeric / Phone / Email / Url / Password) follow; auxiliary
     *  (Symbols / Symbols2 / Emoji) last. Position doesn't affect the
     *  🌐 cycle — that filters by `isLanguage = true`. */
    private fun builtins(): List<KeyboardLayout> = listOf(
        English,
        Numeric, Phone, Email, Url, Password,
        Symbols, Symbols2, Emoji,
    )

    /** Built-in layouts merged with plugin-loaded language layouts.
     *  Called once at composition root. The plugin layouts are
     *  appended AFTER the built-in English so the 🌐-cycle order is
     *  `English → bg → fr → …`. Failures during plugin lift are
     *  swallowed in `LangAdapter` so a broken plugin doesn't break
     *  the IME. */
    fun loadAllLayouts(): List<KeyboardLayout> {
        val bins    = builtins()
        val plugins = LangAdapter.loadAllLangPlugins()
        // Insert plugin layouts right after English (index 0), so
        // editor-driven + symbols still tail at fixed positions.
        return buildList {
            add(bins[0])              // English first
            addAll(plugins)           // bg, fr, …
            addAll(bins.drop(1))      // editor-driven + auxiliary
        }
    }
}

// ─── Layout-pick policy (task 49 step 2) ──────────────────────────────
//
// Two concerns share one layout list:
//   - editor-driven (override): the host calls `on-editor-attached(
//     input-type)`; ImeEventsImpl stores the type; the IME picks a
//     matching specialized layout (Numeric / Phone / Email / Url /
//     Password). 🌐 cycle is disabled while editor-driven layout is
//     active.
//   - user-cycle (default): the user toggles via 🌐 between
//     `isLanguage = true` layouts.
//
// `pickLayout` is the single decision point: given the current
// editor-type (from ImeEventsImpl) AND the user's last cycle
// position, return the layout name to display.

internal fun pickLayout(
    editorType: org.jetbrains.skiko.wasi.wit.ImeInputType,
    userSelectedLang: String,
    userRequestedLayout: String,
): String = when (editorType) {
    org.jetbrains.skiko.wasi.wit.ImeInputType.NUMBER   -> "Numeric"
    org.jetbrains.skiko.wasi.wit.ImeInputType.PHONE    -> "Phone"
    org.jetbrains.skiko.wasi.wit.ImeInputType.EMAIL    -> "Email"
    org.jetbrains.skiko.wasi.wit.ImeInputType.URL      -> "Url"
    org.jetbrains.skiko.wasi.wit.ImeInputType.PASSWORD -> "Password"
    // TEXT and MULTILINE_TEXT honor the user's chosen layout (which
    // is initialized to the cycled language).
    else -> userRequestedLayout
}

/** True when the editor-type forces a layout — disables 🌐 cycle. */
internal fun isEditorTypeOverride(
    editorType: org.jetbrains.skiko.wasi.wit.ImeInputType,
): Boolean = when (editorType) {
    org.jetbrains.skiko.wasi.wit.ImeInputType.NUMBER,
    org.jetbrains.skiko.wasi.wit.ImeInputType.PHONE,
    org.jetbrains.skiko.wasi.wit.ImeInputType.EMAIL,
    org.jetbrains.skiko.wasi.wit.ImeInputType.URL,
    org.jetbrains.skiko.wasi.wit.ImeInputType.PASSWORD -> true
    else -> false
}

// ─── Composable ──────────────────────────────────────────────────────

@Composable
fun ImeKeyboard(
    layouts: List<KeyboardLayout> = ImeKeyboardDefaults.loadAllLayouts(),
    initialLayoutName: String = "English",
) {
    // userRequestedLayout — what the user actively cycled / switched to.
    // The 🌐 cycle and 123/😀/ABC switch buttons write here.
    var userRequestedLayout by remember { mutableStateOf(initialLayoutName) }
    var shifted by remember { mutableStateOf(false) }

    // Poll ImeEventsImpl for the host-delivered editor-type. The
    // backing state is `mutableStateOf` so this read tracks
    // invalidation — when the host calls on-editor-attached the
    // MutableState write triggers a recompose.
    val currentEditorType = testapp.ImeEventsImpl.currentInputType
    val layoutName = pickLayout(
        editorType = currentEditorType,
        userSelectedLang = userRequestedLayout,
        userRequestedLayout = userRequestedLayout,
    )
    val editorTypeOverridden = isEditorTypeOverride(currentEditorType)

    // Diagnostic — log every layout-name change. If pickLayout returns
    // "English" while the screenshot shows Numeric, the buttons drawn
    // and the click handlers wired up diverged.
    androidx.compose.runtime.LaunchedEffect(layoutName, currentEditorType) {
        WitCanvas.Import.logMessage(
            "ImeKeyboard: layoutName=$layoutName editorType=$currentEditorType userReq=$userRequestedLayout"
        )
    }

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
        // Task 49 step 2 — Compose's slot table reuses KeyButton slots
        // across layout changes (English row 1 col 0 → Numeric row 1
        // col 0). The reused slot keeps its pointerInput gesture
        // detector — even though rememberUpdatedState refreshes the
        // onClick reference, there's a brief recompose window where a
        // tap can land on the OLD layout's onClick. Empirically this
        // produced "y q w h" letters when typing into a Phone field
        // moments after the layout flipped from English to Phone.
        //
        // Wrap each row in `key(layoutName, rowIdx)` and each button in
        // `key(layoutName, keyDef.display)` so layout changes force
        // fresh KeyButton instances — Compose creates new gesture
        // detectors with the right closures and the stale-tap window
        // closes.
        rows.forEachIndexed { rowIdx, row ->
            androidx.compose.runtime.key(layoutName, rowIdx) {
            Row(
                modifier = Modifier.fillMaxWidth().weight(1f),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                for (keyDef in row) {
                    androidx.compose.runtime.key(layoutName, keyDef.display) {
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
                                // Task 49 step 2 — write to userRequestedLayout
                                // rather than layoutName directly. The
                                // computed layoutName reads from pickLayout
                                // which considers editor-type override. If
                                // currently overridden (e.g. Numeric for a
                                // Number field), the user's switch lands
                                // but the displayed layout stays Numeric
                                // until the editor defocuses — defensive,
                                // matches Android IME behavior.
                                userRequestedLayout = a.targetLayoutName
                                shifted = false
                            }
                            KeyAction.CycleLanguage -> {
                                // Skip the cycle when an editor-type override
                                // is active — the 🌐 key on Numeric / Phone
                                // / Email / Url / Password layouts shouldn't
                                // exist visually, but defensive guard here
                                // in case a user-supplied layout list has
                                // a 🌐 in an unexpected place.
                                if (!editorTypeOverridden) {
                                    val curIdx = languageLayoutNames.indexOf(userRequestedLayout)
                                    if (languageLayoutNames.isNotEmpty()) {
                                        val next = (curIdx + 1).coerceAtLeast(0) % languageLayoutNames.size
                                        userRequestedLayout = languageLayoutNames[next]
                                        shifted = false
                                    }
                                }
                            }
                            KeyAction.Hide -> {
                                // Reuse the lose-focus path instead of a
                                // bespoke hide-overlay verb: send ESC to
                                // the focused editor. The editor's ESC
                                // handler dismisses the IME (its
                                // SoftwareKeyboardController.hide() fires
                                // notify-editor-detached → arbiter hides
                                // the overlay), and tapping the field
                                // again re-shows the keyboard. No new WIT
                                // surface, no arbiter changes.
                                WitCanvas.Import.logMessage("ime: Hide tapped → sending ESC")
                                sendKey(0, KEY_ESCAPE)
                            }
                        }
                    }
                    }  // close inner key(layoutName, keyDef.display)
                }
            }
            }  // close outer key(layoutName, rowIdx)
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
