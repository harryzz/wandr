package testapp

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect as ComposeRect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.PaintingStyle
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.colorspace.ColorSpaces

/**
 * Smoke test for compose-ui-graphics-wasi. Compiles iff the publication is
 * usable: every type below imports from upstream `androidx.compose.ui.graphics.*`
 * (pulled through compose-ui-graphics-wasi from compose-multiplatform-core).
 *
 * Calls into Paint/Path may throw NotImplementedError at runtime — the stubs
 * are bridged to host WIT canvas one method at a time. This test only verifies
 * **compile-time** API availability.
 */
fun composeUiGraphicsSmokeTest() {
    // ── Color: pure value class, fully functional ─────────────────────────
    val red    = Color.Red
    val custom = Color(0xFF334155.toInt())
    val argb   = Color(red = 0.5f, green = 0.7f, blue = 0.2f, alpha = 1.0f)
    val mixed  = Color(0.5f, 0.5f, 0.5f, 1.0f, ColorSpaces.Srgb)
    val packed: ULong = custom.value

    // ── Brush: gradient factories from upstream ───────────────────────────
    val linearBrush = Brush.linearGradient(
        colors = listOf(red, custom),
        start  = Offset(0f, 0f),
        end    = Offset(100f, 100f),
        tileMode = TileMode.Clamp,
    )
    val radialBrush = Brush.radialGradient(
        colors = listOf(Color.Blue, Color.Green),
        center = Offset(50f, 50f),
        radius = 50f,
    )
    val sweepBrush  = Brush.sweepGradient(
        colors = listOf(Color.Red, Color.Yellow, Color.Cyan),
    )

    // ── Paint: construct + configure (factory returns a working impl) ─────
    val paint = Paint().also {
        it.color = custom
        it.style = PaintingStyle.Stroke
        it.strokeWidth = 4f
        it.strokeCap = StrokeCap.Round
        it.strokeJoin = StrokeJoin.Miter
        it.blendMode = BlendMode.SrcOver
        it.isAntiAlias = true
        it.alpha = 0.8f
    }

    // ── Path: construct, build a small shape, query type ──────────────────
    val path = Path().apply {
        moveTo(0f, 0f)
        lineTo(100f, 0f)
        lineTo(100f, 100f)
        lineTo(0f, 100f)
        close()
    }
    path.fillType = PathFillType.NonZero
    val pathBounds = path.getBounds()
    val pathIsEmpty = path.isEmpty
    val pathIsConvex = path.isConvex

    // Regression check for the Slider crash: exercises the
    // addRoundRect → rewind pattern from material3 Slider's drawTrackPath.
    // Before the PathBuilder.reset/rewind fix in skiko-wasi, rewind() here
    // would infinite-recurse into SkiaBackedPath.reset and stack-overflow
    // wasmtime at engine_type_index.
    val trackPath = Path()
    trackPath.addRoundRect(
        androidx.compose.ui.geometry.RoundRect(
            rect = ComposeRect(Offset(0f, 0f), Size(120f, 8f)),
            topLeft = androidx.compose.ui.geometry.CornerRadius(4f, 4f),
            topRight = androidx.compose.ui.geometry.CornerRadius(4f, 4f),
            bottomRight = androidx.compose.ui.geometry.CornerRadius(4f, 4f),
            bottomLeft = androidx.compose.ui.geometry.CornerRadius(4f, 4f),
        )
    )
    trackPath.rewind()

    // ── Compose-level Rect/Size pulled from ui-geometry through ui-graphics
    val rect = ComposeRect(Offset(10f, 20f), Size(100f, 50f))

    logMessage(
        "compose-ui-graphics smoke: " +
        "Color.Red=${red}, custom.value=0x${packed.toString(16)}, mixed-rgb=(${mixed.red},${mixed.green},${mixed.blue}), " +
        "brushes=${linearBrush::class.simpleName}/${radialBrush::class.simpleName}/${sweepBrush::class.simpleName}, " +
        "paint.style=${paint.style} blend=${paint.blendMode}, " +
        "path: empty=${pathIsEmpty} convex=${pathIsConvex} bounds=${pathBounds}, " +
        "rect=${rect}"
    )
}
