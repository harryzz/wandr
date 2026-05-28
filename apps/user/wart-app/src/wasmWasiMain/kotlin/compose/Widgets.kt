package testapp.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ComposeNode

// Composable primitives. Each emits one CanvasNode via ComposeNode.
// The Compose Compiler tracks parameters for restartability — when state
// changes, only the affected primitives recompose and update their nodes.

@Composable
fun Group(
    x: Float = 0f,
    y: Float = 0f,
    content: @Composable () -> Unit,
) {
    ComposeNode<GroupNode, WitCanvasApplier>(
        factory = ::GroupNode,
        update = {
            set(x) { this.x = it }
            set(y) { this.y = it }
        },
        content = content,
    )
}

/**
 * Stacks children vertically with optional [spacing] between them.
 * Children should position themselves at (0, 0) within their slot;
 * Column supplies the offset based on each child's reported height.
 */
@Composable
fun Column(
    x: Float = 0f,
    y: Float = 0f,
    spacing: Float = 0f,
    content: @Composable () -> Unit,
) {
    ComposeNode<StackNode, WitCanvasApplier>(
        factory = ::StackNode,
        update = {
            set(x)       { this.x = it }
            set(y)       { this.y = it }
            set(spacing) { this.spacing = it }
            // direction is set once; the factory default is Vertical so this is a no-op,
            // but we set it explicitly so a node reused from a Row → Column swap is correct.
            set(Direction.Vertical) { this.direction = it }
        },
        content = content,
    )
}

/** Stacks children horizontally with optional [spacing]. */
@Composable
fun Row(
    x: Float = 0f,
    y: Float = 0f,
    spacing: Float = 0f,
    content: @Composable () -> Unit,
) {
    ComposeNode<StackNode, WitCanvasApplier>(
        factory = ::StackNode,
        update = {
            set(x)       { this.x = it }
            set(y)       { this.y = it }
            set(spacing) { this.spacing = it }
            set(Direction.Horizontal) { this.direction = it }
        },
        content = content,
    )
}

/**
 * Overlays children at the same origin (no automatic stacking). Functionally
 * equivalent to `Group` but reads better at call sites that mean "stack on z".
 */
@Composable
fun Box(
    x: Float = 0f,
    y: Float = 0f,
    content: @Composable () -> Unit,
) {
    Group(x = x, y = y, content = content)
}

/**
 * Imperative escape hatch: render arbitrary Skia draw calls inside a Compose
 * tree. Reports the given [width] and [height] to layout. The [block] runs
 * every frame; capture animated state by reading it from the call site so a
 * recomposition refreshes the lambda.
 */
@Composable
fun RawDraw(
    width: Float,
    height: Float,
    block: (org.jetbrains.skia.Canvas) -> Unit,
) {
    ComposeNode<RawDrawNode, WitCanvasApplier>(
        factory = ::RawDrawNode,
        update = {
            set(width)  { this.width  = it }
            set(height) { this.height = it }
            set(block)  { this.block  = it }
        },
    )
}

@Composable
fun Rect(
    x: Float,
    y: Float,
    width: Float,
    height: Float,
    color: Int = 0xFF000000.toInt(),
) {
    ComposeNode<RectNode, WitCanvasApplier>(
        factory = ::RectNode,
        update = {
            set(x)      { this.x = it }
            set(y)      { this.y = it }
            set(width)  { this.width = it }
            set(height) { this.height = it }
            set(color)  { this.color = it }
        },
    )
}

@Composable
fun Text(
    text: String,
    x: Float,
    y: Float,
    fontSize: Float = 14f,
    color: Int = 0xFFFFFFFF.toInt(),
) {
    ComposeNode<TextNode, WitCanvasApplier>(
        factory = ::TextNode,
        update = {
            set(text)     { this.text = it }
            set(x)        { this.x = it }
            set(y)        { this.y = it }
            set(fontSize) { this.fontSize = it }
            set(color)    { this.color = it }
        },
    )
}

@Composable
fun OnClick(
    x: Float,
    y: Float,
    width: Float,
    height: Float,
    onTap: () -> Unit = {},
    onMove: (Float, Float) -> Unit = { _, _ -> },
    onRelease: () -> Unit = {},
    onScroll: (Float, Float) -> Unit = { _, _ -> },
) {
    ComposeNode<HitNode, WitCanvasApplier>(
        factory = ::HitNode,
        update = {
            set(x)         { this.x = it }
            set(y)         { this.y = it }
            set(width)     { this.width = it }
            set(height)    { this.height = it }
            set(onTap)     { this.onTap = it }
            set(onMove)    { this.onMove = it }
            set(onRelease) { this.onRelease = it }
            set(onScroll)  { this.onScroll = it }
        },
    )
}
