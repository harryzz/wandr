package testapp

import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.SoftwareKeyboardController
import androidx.compose.ui.text.input.KeyboardType

/**
 * In-canvas Compose keyboard visibility controller for wasi.
 *
 * Implements the standard [SoftwareKeyboardController] contract so any
 * Compose call to `LocalSoftwareKeyboardController.current?.show()` /
 * `.hide()` (e.g. from `BasicTextField`'s tap-while-focused or its
 * `ImeAction.Done` handler) drives our in-canvas [WasiSoftKeyboard].
 *
 * The visibility flag is a [MutableState] so composables can react to
 * it without listener wiring — `WasiSoftKeyboard` is drawn iff
 * [isVisible] is true.
 *
 * Wandr's setup (`RealComposeApp.kt`):
 *   1. `MaterialDemoApp` creates one via `rememberWasiKeyboardController()`,
 *      provides it as `LocalSoftwareKeyboardController`, and reads
 *      [isVisible] to decide whether to draw the keyboard.
 *   2. `TextFieldCard` calls `controller.show()` / `.hide()` from a
 *      `Modifier.onFocusChanged { … }` block, so taps that gain or lose
 *      focus toggle the keyboard.
 *   3. Hardware ESC (and the keyboard's own ⌄ key) call `controller.hide()`.
 */
class WasiKeyboardController : SoftwareKeyboardController {
    /** Drive your `if (controller.isVisible.value) WasiSoftKeyboard(...)`. */
    val isVisible: MutableState<Boolean> = mutableStateOf(false)

    /**
     * Task 49 step 2 — the focused field's `KeyboardOptions.keyboardType`.
     * `SoftwareKeyboardController.show()` has no args, so we can't
     * read the keyboardType inside `show()` from Compose's standard
     * path. TextFieldCard writes here in `.onFocusChanged` BEFORE
     * calling `controller.show()`; `show()` reads + threads it
     * through to the WIT `notify-editor-attached` call. Defaults to
     * Text so the external IME shows English QWERTY if no field
     * explicitly sets it.
     */
    var pendingKeyboardType: KeyboardType = KeyboardType.Text

    override fun show() {
        isVisible.value = true
        // Task 47 step 2 + task 49 step 2 — notify the arbiter via the
        // `my:skiko-gfx/ime` WIT verb so the external IME can pick a
        // matching layout (Numeric for KeyboardType.Number, etc.).
        // The arbiter routes `editor-attached <type>` to the
        // currently-active IME app's per-host control socket; the
        // IME calls into our exported `wandr:ime/ime.on-editor-attached`.
        try {
            org.jetbrains.skiko.wasi.wit.Ime.Import.notifyEditorAttached(
                inputType = keyboardTypeToWire(pendingKeyboardType),
                hint = "",
                initialText = "",
                selectionStart = 0u,
                selectionEnd = 0u,
            )
        } catch (t: Throwable) {
            // Defensive — if the host's ime_host_impl can't reach
            // the arbiter (daemon down), don't fail the keyboard
            // show. The in-canvas keyboard still works.
        }
    }

    override fun hide() {
        isVisible.value = false
        try {
            org.jetbrains.skiko.wasi.wit.Ime.Import.notifyEditorDetached()
        } catch (t: Throwable) {
            // Same defensive pattern as show().
        }
    }

    /**
     * Map Compose's `KeyboardType` to the `wandr:ime/input-type` enum
     * tag strings that `my:skiko-gfx/ime.notify-editor-attached`
     * accepts. The wire uses stringly-typed values (see
     * skiko-gfx.wit) to avoid a cross-package WIT import; the host
     * matches on the string and converts to the typed enum before
     * calling the IME guest.
     */
    private fun keyboardTypeToWire(kt: KeyboardType): String = when (kt) {
        KeyboardType.Number,
        KeyboardType.Decimal,
        KeyboardType.NumberPassword -> "number"
        KeyboardType.Phone          -> "phone"
        KeyboardType.Email          -> "email"
        KeyboardType.Uri            -> "url"
        KeyboardType.Password       -> "password"
        else                        -> "text"
    }
}

@Composable
fun rememberWasiKeyboardController(): WasiKeyboardController =
    remember { WasiKeyboardController() }
