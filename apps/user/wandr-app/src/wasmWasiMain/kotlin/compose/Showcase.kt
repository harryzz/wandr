package testapp.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import org.jetbrains.skia.BlendMode
import org.jetbrains.skia.ClipMode
import org.jetbrains.skia.ColorFilter
import org.jetbrains.skia.Font
import org.jetbrains.skia.Image
import org.jetbrains.skia.Paint
import org.jetbrains.skia.PaintMode
import org.jetbrains.skia.PaintStrokeCap
import org.jetbrains.skia.Path
import org.jetbrains.skia.PathBuilder
import org.jetbrains.skia.RRect
import org.jetbrains.skia.Rect as SRect
import org.jetbrains.skia.Shader
import org.jetbrains.skia.TextBlob
import org.jetbrains.skia.TextBlobBuilder
import org.jetbrains.skia.Typeface
import org.jetbrains.skia.paragraph.Paragraph
import org.jetbrains.skia.paragraph.ParagraphBuilder
import org.jetbrains.skia.paragraph.ParagraphStyle
import org.jetbrains.skia.paragraph.TextStyle as ParagraphTextStyle
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

// Top-level demo. Static sections (SVG paths, fonts, gradients, etc.) do
// their per-section allocations ONCE in `remember { … }` and reuse them
// every frame; their `RawDraw` block only references those cached values.
// Animated sections (`SectionSaveLayer`, `SectionDrawRRect`) have their own
// `LaunchedEffect { withFrameNanos { … } }` so only THEY recompose per frame
// — the other sections stay quiescent after initial composition.
@Composable
fun Showcase(
    screenWidth: Float,
    pointerX: Float,
    pointerY: Float,
    pointerDown: Boolean,
    lastKey: Int,
    checkerImg: Image,
    whiteImg: Image,
) {
    val s = screenWidth / 360f
    fun sp(v: Float): Float = v * s
    val margin = sp(12f)

    Column(x = margin, y = margin, spacing = sp(8f)) {
        HeaderCard(s, screenWidth, margin, pointerX, pointerY, pointerDown, lastKey)
        SectionSvgPaths(s)
        SectionSaveLayer(s, screenWidth, margin)
        SectionDrawRRect(s)
        SectionFontStyles(s)
        SectionTask08(s)
        SectionTask09(s)
        SectionTask10(s)
        SectionTask11(s)
        SectionTask12_14(s, checkerImg, whiteImg)
        TapCounterApp(screenWidth)
    }
}

// ── Header ────────────────────────────────────────────────────────────────────

@Composable
private fun HeaderCard(
    s: Float, screenWidth: Float, margin: Float,
    pointerX: Float, pointerY: Float, pointerDown: Boolean, lastKey: Int,
) {
    fun sp(v: Float) = v * s
    val cardH = sp(38f)
    val cardW = screenWidth - margin * 2f
    val titleFont = remember(s) { Font(size = sp(13f), weight = 700) }
    val statusFont = remember(s) { Font(size = sp(9f)) }
    val cardPaint = remember { Paint().apply { color = 0xFF0F3460.toInt(); isAntiAlias = true } }
    val titlePaint = remember { Paint().apply { color = 0xFFE2E8F0.toInt(); isAntiAlias = true } }
    val statusActive = remember { Paint().apply { color = 0xFF00FF88.toInt(); isAntiAlias = true } }
    val statusIdle   = remember { Paint().apply { color = 0xFF94A3B8.toInt(); isAntiAlias = true } }
    val cardRect = remember(s, cardW) { RRect.makeXYWH(0f, 0f, cardW, cardH, sp(8f)) }

    RawDraw(width = cardW, height = cardH) { canvas ->
        canvas.drawRRect(cardRect, cardPaint)
        canvas.drawString("New WIT canvas features", sp(12f), sp(26f), titleFont, titlePaint)
        val keyStr = if (lastKey >= 0) "#$lastKey" else "--"
        canvas.drawString(
            "touch(${pointerX.toInt()},${pointerY.toInt()})  key:$keyStr",
            sp(190f), sp(26f), statusFont,
            if (pointerDown) statusActive else statusIdle
        )
    }
}

// ── SVG paths ────────────────────────────────────────────────────────────────

@Composable
private fun SectionSvgPaths(s: Float) {
    fun sp(v: Float) = v * s
    // Cache parsed SVG paths and paints — these are constant.
    val pHeart   = remember { Path.makeFromSVGString("M 20 12 C 20 6 14 2 9 6 C 4 10 4 18 20 30 C 36 18 36 10 31 6 C 26 2 20 6 20 12 Z") }
    val pArrow   = remember { Path.makeFromSVGString("M 0 8 L 18 8 L 18 1 L 30 14 L 18 27 L 18 20 L 0 20 Z") }
    val pDiamond = remember { Path.makeFromSVGString("M 16 0 L 32 16 L 16 32 L 0 16 Z") }
    val pPacman  = remember { Path.makeFromSVGString("M 20 20 L 36 8 A 16 16 0 1 0 36 32 Z") }

    val labelFont = remember(s) { Font(size = sp(11f)) }
    val labelPaint = remember { Paint().apply { color = 0xFF94A3B8.toInt() } }
    val redFill    = remember { Paint().apply { color = 0xFFE94560.toInt(); isAntiAlias = true } }
    val cyanFill   = remember { Paint().apply { color = 0xFF00D4FF.toInt(); isAntiAlias = true } }
    val goldDimFill = remember { Paint().apply { color = 0x44FFD700.toInt(); isAntiAlias = true } }
    val goldStroke  = remember { Paint().apply {
        color = 0xFFFFD700.toInt(); mode = PaintMode.STROKE; strokeWidth = 2f; isAntiAlias = true
    } }
    val goldFill   = remember { Paint().apply { color = 0xFFFFD700.toInt(); isAntiAlias = true } }

    RawDraw(width = sp(360f), height = sp(70f)) { canvas ->
        canvas.drawString("SVG path strings (makeFromSVGString)",
            0f, sp(12f), labelFont, labelPaint)
        canvas.save(); canvas.translate(0f, sp(20f)); canvas.scale(s, s)
        canvas.drawPath(pHeart, redFill)
        canvas.restore()
        canvas.save(); canvas.translate(sp(55f), sp(25f)); canvas.scale(s, s)
        canvas.drawPath(pArrow, cyanFill)
        canvas.restore()
        canvas.save(); canvas.translate(sp(110f), sp(20f)); canvas.scale(s, s)
        canvas.drawPath(pDiamond, goldDimFill)
        canvas.drawPath(pDiamond, goldStroke)
        canvas.restore()
        canvas.save(); canvas.translate(sp(170f), sp(20f)); canvas.scale(s, s)
        canvas.drawPath(pPacman, goldFill)
        canvas.restore()
    }
}

// ── saveLayer (animated alpha) ───────────────────────────────────────────────

@Composable
private fun SectionSaveLayer(s: Float, screenWidth: Float, margin: Float) {
    fun sp(v: Float) = v * s
    val layerH = sp(64f)
    val sectionH = sp(20f) + layerH
    val sectionW = screenWidth - margin * 2f

    // Time-driven alpha local to THIS section. Other sections don't recompose.
    var alpha by remember { mutableStateOf(127) }
    LaunchedEffect(Unit) {
        while (true) withFrameNanos { now ->
            val ms = ((now / 1_000_000L) % 36_000L).toFloat()
            alpha = (127 + 127 * sin(ms.toDouble() * PI / 1500.0))
                .toInt().coerceIn(60, 255)
        }
    }

    val labelFont   = remember(s) { Font(size = sp(12f)) }
    val labelPaint  = remember { Paint().apply { color = 0xFF94A3B8.toInt(); isAntiAlias = true } }
    val cyanPaint   = remember { Paint().apply { color = 0xFF00D4FF.toInt(); isAntiAlias = true } }
    val redPaint    = remember { Paint().apply { color = 0xFFE94560.toInt(); isAntiAlias = true } }
    val whitePaint  = remember { Paint().apply { color = 0xFFFFFFFF.toInt(); isAntiAlias = true } }
    val rect1 = remember(s) { RRect.makeXYWH(sp(4f),  sp(8f), sp(130f), sp(48f), sp(10f)) }
    val rect2 = remember(s) { RRect.makeXYWH(sp(90f), sp(8f), sp(130f), sp(48f), sp(10f)) }
    val layerBounds = remember(s, sectionW) { SRect.makeXYWH(0f, sp(20f), sectionW, layerH) }

    RawDraw(width = sectionW, height = sectionH) { canvas ->
        canvas.drawString("saveLayer + restoreToCount", 0f, sp(12f), labelFont, labelPaint)
        // alpha-driven Paint must be created per frame because alpha changes; keep it tiny.
        val layerPaint = Paint().apply { this.alpha = alpha }
        val saveCount = canvas.saveLayer(layerBounds, layerPaint)
        canvas.drawRRect(rect1, cyanPaint)
        canvas.drawRRect(rect2, redPaint)
        canvas.drawString("alpha: $alpha", sp(235f), sp(60f), labelFont, whitePaint)
        canvas.restoreToCount(saveCount)
    }
}

// ── drawRRect with rotating square ───────────────────────────────────────────

@Composable
private fun SectionDrawRRect(s: Float) {
    fun sp(v: Float) = v * s
    val rectH = sp(40f)
    val sectionH = sp(24f) + rectH + sp(16f)

    var angle by remember { mutableStateOf(0f) }
    LaunchedEffect(Unit) {
        while (true) withFrameNanos { now ->
            val ms = ((now / 1_000_000L) % 36_000L).toFloat()
            angle = (ms * 0.09f) % 360f
        }
    }

    val labelFont = remember(s) { Font(size = sp(12f)) }
    val labelPaint = remember { Paint().apply { color = 0xFF94A3B8.toInt(); isAntiAlias = true } }
    val purpleFill = remember { Paint().apply { color = 0xFF533483.toInt(); isAntiAlias = true } }
    val cyanStroke = remember { Paint().apply {
        color = 0xFF00D4FF.toInt(); mode = PaintMode.STROKE
        strokeWidth = sp(2.5f); isAntiAlias = true
    } }
    val redFill = remember { Paint().apply { color = 0xFFFF6B6B.toInt(); isAntiAlias = true } }
    val rrect1 = remember(s) { RRect.makeXYWH(0f, sp(24f), sp(90f), rectH, sp(20f)) }
    val rrect2 = remember(s) { RRect.makeXYWH(sp(100f), sp(24f), sp(90f), rectH, sp(8f)) }
    val sq = sp(24f)
    val rrectRotating = remember(s) { RRect.makeXYWH(-sq, -sq, sq * 2f, sq * 2f, sp(6f)) }

    RawDraw(width = sp(330f), height = sectionH) { canvas ->
        canvas.drawString("drawRRect  (fill / stroke / rotating)",
            0f, sp(14f), labelFont, labelPaint)
        canvas.drawRRect(rrect1, purpleFill)
        canvas.drawRRect(rrect2, cyanStroke)
        canvas.save()
        canvas.translate(sp(240f), sp(24f) + rectH / 2f)
        canvas.rotate(angle)
        canvas.drawRRect(rrectRotating, redFill)
        canvas.restore()
    }
}

// ── Font styles ──────────────────────────────────────────────────────────────

@Composable
private fun SectionFontStyles(s: Float) {
    fun sp(v: Float) = v * s
    val sectionH = sp(14f) + sp(16f) + sp(22f) * 2

    val labelFont   = remember(s) { Font(size = sp(12f)) }
    val labelPaint  = remember { Paint().apply { color = 0xFF94A3B8.toInt(); isAntiAlias = true } }
    val tagFont     = remember(s) { Font(size = sp(9f)) }
    val tagPaint    = remember { Paint().apply { color = 0xFF4B5563.toInt(); isAntiAlias = true } }
    val boldFont    = remember(s) { Font(size = sp(15f), weight = 700) }
    val italicFont  = remember(s) { Font(size = sp(15f), italic = true) }
    val boldPaint   = remember { Paint().apply { color = 0xFFE2E8F0.toInt(); isAntiAlias = true } }
    val italicPaint = remember { Paint().apply { color = 0xFF93C5FD.toInt(); isAntiAlias = true } }

    RawDraw(width = sp(330f), height = sectionH) { canvas ->
        canvas.drawString("font styles & typefaces", 0f, sp(14f), labelFont, labelPaint)
        var rowY = sp(30f)
        canvas.drawString("bold", 0f, rowY, tagFont, tagPaint)
        canvas.drawString("The quick brown fox jumps", sp(88f), rowY, boldFont, boldPaint)
        rowY += sp(22f)
        canvas.drawString("italic", 0f, rowY, tagFont, tagPaint)
        canvas.drawString("The quick brown fox jumps", sp(88f), rowY, italicFont, italicPaint)
    }
}

// ── Task 08 ──────────────────────────────────────────────────────────────────

@Composable
private fun SectionTask08(s: Float) {
    fun sp(v: Float) = v * s
    val sectionH = sp(74f)

    val labelFont = remember(s) { Font(size = sp(11f)) }
    val labelPaint = remember { Paint().apply { color = 0xFF94A3B8.toInt() } }
    val arcPaint = remember { Paint().apply {
        color = 0xFF00D4FF.toInt(); mode = PaintMode.STROKE
        strokeWidth = sp(4f); strokeCap = PaintStrokeCap.ROUND; isAntiAlias = true
    } }
    val drrectPaint = remember { Paint().apply { color = 0xFFE94560.toInt(); isAntiAlias = true } }
    val purplePaint = remember { Paint().apply { color = 0xFF533483.toInt() } }
    val whitePaint = remember { Paint().apply { color = 0xFFFFFFFF.toInt() } }
    val cyanPaint = remember { Paint().apply { color = 0xFF00D4FF.toInt(); isAntiAlias = true } }
    val redMultiply = remember { Paint().apply {
        color = 0xFFFF6B6B.toInt(); blendMode = BlendMode.MULTIPLY; isAntiAlias = true
    } }
    val clipText = remember(s) { Font(size = sp(11f)) }
    val arcRect = remember(s) { SRect.makeXYWH(0f, sp(16f), sp(48f), sp(48f)) }
    val drrectOuter = remember(s) { RRect.makeXYWH(sp(60f), sp(16f), sp(48f), sp(48f), sp(8f)) }
    val drrectInner = remember(s) { RRect.makeXYWH(sp(68f), sp(24f), sp(32f), sp(32f), sp(4f)) }
    val clipRrect = remember(s) { RRect.makeXYWH(sp(120f), sp(16f), sp(80f), sp(48f), sp(16f)) }
    val blendBg = remember(s) { SRect.makeXYWH(0f, 0f, sp(40f), sp(48f)) }
    val blendFg = remember(s) { SRect.makeXYWH(sp(10f), sp(8f), sp(40f), sp(48f)) }

    RawDraw(width = sp(330f), height = sectionH) { canvas ->
        canvas.drawString("task 08: arc / drrect / blendMode / clipRRect", 0f, sp(8f), labelFont, labelPaint)
        canvas.drawArc(arcRect, -90f, 270f, false, arcPaint)
        canvas.drawDRRect(drrectOuter, drrectInner, drrectPaint)
        canvas.save()
        canvas.clipRRect(clipRrect, ClipMode.INTERSECT, true)
        canvas.drawPaint(purplePaint)
        canvas.drawString("clipped", sp(124f), sp(44f), clipText, whitePaint)
        canvas.restore()
        canvas.save()
        canvas.translate(sp(210f), sp(16f))
        canvas.drawRect(blendBg, cyanPaint)
        canvas.drawRect(blendFg, redMultiply)
        canvas.restore()
    }
}

// ── Task 09 ──────────────────────────────────────────────────────────────────

@Composable
private fun SectionTask09(s: Float) {
    fun sp(v: Float) = v * s
    val sectionH = sp(74f)

    val labelFont = remember(s) { Font(size = sp(11f)) }
    val labelPaint = remember { Paint().apply { color = 0xFF94A3B8.toInt() } }
    val starFill = remember { Paint().apply { color = 0xFFFFD700.toInt(); isAntiAlias = true } }
    val clipBg = remember { Paint().apply { color = 0xFF0F3460.toInt() } }
    val clipFg = remember { Paint().apply { color = 0xFF00D4FF.toInt() } }
    val rrectStroke = remember { Paint().apply {
        color = 0xFFE94560.toInt(); mode = PaintMode.STROKE
        strokeWidth = sp(3f); isAntiAlias = true
    } }
    val clipFont = remember(s) { Font(size = sp(11f)) }
    // Star Path — ten vertices, alternating outer/inner radii.
    val starPath = remember(s) {
        Path().apply {
            val cx = sp(36f)
            val cy = sp(50f)
            val outer = sp(28f)
            val inner = sp(12f)
            for (i in 0..9) {
                val ang = -PI.toFloat() / 2f + i * PI.toFloat() / 5f
                val r = if (i % 2 == 0) outer else inner
                val px = cx + r * cos(ang)
                val py = cy + r * sin(ang)
                if (i == 0) moveTo(px, py) else lineTo(px, py)
            }
            close()
        }
    }
    val clipOval = remember(s) {
        Path().apply { addOval(SRect.makeXYWH(sp(80f), sp(18f), sp(70f), sp(56f))) }
    }
    val rrectPath = remember(s) {
        PathBuilder().apply {
            addRRect(RRect.makeXYWH(sp(160f), sp(18f), sp(80f), sp(56f), sp(16f)))
        }.build()
    }

    RawDraw(width = sp(330f), height = sectionH) { canvas ->
        canvas.drawString("task 09: drawPath / clipPath", 0f, sp(8f), labelFont, labelPaint)
        canvas.drawPath(starPath, starFill)
        canvas.save()
        canvas.clipPath(clipOval, ClipMode.INTERSECT, true)
        canvas.drawPaint(clipBg)
        canvas.drawString("clipped text", sp(82f), sp(50f), clipFont, clipFg)
        canvas.restore()
        canvas.drawPath(rrectPath, rrectStroke)
    }
}

// ── Task 10 — TextBlob (cached + disposed on unmount) ───────────────────────

@Composable
private fun SectionTask10(s: Float) {
    fun sp(v: Float) = v * s
    val labelFont = remember(s) { Font(size = sp(11f)) }
    val labelPaint = remember { Paint().apply { color = 0xFF94A3B8.toInt() } }
    val blobPaint = remember { Paint().apply { color = 0xFFE2E8F0.toInt(); isAntiAlias = true } }
    val multiBlob: TextBlob = remember(s) {
        val baselineY = sp(30f)
        TextBlobBuilder().apply {
            appendRun(Font(size = sp(16f), weight = 400), "Hello ", 0f, baselineY)
            appendRun(Font(size = sp(16f), weight = 700), "bold ",  sp(52f), baselineY)
            appendRun(Font(size = sp(16f), italic = true), "italic ", sp(96f), baselineY)
            appendRun(
                Font(Typeface.makeFromFile("/system/fonts/DroidSansMono.ttf"), sp(14f)),
                "mono", sp(148f), baselineY)
        }.build()
    }
    // Phase B: TextBlob is a pure guest value (runs drawn as host
    // paragraphs per draw) — nothing host-side to dispose.

    RawDraw(width = sp(330f), height = sp(34f)) { canvas ->
        canvas.drawString("task 10: TextBlobBuilder multi-run", 0f, sp(8f), labelFont, labelPaint)
        canvas.drawTextBlob(multiBlob, 0f, 0f, blobPaint)
    }
}

// ── Task 11 — Gradient shaders (cached + disposed) ──────────────────────────

@Composable
private fun SectionTask11(s: Float) {
    fun sp(v: Float) = v * s
    val labelFont = remember(s) { Font(size = sp(11f)) }
    val labelPaint = remember { Paint().apply { color = 0xFF94A3B8.toInt() } }
    val labelTextFont = remember(s) { Font(size = sp(13f), weight = 700) }
    val whiteFill = remember { Paint().apply { color = 0xFFFFFFFF.toInt(); isAntiAlias = true } }

    val linShader = remember(s) {
        Shader.makeLinearGradient(
            0f, sp(18f), sp(130f), sp(18f),
            intArrayOf(0xFF0F3460.toInt(), 0xFF00D4FF.toInt(), 0xFFE94560.toInt()))
    }
    val cx = sp(160f); val cy = sp(38f)
    val radShader = remember(s) {
        Shader.makeRadialGradient(cx, cy, sp(28f),
            intArrayOf(0xFFFFD700.toInt(), 0xFFFF6B6B.toInt(), 0xFF1A1A2E.toInt()))
    }
    val cx2 = sp(230f)
    val linShader2 = remember(s) {
        Shader.makeLinearGradient(cx2, sp(18f), cx2 + sp(100f), sp(58f),
            intArrayOf(0xFF533483.toInt(), 0xFFE94560.toInt()))
    }
    DisposableEffect(linShader, radShader, linShader2) {
        onDispose {
            linShader.discard(); radShader.discard(); linShader2.discard()
        }
    }
    val linPaint = remember(linShader) { Paint().apply { shader = linShader; isAntiAlias = true } }
    val radPaint = remember(radShader) { Paint().apply { shader = radShader; isAntiAlias = true } }
    val linPaint2 = remember(linShader2) { Paint().apply { shader = linShader2; isAntiAlias = true } }
    val rrect1 = remember(s) { RRect.makeXYWH(0f, sp(18f), sp(130f), sp(40f), sp(8f)) }
    val ovalRect = remember(s) { SRect.makeXYWH(cx - sp(28f), cy - sp(28f), sp(56f), sp(56f)) }
    val rrect2 = remember(s) { RRect.makeXYWH(cx2, sp(18f), sp(100f), sp(40f), sp(6f)) }

    RawDraw(width = sp(330f), height = sp(68f)) { canvas ->
        canvas.drawString("task 11: linear + radial gradient", 0f, sp(8f), labelFont, labelPaint)
        canvas.drawRRect(rrect1, linPaint)
        canvas.drawOval(ovalRect, radPaint)
        canvas.drawRRect(rrect2, linPaint2)
        canvas.drawString("gradient", cx2 + sp(8f), sp(44f), labelTextFont, whiteFill)
    }
}

// ── Tasks 12 / 13 / 14 — Image + ColorFilter + Paragraph (cached) ───────────

@Composable
private fun SectionTask12_14(s: Float, checkerImg: Image, whiteImg: Image) {
    fun sp(v: Float) = v * s
    val labelFont = remember(s) { Font(size = sp(10f)) }
    val labelPaint = remember { Paint().apply { color = 0xFF94A3B8.toInt() } }
    val cyanTint = remember { Paint().apply {
        colorFilter = ColorFilter.makeBlend(0xFF00D4FF.toInt(), BlendMode.MULTIPLY)
    } }
    val redTint = remember { Paint().apply {
        colorFilter = ColorFilter.makeBlend(0xFFE94560.toInt(), BlendMode.MULTIPLY)
    } }
    val invertFilter = remember { Paint().apply { colorFilter = ColorFilter.makeInvert() } }
    val alphaPaint = remember { Paint().apply { alpha = 100 } }

    val checkerSrc = remember(checkerImg) {
        SRect.makeWH(checkerImg.width.toFloat(), checkerImg.height.toFloat())
    }
    val checkerSrcQ = remember(checkerImg) {
        SRect.makeXYWH(0f, 0f, checkerImg.width / 2f, checkerImg.height / 2f)
    }
    val whiteSrc = remember(whiteImg) {
        SRect.makeWH(whiteImg.width.toFloat(), whiteImg.height.toFloat())
    }
    val checkerDst = remember(s) { SRect.makeXYWH(0f,        sp(12f), sp(28f), sp(28f)) }
    val zoomDst    = remember(s) { SRect.makeXYWH(sp(34f),   sp(12f), sp(32f), sp(32f)) }
    val alphaDst   = remember(s) { SRect.makeXYWH(sp(72f),   sp(12f), sp(28f), sp(28f)) }
    val t13X = sp(118f)
    val cyanDst    = remember(s) { SRect.makeXYWH(t13X,            sp(12f), sp(28f), sp(28f)) }
    val redDst     = remember(s) { SRect.makeXYWH(t13X + sp(34f),  sp(12f), sp(28f), sp(28f)) }
    val invertDst  = remember(s) { SRect.makeXYWH(t13X + sp(68f),  sp(12f), sp(28f), sp(28f)) }

    val t14X = sp(218f)
    val t14Width = sp(360f) - sp(12f) - t14X
    val paragraph: Paragraph = remember(s) {
        ParagraphBuilder(ParagraphStyle(), t14Width).apply {
            pushStyle(ParagraphTextStyle().apply {
                fontSize = sp(10f); color = 0xFF94A3B8.toInt() })
            addText("t14:")
            pop()
            pushStyle(ParagraphTextStyle().apply {
                fontSize = sp(12f); fontWeight = 700; color = 0xFF00D4FF.toInt() })
            addText(" Paragraph\nlayout works")
            pop()
        }.build().apply { layout(t14Width) }
    }
    DisposableEffect(paragraph) { onDispose { paragraph.close() } }

    RawDraw(width = sp(330f), height = sp(50f)) { canvas ->
        canvas.drawString("t12: imageRect  t13: colorFilter  t14: paragraph",
            0f, sp(8f), labelFont, labelPaint)
        canvas.drawImageRect(checkerImg, src = checkerSrc,  dst = checkerDst)
        canvas.drawImageRect(checkerImg, src = checkerSrcQ, dst = zoomDst)
        canvas.drawImageRect(checkerImg, src = checkerSrc,  dst = alphaDst, paint = alphaPaint)
        canvas.drawImageRect(whiteImg,   src = whiteSrc,    dst = cyanDst,  paint = cyanTint)
        canvas.drawImageRect(whiteImg,   src = whiteSrc,    dst = redDst,   paint = redTint)
        canvas.drawImageRect(checkerImg, src = checkerSrc,  dst = invertDst, paint = invertFilter)
        paragraph.paint(canvas, t14X, sp(10f))
    }
}
