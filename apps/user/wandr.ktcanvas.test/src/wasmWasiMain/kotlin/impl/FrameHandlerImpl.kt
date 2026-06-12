// Stage-1 spike scene, upgraded to wasi:canvas@0.0.2 (task 103 path B):
// every 0.0.2 delta has a visible element — scene layer (spin WITHOUT
// re-record), paint color-filter tint, builder setters (max-lines +
// ellipsis + did-exceed), gradient local param, and the 0.0.2
// pointer-handler export (device/button/buttons readout on screen);
// frames arrive via the 0.0.2 frame-handler export (legacy renderer retired).
package impl

import bindings.Draw
import bindings.Embedding
import bindings.Layout
import bindings.PointerHandler
import bindings.FrameHandler
import bindings.Scene
import bindings.Types
import kotlin.math.cos
import kotlin.math.sin

private fun fill(
    color: UInt,
    shader: Types.Shader? = null,
    blur: Types.MaskBlur? = null,
    alpha: UByte = 255u.toUByte(),
    filter: Types.ColorFilter? = null,
) = Types.Paint(
    Types.PaintStyle.FILL, color, alpha, Types.BlendMode.SRC_OVER,
    true, shader, 0f, Types.StrokeCap.BUTT, Types.StrokeJoin.MITER, 4f, blur,
    filter,
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
    Layout.TextStyle(
        "", size, weight, false, color,
        0f, 0f, 0f, null, listOf(), null,
    )

// 5-point star in a 100x100 unit box (shape definition; placed/sized by
// the LAYER transform — never re-recorded).
private const val STAR_PATH =
    "M50 5 L61 38 L96 38 L68 59 L79 92 L50 72 L21 92 L32 59 L4 38 L39 38 Z"

object FrameHandlerImpl : FrameHandler {
    private var ctx: Embedding.CanvasContext? = null
    private var gfx: Draw.Graphics? = null

    private var cachedW = -1f
    private var cardShader: Types.Shader? = null
    private var checkerImage: Types.Image? = null
    private var starLayer: Scene.Layer? = null
    private var starSide = 0f

    private var frame = 0
    internal var lastPtr: String = "pointer: (none yet)"

    private fun context(): Embedding.CanvasContext =
        ctx ?: Embedding.Import.getContext().also { ctx = it }

    private fun graphics(): Draw.Graphics =
        gfx ?: context().graphics().also { gfx = it }

    /** Size-keyed resources; the star LAYER's content is recorded exactly
     *  once per size — per-frame motion is set-transform only (the scene
     *  contract's whole point). */
    private fun rebuildSizedResources(w: Float, h: Float) {
        if (w == cachedW) return
        cardShader?.close()
        checkerImage?.close()
        starLayer?.close()

        val margin = w * 0.06f
        val card = Types.Rect(margin, h * 0.08f, w - 2f * margin, h * 0.18f)
        cardShader = graphics().linearGradient(
            Types.Point(card.x, card.y),
            Types.Point(card.x + card.width, card.y + card.height),
            listOf(0.0f to 0xFF7C4DFFu, 1.0f to 0xFF00BCD4u),
            Types.TileMode.CLAMP,
            null,
        )

        val side = (w * 0.12f).toUInt().coerceAtLeast(2u)
        val off = graphics().newOffscreen(side, side)
        val half = side.toFloat() / 2f
        off.drawPaint(fill(0xFFFFC107u))
        off.drawRect(Types.Rect(half, 0f, half, half), fill(0xFF263238u))
        off.drawRect(Types.Rect(0f, half, half, half), fill(0xFF263238u))
        checkerImage = off.snapshot().getOrThrow()
        off.close()

        // scene 0.0.2: the star is recorded ONCE into a layer; every frame
        // only mutates the layer transform (no re-record, no path re-send).
        starSide = w * 0.28f
        val rec = graphics().startRecording(Types.Rect(0f, 0f, 100f, 100f))
        rec.drawPath(STAR_PATH, Types.FillRule.NONZERO, fill(0xFFFFAB40u))
        val layer = Scene.Layer.new(graphics())
        layer.setContent(rec)
        layer.setBounds(Types.Rect(0f, 0f, 100f, 100f))
        starLayer = layer

        cachedW = w
    }

    /** Rotation about (cx,cy) composed with scale s and translation — the
     *  layer's full 3x3, computed guest-side from real geometry. */
    private fun starTransform(cx: Float, cy: Float, s: Float, deg: Float): Types.Transform {
        val r = deg * (3.1415927f / 180f)
        val c = cos(r) * s
        val n = sin(r) * s
        // T(cx,cy) · R·S · T(-50,-50)
        return Types.Transform(
            c, -n, cx + (-50f * c) + (50f * n),
            n, c, cy + (-50f * n) + (-50f * c),
            0f, 0f, 1f,
        )
    }

    override fun onFrame(nanos: ULong) {
        frame += 1
        val c = context()
        val cv = c.getCurrentBuffer()
        val w = cv.width()
        val h = cv.height()
        rebuildSizedResources(w, h)

        val margin = w * 0.06f
        val card = Types.Rect(margin, h * 0.08f, w - 2f * margin, h * 0.18f)
        val corner = w * 0.04f

        cv.drawPaint(fill(0xFF101418u))

        val shadow = Types.Rect(card.x, card.y + w * 0.015f, card.width, card.height)
        cv.drawRoundedRect(
            rrect(shadow, corner),
            fill(0xFF000000u, blur = Types.MaskBlur(Types.BlurStyle.NORMAL, w * 0.02f), alpha = 160u.toUByte()),
        )
        cv.drawRoundedRect(rrect(card, corner), fill(0xFF000000u, shader = cardShader))

        val barY = card.y + card.height + h * 0.02f
        cv.drawRect(Types.Rect(margin, barY, card.width, h * 0.012f), fill(0xFF00E676u))

        // scene: transform-only animation (content recorded once).
        starLayer?.let { layer ->
            val deg = ((nanos / 1_000_000uL).toLong() % 36000L).toFloat() / 100f
            layer.setTransform(
                starTransform(w / 2f, barY + h * 0.05f + starSide / 2f, starSide / 100f, deg)
            )
            Scene.Import.drawLayer(cv, layer)
        }

        val titleSize = h * 0.028f
        run {
            val b = Layout.ParagraphBuilder.new(textStyle(titleSize, 0xFFFFFFFFu, weight = 700u))
            b.setAlign(Layout.Align.CENTER)
            b.addText("wasi:canvas 0.0.2 × Kotlin")
            val p = Layout.ParagraphBuilder.build(b)
            p.layout(card.width)
            p.paint(cv, Types.Point(card.x, card.y + (card.height - p.height()) / 2f))
            p.close()
        }
        run {
            val bodyTop = barY + h * 0.05f + starSide + h * 0.04f
            // builder setters: clamp to 2 lines with an ellipsis; the
            // did-exceed flag is rendered so the truncation is provable.
            val b = Layout.ParagraphBuilder.new(textStyle(h * 0.021f, 0xFFB0BEC5u))
            b.setMaxLines(2u)
            b.setEllipsis("…")
            b.addText(
                "This paragraph is deliberately longer than two lines so the " +
                "0.0.2 builder setters (set-max-lines + set-ellipsis) take " +
                "effect and did-exceed-max-lines() returns true below the cut.",
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
            val bodyBottom = bodyTop + p.height()
            val exceeded = p.didExceedMaxLines()
            p.close()

            val fb = Layout.ParagraphBuilder.new(textStyle(h * 0.018f, 0xFF80DEEAu))
            fb.addText("frame $frame · truncated=$exceeded\n$lastPtr")
            val fp = Layout.ParagraphBuilder.build(fb)
            fp.layout(card.width)
            fp.paint(cv, Types.Point(card.x, bodyBottom + h * 0.02f))
            fp.close()
        }

        // color-filter: same checkerboard twice — raw, then tinted via
        // paint.filter = blend(cyan, src-in) (the dart:ui/Compose icon-tint
        // shape the 0.0.2 paint carries).
        checkerImage?.let { img ->
            val s = Types.Sampling(Types.FilterMode.LINEAR, Types.MipmapMode.NONE)
            cv.drawImage(img, Types.Point(margin, h - margin - w * 0.12f), s, fill(0xFFFFFFFFu))
            cv.drawImage(
                img,
                Types.Point(margin * 2f + w * 0.12f, h - margin - w * 0.12f),
                s,
                fill(
                    0xFFFFFFFFu,
                    filter = Types.ColorFilter.Blend(
                        Types.ColorBlend(0xFF00E5FFu, Types.BlendMode.SRC_IN)
                    ),
                ),
            )
        }

        cv.close()
        c.present()
    }

    override fun onResize(w: UInt, h: UInt) {
        cachedW = -1f
    }

}

/// 0.0.2 pointer export: renders the union record live (device kind, the
/// changed button, the held set, hover enter/leave) — the on-screen proof
/// that flags + new enums lower correctly through the Kotlin generator.
object PointerHandlerImpl : PointerHandler {
    override fun onPointer(ev: PointerHandler.PointerEvent) {
        val held = buildString {
            if (ev.buttons.primary) append("P")
            if (ev.buttons.secondary) append("S")
            if (ev.buttons.middle) append("M")
            if (ev.buttons.back) append("B")
            if (ev.buttons.forward) append("F")
        }.ifEmpty { "-" }
        val scroll = if (ev.kind == PointerHandler.Kind.SCROLL)
            " d=(${ev.scrollDx.toInt()},${ev.scrollDy.toInt()})" else ""
        FrameHandlerImpl.lastPtr =
            "pointer: ${ev.kind.name.lowercase()} ${ev.device.name.lowercase()} " +
            "btn=${ev.button.name.lowercase()} held=$held " +
            "(${ev.x.toInt()},${ev.y.toInt()})$scroll"
    }
}

fun main() {
    // Never invoked — reactor component (renderer + pointer exports only);
    // binaries.executable() just needs an entry point to compile.
}
