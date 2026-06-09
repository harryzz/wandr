package testapp.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos

// Demo composable. Uses Column for vertical layout — no manual y= positioning.
//
// Exercises:
//   - tap counter with animated tile-flash on tap (animateFloatAsState)
//   - drag-handle with capture; release snaps to nearest 25% via animateFloatAsState
//   - LaunchedEffect heartbeat counter to show the frame clock keeps ticking
@Composable
fun TapCounterApp(screenWidth: Float) {
    val s = screenWidth / 360f
    fun sp(v: Float) = v * s
    var frames by remember { mutableStateOf(0) }
    LaunchedEffect(Unit) {
        while (true) withFrameNanos { _ -> frames += 1 }
    }
    Column(x = sp(20f), y = sp(380f), spacing = sp(20f)) {
        DragSlider(
            trackW = sp(320f),
            handleW = sp(40f),
            handleH = sp(40f),
            tickFontSize = sp(12f),
        )
        TapTile(
            width = sp(320f),
            height = sp(80f),
            fontSize = sp(16f),
            frames = frames,
        )
    }
}

@Composable
private fun TapTile(width: Float, height: Float, fontSize: Float, frames: Int) {
    var count       by remember { mutableStateOf(0) }
    var flashTarget by remember { mutableStateOf(0f) }
    val flash by animateFloatAsState(targetValue = flashTarget, durationMillis = 200)

    val r = (0x0F + (0x00 - 0x0F) * flash).toInt().coerceIn(0, 255)
    val g = (0x34 + (0xD4 - 0x34) * flash).toInt().coerceIn(0, 255)
    val b = (0x60 + (0xFF - 0x60) * flash).toInt().coerceIn(0, 255)
    val tileColor = (0xFF000000.toInt()) or (r shl 16) or (g shl 8) or b

    Box {
        Rect(0f, 0f, width, height, color = tileColor)
        Text("count=$count  frames=$frames",
             x = fontSize, y = height - fontSize,
             fontSize = fontSize, color = 0xFFE2E8F0.toInt())
        OnClick(0f, 0f, width, height, onTap = {
            count++
            flashTarget = if (flashTarget == 1f) 0f else 1f
        })
    }
}

@Composable
private fun DragSlider(
    trackW: Float,
    handleW: Float,
    handleH: Float,
    tickFontSize: Float,
) {
    var dragX      by remember { mutableStateOf(0f) }
    var snapTarget by remember { mutableStateOf(0f) }
    var dragging   by remember { mutableStateOf(false) }
    val snapAnim by animateFloatAsState(targetValue = snapTarget, durationMillis = 350)
    val displayDrag = if (dragging) dragX else snapAnim
    val handleX = (trackW - handleW) * displayDrag

    Box {
        // Track + tick marks
        Rect(0f, handleH * 0.4f, trackW, handleH * 0.2f, color = 0xFF334155.toInt())
        for (i in 0..4) {
            val tickX = (trackW - handleW) * (i / 4f) + handleW / 2f - 1f
            Rect(tickX, handleH * 0.2f, 2f, handleH * 0.6f, color = 0xFF64748B.toInt())
        }
        val handleColor = if (dragging) 0xFF00FF88.toInt() else 0xFFE94560.toInt()
        Rect(handleX, 0f, handleW, handleH, color = handleColor)
        OnClick(
            handleX, 0f, handleW, handleH,
            onTap     = { dragging = true; dragX = displayDrag },
            onMove    = { dx, _ ->
                dragX = ((trackW - handleW) * dragX + dx) / (trackW - handleW)
                dragX = dragX.coerceIn(0f, 1f)
            },
            onRelease = {
                dragging = false
                snapTarget = (kotlin.math.round(dragX * 4f) / 4f).coerceIn(0f, 1f)
            },
        )
        Text("drag=${(displayDrag * 100).toInt()}%",
             x = 0f, y = handleH + tickFontSize,
             fontSize = tickFontSize, color = 0xFFE2E8F0.toInt())
    }
}
