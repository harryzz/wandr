package testapp.compose

import androidx.compose.runtime.AbstractApplier

// Routes Compose's tree-mutation operations (insert/remove/move) to our
// CanvasNode tree. The Composer drives this during composition; we don't
// invoke it manually.
class WitCanvasApplier(root: GroupNode) : AbstractApplier<CanvasNode>(root) {

    override fun insertTopDown(index: Int, instance: CanvasNode) {
        // Top-down isn't useful for our flat draw model; defer everything to
        // bottom-up where we have correct ordering.
    }

    override fun insertBottomUp(index: Int, instance: CanvasNode) {
        current.children.add(index, instance)
    }

    override fun remove(index: Int, count: Int) {
        current.children.remove(index, count)
    }

    override fun move(from: Int, to: Int, count: Int) {
        current.children.move(from, to, count)
    }

    override fun onClear() {
        (root as GroupNode).children.clear()
    }
}
