package testapp

import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp

/**
 * Compose-ui core smoke test. Compiles iff compose-ui-wasi is linkable;
 * touches Modifier, Alignment, Key, PointerEventType, semantics keys, Density
 * compositionLocal default — all from upstream commonMain. No runtime
 * dependency on rendering — just type/identifier resolution.
 */
fun composeUiSmokeTest() {
    val alignment = Alignment.Center
    val modifier: Modifier = Modifier
    val rect = Rect(Offset.Zero, Size(100f, 100f))
    val intOff = IntOffset(0, 0)
    val intSize = IntSize(100, 100)
    val key = Key.Enter
    val ptr = PointerEventType.Press
    val color = Color.Red
    val density = LocalDensity
    val descKey = SemanticsProperties.ContentDescription
    logMessage(
        "compose-ui smoke: alignment=${alignment}, rect=${rect}, intOff=${intOff}, " +
        "intSize=${intSize}, key=${key.keyCode}, ptr=${ptr}, color=${color}, " +
        "modifier=${modifier::class.simpleName}, density-local=${density::class.simpleName}, " +
        "desc-key=${descKey.name}"
    )
}
