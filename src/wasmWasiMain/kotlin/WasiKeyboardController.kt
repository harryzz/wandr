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

    override fun show() { isVisible.value = true }

    override fun hide() { isVisible.value = false }
}

@Composable
fun rememberWasiKeyboardController(): WasiKeyboardController =
    remember { WasiKeyboardController() }
