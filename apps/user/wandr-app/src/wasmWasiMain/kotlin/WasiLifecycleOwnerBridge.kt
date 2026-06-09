package testapp

import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import org.jetbrains.skiko.wasi.WasiLifecycle
import org.jetbrains.skiko.wasi.wit.Lifecycle as WitLifecycle

/**
 * Adapts the host-driven activity lifecycle (delivered via WIT
 * `renderer.on-lifecycle-changed` and surfaced through [WasiLifecycle]) into
 * the [androidx.lifecycle.LifecycleOwner] / [Lifecycle] contract that Compose
 * Multiplatform expects via `LocalLifecycleOwner`.
 *
 * Construct once at app startup and provide via
 * `CompositionLocalProvider(LocalLifecycleOwner provides this)` at the scene
 * root. State is seeded from `WasiLifecycle.currentState()` and kept in sync
 * via an installed [WasiLifecycle.Observer].
 */
class WasiLifecycleOwnerBridge : LifecycleOwner {
    private val registry = LifecycleRegistry(this)
    override val lifecycle: Lifecycle = registry

    init {
        registry.currentState = mapState(WasiLifecycle.currentState())
        WasiLifecycle.addObserver { state ->
            registry.currentState = mapState(state)
        }
    }

    private fun mapState(s: WitLifecycle.State): Lifecycle.State = when (s) {
        WitLifecycle.State.INITIALIZED -> Lifecycle.State.INITIALIZED
        WitLifecycle.State.CREATED     -> Lifecycle.State.CREATED
        WitLifecycle.State.STARTED     -> Lifecycle.State.STARTED
        WitLifecycle.State.RESUMED     -> Lifecycle.State.RESUMED
        // androidx.lifecycle has no PAUSED/STOPPED *states* — those are
        // transient events; the corresponding stable states are STARTED and
        // CREATED respectively.
        WitLifecycle.State.PAUSED      -> Lifecycle.State.STARTED
        WitLifecycle.State.STOPPED     -> Lifecycle.State.CREATED
        WitLifecycle.State.DESTROYED   -> Lifecycle.State.DESTROYED
    }
}
