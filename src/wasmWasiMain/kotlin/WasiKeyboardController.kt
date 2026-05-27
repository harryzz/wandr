package testapp

import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.SoftwareKeyboardController

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
 * Wart's setup (`RealComposeApp.kt`):
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

    override fun show() {
        isVisible.value = true
        // Task 47 step 2 — also notify the arbiter via the new
        // `my:skiko-gfx/ime` WIT verb. The arbiter routes
        // `on-editor-attached` to the currently-active IME app.
        //
        // We co-exist with the in-canvas keyboard (it still renders
        // based on `isVisible.value`); the WIT call is a separate
        // outbound signal that lights up the protocol path without
        // changing user-visible behavior. The "no UI swap yet"
        // promise in the scope doc is honored — step 4 swaps the
        // in-canvas surface for the real external IME.
        try {
            org.jetbrains.skiko.wasi.wit.Ime.Import.notifyEditorAttached(
                inputType = "text",   // future: thread BasicTextField's
                                       //         KeyboardOptions.keyboardType through
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
}

@Composable
fun rememberWasiKeyboardController(): WasiKeyboardController =
    remember { WasiKeyboardController() }
