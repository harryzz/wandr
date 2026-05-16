package testapp.compose

import androidx.compose.runtime.BroadcastFrameClock
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Composition
import androidx.compose.runtime.Recomposer
import androidx.compose.runtime.snapshots.Snapshot
import kotlin.coroutines.EmptyCoroutineContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.jetbrains.skia.Canvas

class WasiComposeScene {
    private val rootNode = GroupNode()
    private val applier = WitCanvasApplier(rootNode)
    private val frameClock = BroadcastFrameClock()
    private val recomposeContext = EmptyCoroutineContext + frameClock

    // Allocated lazily by setContent() and reset by dispose(). Keeping them
    // nullable lets setContent be called repeatedly (hot-reload-style) without
    // leaking the previous composition / coroutine scope.
    private var recomposer: Recomposer? = null
    private var composition: Composition? = null
    private var scope: CoroutineScope? = null

    // Pointer capture state. When DOWN hits a HitNode, we remember it as the
    // "captured" target. Subsequent MOVE/UP events route to that target until
    // UP releases the capture.
    private var captured: HitNode? = null
    private var lastX: Float = 0f
    private var lastY: Float = 0f

    // Top-level key handler; apps register one via setKeyHandler().
    private var keyHandler: ((Boolean, Int) -> Unit)? = null

    /**
     * Sets the composable root. Idempotent: if a previous composition is
     * still active it is disposed first so the new one starts clean.
     */
    fun setContent(content: @Composable () -> Unit) {
        dispose()
        val newRecomposer = Recomposer(recomposeContext)
        val newComposition = Composition(applier, newRecomposer)
        val newScope = CoroutineScope(recomposeContext)
        // Start the recomposer's main loop. It suspends on the frame clock;
        // each render() call resumes it via sendFrame().
        newScope.launch {
            newRecomposer.runRecomposeAndApplyChanges()
        }
        newComposition.setContent(content)
        recomposer = newRecomposer
        composition = newComposition
        scope = newScope
    }

    /**
     * Tears down the composition, recomposer, and coroutine scope. Safe to
     * call multiple times; safe to call before any setContent. Drops the
     * captured pointer / key handler so events go nowhere until the next
     * setContent().
     */
    fun dispose() {
        composition?.dispose()
        composition = null
        recomposer?.let {
            it.cancel()
            it.close()
        }
        recomposer = null
        scope?.cancel()
        scope = null
        captured = null
        keyHandler = null
        // Clear any node tree built up by the previous applier so the next
        // composition starts from an empty root.
        rootNode.children.clear()
    }

    fun render(canvas: Canvas, nanos: Long) {
        // Resume any pending withFrameNanos awaiters; this lets the recomposer
        // observe snapshot invalidations and apply changes via the applier.
        frameClock.sendFrame(nanos)
        rootNode.draw(canvas)
    }

    fun setKeyHandler(handler: (down: Boolean, keyCode: Int) -> Unit) {
        keyHandler = handler
    }

    /** Returns true if the point hit a HitNode and onTap fired. */
    fun dispatchPointerDown(x: Float, y: Float): Boolean {
        val hit = rootNode.hitTest(x, y) ?: return false
        captured = hit
        lastX = x; lastY = y
        hit.onTap()
        applyAndNotify()
        return true
    }

    /** Routes to the captured node's onMove with the delta since last event. */
    fun dispatchPointerMove(x: Float, y: Float): Boolean {
        val target = captured ?: return false
        val dx = x - lastX
        val dy = y - lastY
        lastX = x; lastY = y
        target.onMove(dx, dy)
        applyAndNotify()
        return true
    }

    /** Releases capture, fires onRelease on the captured node. */
    fun dispatchPointerUp(x: Float, y: Float): Boolean {
        val target = captured ?: return false
        captured = null
        target.onRelease()
        applyAndNotify()
        return true
    }

    /** Hit-tests fresh (no capture) and forwards the delta. */
    fun dispatchPointerScroll(x: Float, y: Float, dx: Float, dy: Float): Boolean {
        val hit = rootNode.hitTest(x, y) ?: return false
        hit.onScroll(dx, dy)
        applyAndNotify()
        return true
    }

    fun dispatchKeyEvent(down: Boolean, keyCode: Int): Boolean {
        val h = keyHandler ?: return false
        h(down, keyCode)
        applyAndNotify()
        return true
    }

    private fun applyAndNotify() {
        // State writes from event callbacks go to the global snapshot. Push
        // them through so the recomposer's apply observer sees the invalidations
        // before the next frame.
        Snapshot.sendApplyNotifications()
    }
}
