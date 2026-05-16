package testapp.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue

/**
 * Counter sample with a slider — same logic as the canonical Compose
 * Multiplatform Counter, plus a draggable handle that controls how much
 * each tap increments by (1..10). Demonstrates state, recomposition,
 * pointer DOWN (tap), and pointer DOWN/MOVE/UP capture (drag).
 */
@Composable
fun CounterDemo(screenWidth: Float) {
    val s = screenWidth / 360f
    fun sp(v: Float) = v * s

    var count       by remember { mutableStateOf(0) }
    var sliderValue by remember { mutableStateOf(0f) }    // 0..1
    var dragging    by remember { mutableStateOf(false) }
    val increment = 1 + (sliderValue * 9).toInt()         // 1..10

    val cardW = sp(280f)
    val labelH = sp(40f)
    val rowH = sp(48f)
    val halfW = sp(132f)

    Column(x = (screenWidth - cardW) / 2f, y = sp(40f), spacing = sp(16f)) {

        // ── Count card ─────────────────────────────────────────────────────
        Box {
            Rect(0f, 0f, cardW, labelH, color = 0xFF0F3460.toInt())
            Text("Count: $count",
                 x = sp(16f), y = sp(28f),
                 fontSize = sp(18f), color = 0xFFE2E8F0.toInt())
        }

        // ── Increment + Reset buttons ──────────────────────────────────────
        Row(spacing = sp(16f)) {
            Box {
                Rect(0f, 0f, halfW, rowH, color = 0xFF00D4FF.toInt())
                Text("+$increment",
                     x = sp(48f), y = sp(31f),
                     fontSize = sp(15f), color = 0xFF1A1A2E.toInt())
                OnClick(0f, 0f, halfW, rowH, onTap = { count += increment })
            }
            Box {
                Rect(0f, 0f, halfW, rowH, color = 0xFFE94560.toInt())
                Text("Reset",
                     x = sp(40f), y = sp(31f),
                     fontSize = sp(15f), color = 0xFFE2E8F0.toInt())
                OnClick(0f, 0f, halfW, rowH, onTap = { count = 0 })
            }
        }

        // ── Increment-amount slider ────────────────────────────────────────
        Box {
            Text("Increment amount: $increment",
                 x = 0f, y = sp(12f),
                 fontSize = sp(12f), color = 0xFF94A3B8.toInt())
            val trackY = sp(20f)
            val handleW = sp(40f)
            val handleH = sp(40f)
            val trackW = cardW
            val handleX = (trackW - handleW) * sliderValue

            // Track + tick marks
            Rect(0f, trackY + handleH * 0.4f, trackW, handleH * 0.2f,
                 color = 0xFF334155.toInt())
            for (i in 0..9) {
                val tickX = (trackW - handleW) * (i / 9f) + handleW / 2f - 1f
                Rect(tickX, trackY + sp(8f), 2f, sp(24f), color = 0xFF64748B.toInt())
            }
            val handleColor = if (dragging) 0xFF00FF88.toInt() else 0xFFFFD700.toInt()
            Rect(handleX, trackY, handleW, handleH, color = handleColor)
            OnClick(
                handleX, trackY, handleW, handleH,
                onTap     = { dragging = true },
                onMove    = { dx, _ ->
                    sliderValue =
                        ((trackW - handleW) * sliderValue + dx) / (trackW - handleW)
                    sliderValue = sliderValue.coerceIn(0f, 1f)
                },
                onRelease = { dragging = false },
            )
        }
    }
}
