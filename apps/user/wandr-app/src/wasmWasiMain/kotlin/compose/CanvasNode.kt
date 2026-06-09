package testapp.compose

import org.jetbrains.skia.Canvas
import org.jetbrains.skia.Font
import org.jetbrains.skia.Paint
import org.jetbrains.skia.Rect

// Minimal canvas-tree node hierarchy. Each node maps to one drawing primitive
// or a transform/clip group. The applier mutates `children` as composition runs.

enum class Direction { Horizontal, Vertical }

sealed class CanvasNode {
    val children: MutableList<CanvasNode> = mutableListOf()
    abstract fun draw(canvas: Canvas)

    /**
     * Top-down hit test. Returns the deepest node whose `OnClick` rectangle
     * contains (px, py), or null if none. (px, py) are in the parent's frame.
     */
    open fun hitTest(px: Float, py: Float): HitNode? {
        for (i in children.indices.reversed()) {
            val hit = children[i].hitTest(px, py)
            if (hit != null) return hit
        }
        return null
    }

    /** Self-reported bounds in the parent's frame. Used by `StackNode` layout. */
    open val boundsWidth:  Float get() = children.maxOfOrNull { it.boundsWidth  } ?: 0f
    open val boundsHeight: Float get() = children.maxOfOrNull { it.boundsHeight } ?: 0f
}

class GroupNode : CanvasNode() {
    var x: Float = 0f
    var y: Float = 0f

    override fun draw(canvas: Canvas) {
        canvas.save()
        canvas.translate(x, y)
        children.forEach { it.draw(canvas) }
        canvas.restore()
    }

    override fun hitTest(px: Float, py: Float): HitNode? {
        // Translate point into local frame for descendants.
        val lx = px - x
        val ly = py - y
        for (i in children.indices.reversed()) {
            val hit = children[i].hitTest(lx, ly)
            if (hit != null) return hit
        }
        return null
    }
}

/**
 * Stacks children along [direction] with [spacing] between them. Each child's
 * `boundsWidth`/`boundsHeight` determines how much space it takes; the next
 * child is placed immediately after (plus spacing).
 *
 * `x`, `y` are the stack's own origin in its parent's frame. Children are
 * expected to position themselves at (0, 0) within their slot — the stack
 * supplies the offset.
 */
class StackNode : CanvasNode() {
    var direction: Direction = Direction.Vertical
    var spacing: Float = 0f
    var x: Float = 0f
    var y: Float = 0f

    override fun draw(canvas: Canvas) {
        canvas.save()
        canvas.translate(x, y)
        var offset = 0f
        for (child in children) {
            canvas.save()
            if (direction == Direction.Vertical) canvas.translate(0f, offset)
            else                                  canvas.translate(offset, 0f)
            child.draw(canvas)
            canvas.restore()
            offset += if (direction == Direction.Vertical) child.boundsHeight else child.boundsWidth
            offset += spacing
        }
        canvas.restore()
    }

    override fun hitTest(px: Float, py: Float): HitNode? {
        val lx = px - x
        val ly = py - y
        var offset = 0f
        // Walk in reverse so later (top-most) children win.
        val positions = ArrayList<Float>(children.size)
        for (child in children) {
            positions.add(offset)
            offset += if (direction == Direction.Vertical) child.boundsHeight else child.boundsWidth
            offset += spacing
        }
        for (i in children.indices.reversed()) {
            val o = positions[i]
            val cx = if (direction == Direction.Vertical) lx else lx - o
            val cy = if (direction == Direction.Vertical) ly - o else ly
            children[i].hitTest(cx, cy)?.let { return it }
        }
        return null
    }

    override val boundsWidth: Float
        get() = if (direction == Direction.Horizontal) {
            var sum = 0f
            for ((i, c) in children.withIndex()) {
                sum += c.boundsWidth
                if (i < children.size - 1) sum += spacing
            }
            sum
        } else {
            children.maxOfOrNull { it.boundsWidth } ?: 0f
        }

    override val boundsHeight: Float
        get() = if (direction == Direction.Vertical) {
            var sum = 0f
            for ((i, c) in children.withIndex()) {
                sum += c.boundsHeight
                if (i < children.size - 1) sum += spacing
            }
            sum
        } else {
            children.maxOfOrNull { it.boundsHeight } ?: 0f
        }
}

class RectNode : CanvasNode() {
    var x: Float = 0f
    var y: Float = 0f
    var width: Float = 0f
    var height: Float = 0f
    var color: Int = 0xFF000000.toInt()

    override fun draw(canvas: Canvas) {
        canvas.drawRect(
            Rect.makeXYWH(x, y, width, height),
            Paint().apply { color = this@RectNode.color; isAntiAlias = true }
        )
    }

    override val boundsWidth:  Float get() = x + width
    override val boundsHeight: Float get() = y + height
}

class TextNode : CanvasNode() {
    var text: String = ""
    var x: Float = 0f
    var y: Float = 0f
    var fontSize: Float = 14f
    var color: Int = 0xFFFFFFFF.toInt()

    override fun draw(canvas: Canvas) {
        canvas.drawString(
            text, x, y,
            Font(size = fontSize),
            Paint().apply { color = this@TextNode.color; isAntiAlias = true }
        )
    }

    // Rough estimates — Compose proper would use font metrics. For our demo,
    // assume ~0.6× monospace-ish width per char and ~1.2× line height.
    override val boundsWidth:  Float get() = x + text.length * fontSize * 0.6f
    override val boundsHeight: Float get() = y + fontSize * 1.2f
}

/**
 * Escape hatch: wraps a `(Canvas) -> Unit` block with explicit bounds so Column /
 * Row can stack it. Useful for porting imperative draw code into Compose
 * without writing a node + composable for every primitive (Path / Arc /
 * DRRect / TextBlob / Gradient / Image / Paragraph). The block runs every
 * frame; its captured state is refreshed on each recomposition.
 */
class RawDrawNode : CanvasNode() {
    var width: Float = 0f
    var height: Float = 0f
    var block: (Canvas) -> Unit = {}

    override fun draw(canvas: Canvas) {
        canvas.save()
        block(canvas)
        canvas.restore()
    }

    override val boundsWidth:  Float get() = width
    override val boundsHeight: Float get() = height
}

class HitNode : CanvasNode() {
    var x: Float = 0f
    var y: Float = 0f
    var width: Float = 0f
    var height: Float = 0f
    var onTap:     () -> Unit = {}
    /** Called after capture on each MOVE event with the delta since last call. */
    var onMove:    (Float, Float) -> Unit = { _, _ -> }
    /** Called when the captured pointer is released (UP). */
    var onRelease: () -> Unit = {}
    /** Called on SCROLL events that hit this node, with the delta. */
    var onScroll:  (Float, Float) -> Unit = { _, _ -> }

    override fun draw(canvas: Canvas) {
        // Hit nodes are invisible — they only participate in hit testing.
    }

    override fun hitTest(px: Float, py: Float): HitNode? =
        if (px >= x && px < x + width && py >= y && py < y + height) this else null

    // HitNode contributes 0 extent to layouts (overlay-only).
    override val boundsWidth:  Float get() = 0f
    override val boundsHeight: Float get() = 0f
}
