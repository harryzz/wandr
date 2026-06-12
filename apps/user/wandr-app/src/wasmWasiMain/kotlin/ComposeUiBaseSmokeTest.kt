package testapp

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.util.fastRoundToInt
import androidx.compose.ui.util.floatFromBits
import androidx.compose.ui.util.trace

fun composeUiBaseSmokeTest() {
    val offset = Offset(10f, 20f)
    val size   = Size(100f, 50f)
    val rect   = Rect(offset, size)
    val px     = IntOffset(1, 2)
    val isz    = IntSize(3, 4)

    val width: Dp  = 16.dp
    val fontSize   = 14.sp
    val density    = Density(density = 2.0f, fontScale = 1.0f)
    val widthPx    = with(density) { width.toPx() }

    val rounded   = 3.6f.fastRoundToInt()
    val piBitsHi  = floatFromBits(0x40490FDB)         // ~π
    val traced    = trace("smoke") { rect.width + rect.height }

    logMessage(
        "compose-ui-base smoke: rect=${rect}, px=${px}, isz=${isz}, " +
        "16dp@2x=${widthPx}px, 14sp=${fontSize}, " +
        "round(3.6)=${rounded}, π≈${piBitsHi}, traced=${traced}"
    )
}
