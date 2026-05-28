package testapp.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import org.jetbrains.skia.Image

/**
 * Top-level app shell with a tab bar that switches between demos. State
 * inside each demo is reset on tab switch (each branch of the `when`
 * recomposes independently — its `remember` slots are released when the
 * branch unmounts).
 */
@Composable
fun App(
    screenWidth: Float,
    pointerX: Float,
    pointerY: Float,
    pointerDown: Boolean,
    lastKey: Int,
    checkerImg: Image,
    whiteImg: Image,
) {
    val s = screenWidth / 360f
    fun sp(v: Float) = v * s
    var tab by remember { mutableStateOf(0) }   // 0 = Counter, 1 = Showcase

    val tabBarH = sp(36f)
    val tabW = (screenWidth - sp(24f)) / 2f

    Column(spacing = sp(8f)) {
        // ── Tab bar ────────────────────────────────────────────────────────
        Row(x = sp(8f), y = sp(8f), spacing = sp(8f)) {
            TabButton("Counter",  selected = tab == 0, w = tabW, h = tabBarH, sp = ::sp) { tab = 0 }
            TabButton("Showcase", selected = tab == 1, w = tabW, h = tabBarH, sp = ::sp) { tab = 1 }
        }
        // ── Active demo ───────────────────────────────────────────────────
        Box {
            when (tab) {
                0 -> CounterDemo(screenWidth)
                else -> Showcase(
                    screenWidth = screenWidth,
                    pointerX = pointerX,
                    pointerY = pointerY,
                    pointerDown = pointerDown,
                    lastKey = lastKey,
                    checkerImg = checkerImg,
                    whiteImg = whiteImg,
                )
            }
        }
    }
}

@Composable
private fun TabButton(
    label: String,
    selected: Boolean,
    w: Float,
    h: Float,
    sp: (Float) -> Float,
    onTap: () -> Unit,
) {
    val bg = if (selected) 0xFF00D4FF.toInt() else 0xFF334155.toInt()
    val fg = if (selected) 0xFF1A1A2E.toInt() else 0xFFE2E8F0.toInt()
    Box {
        Rect(0f, 0f, w, h, color = bg)
        // Crude horizontal-center: estimate label width as label.length * fontSize * 0.5,
        // place at (w - estWidth) / 2.
        val labelFontSize = sp(14f)
        val estLabelW = label.length * labelFontSize * 0.5f
        Text(label,
             x = (w - estLabelW) / 2f,
             y = h * 0.65f,
             fontSize = labelFontSize,
             color = fg)
        OnClick(0f, 0f, w, h, onTap = onTap)
    }
}
