package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/// Hand-crafted 4×4 RGB PNG used by the task-27 smoke card.
/// Quadrants: red / green / blue / yellow (2×2 cells of each). Tiny on
/// purpose (83 bytes) — verifies the makeFromEncoded → image-shader →
/// blend-shader chain end-to-end without bloating the cwasm.
private val testPng: ByteArray = byteArrayOf(
    0x89.toByte(), 0x50.toByte(), 0x4e.toByte(), 0x47.toByte(),
    0x0d.toByte(), 0x0a.toByte(), 0x1a.toByte(), 0x0a.toByte(),
    0x00.toByte(), 0x00.toByte(), 0x00.toByte(), 0x0d.toByte(),
    0x49.toByte(), 0x48.toByte(), 0x44.toByte(), 0x52.toByte(),
    0x00.toByte(), 0x00.toByte(), 0x00.toByte(), 0x04.toByte(),
    0x00.toByte(), 0x00.toByte(), 0x00.toByte(), 0x04.toByte(),
    0x08.toByte(), 0x02.toByte(), 0x00.toByte(), 0x00.toByte(),
    0x00.toByte(), 0x26.toByte(), 0x93.toByte(), 0x09.toByte(),
    0x29.toByte(), 0x00.toByte(), 0x00.toByte(), 0x00.toByte(),
    0x1a.toByte(), 0x49.toByte(), 0x44.toByte(), 0x41.toByte(),
    0x54.toByte(), 0x78.toByte(), 0xda.toByte(), 0x63.toByte(),
    0xf8.toByte(), 0xcf.toByte(), 0xc0.toByte(), 0x00.toByte(),
    0x44.toByte(), 0x0c.toByte(), 0xff.toByte(), 0x91.toByte(),
    0x10.toByte(), 0x8c.toByte(), 0x02.toByte(), 0xd3.toByte(),
    0xff.toByte(), 0xff.toByte(), 0x03.toByte(), 0x31.toByte(),
    0x2a.toByte(), 0x07.toByte(), 0x00.toByte(), 0xca.toByte(),
    0x77.toByte(), 0x13.toByte(), 0xed.toByte(), 0x80.toByte(),
    0x19.toByte(), 0x49.toByte(), 0xdc.toByte(), 0x00.toByte(),
    0x00.toByte(), 0x00.toByte(), 0x00.toByte(), 0x49.toByte(),
    0x45.toByte(), 0x4e.toByte(), 0x44.toByte(), 0xae.toByte(),
    0x42.toByte(), 0x60.toByte(), 0x82.toByte(),
)

/// Task-27 smoke verification. One-shot at composition time:
///   1. Image.makeFromEncoded → decodes the testPng above.
///   2. Image.makeShader      → tiled pattern shader.
///   3. Shader.makeSweepGradient (legacy params overload).
///   4. Shader.makeBlend       → composites the two above.
/// Status text reports which calls returned valid handles; visual
/// strip below shows Compose's Brush.sweepGradient (which now goes
/// through the new create-sweep-gradient WIT verb).
@Composable
internal fun Task27SmokeCard() {
    val status = remember {
        val lines = mutableListOf<String>()
        runCatching {
            val img = org.jetbrains.skia.Image.makeFromEncoded(testPng)
            lines += "Image.makeFromEncoded: ${img.width}x${img.height}"
            val imgShader = img.makeShader(
                tmx = org.jetbrains.skia.FilterTileMode.REPEAT,
                tmy = org.jetbrains.skia.FilterTileMode.REPEAT,
                sampling = org.jetbrains.skia.SamplingMode.LINEAR,
                localMatrix = null,
            )
            lines += "Image.makeShader: ok"
            val sweep = org.jetbrains.skia.Shader.makeSweepGradient(
                cx = 100f, cy = 100f,
                colors = intArrayOf(0xFFFF0000.toInt(), 0xFF00FF00.toInt(), 0xFF0000FF.toInt(), 0xFFFF0000.toInt()),
                stops = null,
                tileMode = org.jetbrains.skia.FilterTileMode.CLAMP,
                startAngle = 0f, endAngle = 360f,
            )
            lines += "Shader.makeSweepGradient: ok"
            val blended = org.jetbrains.skia.Shader.makeBlend(
                mode = org.jetbrains.skia.BlendMode.MULTIPLY,
                dst  = imgShader,
                src  = sweep,
            )
            lines += "Shader.makeBlend: ok"
            // Free the host-side shaders + image; ids stay live only for
            // this remember{} block.
            imgShader.discard()
            sweep.discard()
            blended.discard()
            img.close()
        }.onFailure { lines += "FAILED: ${it.message}" }
        lines.joinToString("\n")
    }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Task 27 — image / shader APIs",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(text = status, fontSize = 11.sp)
            // Visual confirmation: Compose's Brush.sweepGradient now
            // resolves to the new create-sweep-gradient WIT verb.
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(
                    modifier = Modifier
                        .size(80.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(
                            Brush.sweepGradient(
                                listOf(Color.Red, Color.Yellow, Color.Green, Color.Cyan, Color.Blue, Color.Magenta, Color.Red),
                            )
                        ),
                )
                Box(
                    modifier = Modifier
                        .size(80.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(
                            Brush.linearGradient(
                                listOf(Color.Red, Color.Blue),
                            )
                        ),
                )
                Box(
                    modifier = Modifier
                        .size(80.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(
                            Brush.radialGradient(
                                listOf(Color.Yellow, Color.Black),
                            )
                        ),
                )
            }
            Spacer(Modifier.height(4.dp))
            Text(
                text = "sweep • linear • radial",
                fontSize = 10.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

