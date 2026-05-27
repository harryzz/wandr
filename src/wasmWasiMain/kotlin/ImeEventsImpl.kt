// Receiver for the `war:ime/ime` exported events (task 49 step 1b).
// The host calls into our @WasmExport wrappers in
// `generated/ImeExports.kt`, which lift the canonical-ABI params and
// route here.
//
// Step 2: `currentInputType` is a Compose `MutableState` so layout-
// pick in `ImeKeyboard.kt` reactively updates when the host
// delivers `editor-attached(input-type)`. The @WasmExport stubs
// write into the State via `mutableStateOf`'s `.value =` — Compose
// invalidates subscribers automatically.

package testapp

import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import org.jetbrains.skiko.wasi.wit.ImeInputType

object ImeEventsImpl {
    /// Compose-tracked input type. ImeKeyboard reads this inside
    /// composition; writes from @WasmExport invalidate the subscribers.
    private val _currentInputType: MutableState<ImeInputType> =
        mutableStateOf(ImeInputType.TEXT)

    val currentInputType: ImeInputType
        get() = _currentInputType.value

    /// Compose-tracked focus state. Drives whether the IME shows the
    /// "no editor focused" placeholder vs the real keyboard.
    private val _hasFocusedEditor: MutableState<Boolean> = mutableStateOf(false)

    val hasFocusedEditor: Boolean
        get() = _hasFocusedEditor.value

    /// Diagnostic — used by step 6 smoke to verify the wire reached
    /// the guest end. Bumped on every editor-attached.
    var attachCount: Int = 0
        private set

    fun recordInputTypeTag(tag: Int) {
        _currentInputType.value = when (tag) {
            0 -> ImeInputType.TEXT
            1 -> ImeInputType.NUMBER
            2 -> ImeInputType.PHONE
            3 -> ImeInputType.EMAIL
            4 -> ImeInputType.URL
            5 -> ImeInputType.PASSWORD
            6 -> ImeInputType.MULTILINE_TEXT
            else -> ImeInputType.TEXT
        }
        _hasFocusedEditor.value = true
        attachCount += 1
    }

    fun recordDetached() {
        _hasFocusedEditor.value = false
        _currentInputType.value = ImeInputType.TEXT
    }
}
