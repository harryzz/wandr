package testapp

import androidx.compose.foundation.MutatePriority
import androidx.compose.foundation.gestures.FlingBehavior
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.ScrollableDefaults
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.interaction.HoverInteraction
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

/**
 * compose-foundation smoke test. Compiles iff compose-foundation-wasi is
 * linkable. Exercises:
 *   - gesture/scroll types (Orientation, FlingBehavior, ScrollableDefaults)
 *   - interaction sources (Hover/Press)
 *   - lazy list state
 *   - text-field knobs (KeyboardOptions, ImeAction, KeyboardType, LineLimits)
 * Doesn't actually run @Composable; just verifies symbol resolution.
 */
fun composeFoundationSmokeTest() {
    val orientations = listOf(Orientation.Horizontal, Orientation.Vertical)
    val mutatePrio = MutatePriority.PreventUserInput
    val pe = PointerEventType.Press
    val interactionSource = MutableInteractionSource()
    val pressInteraction = PressInteraction.Press(Offset(10f, 20f))
    val hoverInteraction = HoverInteraction.Enter()
    val lazyState = LazyListState(firstVisibleItemIndex = 5, firstVisibleItemScrollOffset = 42)

    val keyboardOptions = KeyboardOptions(
        capitalization = KeyboardCapitalization.Sentences,
        autoCorrectEnabled = true,
        keyboardType = KeyboardType.Text,
        imeAction = ImeAction.Done,
    )
    val singleLine = TextFieldLineLimits.SingleLine
    val multiLine = TextFieldLineLimits.MultiLine(minHeightInLines = 1, maxHeightInLines = 4)

    WitCanvas.Import.logMessage(
        "compose-foundation smoke: " +
        "orientations=${orientations}, mutate=${mutatePrio}, pointerType=${pe}, " +
        "press=${pressInteraction}, hover=${hoverInteraction::class.simpleName}, " +
        "lazyState=firstIdx=${lazyState.firstVisibleItemIndex} scrollOff=${lazyState.firstVisibleItemScrollOffset}, " +
        "keyboardOptions={cap=${keyboardOptions.capitalization}, type=${keyboardOptions.keyboardType}, ime=${keyboardOptions.imeAction}}, " +
        "lineLimits=[${singleLine::class.simpleName}, ${multiLine::class.simpleName}(${multiLine.minHeightInLines}..${multiLine.maxHeightInLines})]"
    )
}
