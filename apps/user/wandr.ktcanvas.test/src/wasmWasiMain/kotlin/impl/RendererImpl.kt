// Stage-1 spike scene: every element exercises one canonical-ABI category
// of the generated wasi:canvas binding (see wit/ktcanvas-test.wit).
package impl

import bindings.Draw
import bindings.Embedding
import bindings.Layout
import bindings.Renderer
import bindings.Types

private fun fill(
    color: UInt,
    shader: Types.Shader? = null,
    blur: Types.MaskBlur? = null,
    alpha: UByte = 255u.toUByte(),
) = Types.Paint(
    Types.PaintStyle.FILL, color, alpha, Types.BlendMode.SRC_OVER,
    true, shader, 0f, Types.StrokeCap.BUTT, Types.StrokeJoin.MITER, 4f, blur,
    null,
)

private fun stroke(color: UInt, width: Float) = Types.Paint(
    Types.PaintStyle.STROKE, color, 255u.toUByte(), Types.BlendMode.SRC_OVER,
    true, null, width, Types.StrokeCap.ROUND, Types.StrokeJoin.ROUND, 4f, null,
    null,
)

private fun rrect(r: Types.Rect, radius: Float): Types.RoundedRect {
    val p = Types.Point(radius, radius)
    return Types.RoundedRect(r, p, p, p, p)
}

private fun textStyle(size: Float, color: UInt, weight: UInt = 400u) =
    Layout.TextStyle("", size, weight, false, color, 0f, 0f)

// 5-point star in a 100x100 unit box (shape definition; placed/sized by
// canvas transform from real geometry).
private const val STAR_PATH =
    "M50 5 L61 38 L96 38 L68 59 L79 92 L50 72 L21 92 L32 59 L4 38 L39 38 Z"

object RendererImpl : Renderer {
    // Lazy per-instance state: imports may only be called once the host has
    // the instance up (first render-frame), never at module init.
    private var ctx: Embedding.CanvasContext? = null
    private var gfx: Draw.Graphics? = null

    // Geometry-derived caches, keyed by the width they were built for.
    private var cachedW = -1f
    private var cardShader: Types.Shader? = null
    private var checkerImage: Types.Image? = null

    private var frame = 0

    private fun context(): Embedding.CanvasContext =
        ctx ?: Embedding.Import.getContext().also { ctx = it }

    private fun graphics(): Draw.Graphics =
        gfx ?: context().graphics().also { gfx = it }

    /** Rebuild the size-dependent resources (gradient shader, checkerboard
     *  image) when the surface width changes — exercises resource drop +
     *  re-create on top of the steady per-frame churn. */
    private fun rebuildSizedResources(w: Float, h: Float) {
        if (w == cachedW) return
        cardShader?.close()
        checkerImage?.close()

        val margin = w * 0.06f
        val card = Types.Rect(margin, h * 0.08f, w - 2f * margin, h * 0.18f)
        cardShader = graphics().linearGradient(
            Types.Point(card.x, card.y),
            Types.Point(card.x + card.width, card.y + card.height),
            listOf(0.0f to 0xFF7C4DFFu, 1.0f to 0xFF00BCD4u),
            Types.TileMode.CLAMP,
        )

        // Offscreen → snapshot → image (result<image> lift + offscreen drop).
        val side = (w * 0.12f).toUInt().coerceAtLeast(2u)
        val off = graphics().newOffscreen(side, side)
        val half = side.toFloat() / 2f
        off.drawPaint(fill(0xFFFFC107u))
        off.drawRect(Types.Rect(half, 0f, half, half), fill(0xFF263238u))
        off.drawRect(Types.Rect(0f, half, half, half), fill(0xFF263238u))
        checkerImage = off.snapshot().getOrThrow()
        off.close()

        cachedW = w
    }

    override fun renderFrame(nanos: ULong) {
        frame += 1
        val c = context()
        val cv = c.getCurrentBuffer()
        val w = cv.width()
        val h = cv.height()
        rebuildSizedResources(w, h)

        val margin = w * 0.06f
        val card = Types.Rect(margin, h * 0.08f, w - 2f * margin, h * 0.18f)
        val corner = w * 0.04f

        // 1. flat-path paint (≤16 args): background fill.
        cv.drawPaint(fill(0xFF101418u))

        // 5. option<mask-blur>: soft shadow under the card.
        val shadow = Types.Rect(card.x, card.y + w * 0.015f, card.width, card.height)
        cv.drawRoundedRect(
            rrect(shadow, corner),
            fill(0xFF000000u, blur = Types.MaskBlur(Types.BlurStyle.NORMAL, w * 0.02f), alpha = 160u.toUByte()),
        )

        // 3. shader borrow inside the spilled paint blob: gradient card.
        cv.drawRoundedRect(rrect(card, corner), fill(0xFF000000u, shader = cardShader))

        // 2. plain spilled blob (>16 flat args, no shader): accent bar.
        val barY = card.y + card.height + h * 0.02f
        cv.drawRect(Types.Rect(margin, barY, card.width, h * 0.012f), fill(0xFF00E676u))

        // 4. string + enum: SVG star, placed by transform, spinning from the
        // host-provided frame clock (never currentNanoTime — realloc trap).
        val starSide = w * 0.28f
        cv.save()
        cv.translate(w / 2f, barY + h * 0.05f + starSide / 2f)
        cv.rotate(((nanos / 1_000_000uL).toLong() % 36000L).toFloat() / 100f)
        cv.scale(starSide / 100f, starSide / 100f)
        cv.translate(-50f, -50f)
        cv.drawPath(STAR_PATH, Types.FillRule.NONZERO, fill(0xFFFFAB40u))
        cv.restore()

        // 6. layout: title inside the card + wrapped body; lines() drives
        // baseline tick marks (proves the list<record> lift carries real
        // metrics, not garbage).
        val titleSize = h * 0.028f
        run {
            val b = Layout.ParagraphBuilder.new(
                textStyle(titleSize, 0xFFFFFFFFu, weight = 700u), Layout.Align.CENTER,
            )
            b.addText("wasi:canvas × Kotlin")
            val p = Layout.ParagraphBuilder.build(b)
            p.layout(card.width)
            p.paint(cv, Types.Point(card.x, card.y + (card.height - p.height()) / 2f))
            p.close()
        }
        run {
            val bodyTop = barY + h * 0.05f + starSide + h * 0.04f
            val b = Layout.ParagraphBuilder.new(
                textStyle(h * 0.021f, 0xFFB0BEC5u), Layout.Align.START,
            )
            b.addText(
                "Every element on this screen crossed the wasi:canvas draft " +
                "through bindings generated by the Kotlin wit-bindgen fork: " +
                "spilled paint records, SVG path strings, gradient stop lists, " +
                "and these baseline ticks read back from paragraph.lines().",
            )
            val p = Layout.ParagraphBuilder.build(b)
            p.layout(card.width)
            p.paint(cv, Types.Point(card.x, bodyTop))
            for (line in p.lines()) {
                val y = bodyTop + line.baseline
                cv.drawLine(
                    Types.Point(margin * 0.25f, y),
                    Types.Point(margin * 0.75f, y),
                    stroke(0xFF00E676u, h * 0.002f),
                )
            }
            // 7. per-frame create/close churn (this paragraph + the frame
            // counter below) shakes out drop bugs at 60 fps.
            val bodyBottom = bodyTop + p.height()
            p.close()

            val fb = Layout.ParagraphBuilder.new(
                textStyle(h * 0.018f, 0xFF80DEEAu), Layout.Align.START,
            )
            fb.addText("frame $frame")
            val fp = Layout.ParagraphBuilder.build(fb)
            fp.layout(card.width)
            fp.paint(cv, Types.Point(card.x, bodyBottom + h * 0.02f))
            fp.close()
        }

        // 8. image draw (snapshot of the offscreen checkerboard).
        checkerImage?.let { img ->
            cv.drawImage(
                img,
                Types.Point(margin, h - margin - w * 0.12f),
                Types.Sampling(Types.FilterMode.LINEAR, Types.MipmapMode.NONE),
                fill(0xFFFFFFFFu),
            )
        }

        cv.close()
        c.present()
    }

    override fun onResize(w: UInt, h: UInt) {
        // Geometry is re-derived from the buffer every frame; just invalidate
        // the size-keyed caches.
        cachedW = -1f
    }

    override fun onPointerEvent(kind: Renderer.PointerKind, x: Float, y: Float) {}
    override fun onKeyEvent(kind: Renderer.KeyKind, keyCode: UInt) {}
    override fun onScheduledCallback(callbackId: UInt) {}
    override fun onPointerEventV2(pointerId: UInt, kind: Renderer.PointerKind, x: Float, y: Float, pressure: Float) {}
    override fun onKeyEventV2(kind: Renderer.KeyKind, codePoint: UInt, keyId: UInt) {}
    override fun onLifecycleChanged(state: UInt) {}
}

fun main() {
    // Never invoked — reactor component (renderer exports only);
    // binaries.executable() just needs an entry point to compile.
}
