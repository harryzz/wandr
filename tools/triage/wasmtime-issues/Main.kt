@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
)

// Task 25 step 1 — Tightened reproducer. ZERO wart-side code in the
// suspend cycle: no WasiScheduler, no HashMap, no callback IDs, no
// per-tick lambda. Pure Kotlin/Wasm `suspendCoroutine` driven
// directly from the wart-host's per-frame WIT renderFrame call.
//
// Suspend cycle each "tick":
//   1. Coroutine calls `awaitNextFrame()`.
//   2. `suspendCoroutine { cont -> pendingNextFrame = cont }`
//      allocates a state-machine instance, stores cont in a single
//      global slot, returns to the dispatcher.
//   3. wart-host's render loop fires `render-frame(nanos)`.
//   4. `RendererExports.kt::__wasm_export_renderFrame` calls
//      `LeakReproDriver.tick()` (our override of the default body).
//   5. `LeakReproDriver.tick()` drains `pendingNextFrame` and resumes
//      the continuation. The state-machine instance from step 2
//      SHOULD now be unreachable.
//
// If this still leaks at task-24-step-1 rates (~9 MB/s), the bug is
// irrefutably in Kotlin/Wasm `suspend` codegen.

package testapp

import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.resume
import kotlin.coroutines.startCoroutine
import kotlin.coroutines.suspendCoroutine
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

// Single-slot park for the next-frame continuation.
private var pendingNextFrame: Continuation<Unit>? = null

/// Called directly from `RendererExports.kt::__wasm_export_renderFrame`
/// (overridden to bypass `RendererImpl` for this reproducer). No
/// HashMap, no callback IDs, no host-side scheduler. Just a direct
/// resume of the parked continuation.
internal object LeakReproDriver {
    fun tick() {
        val c = pendingNextFrame
        pendingNextFrame = null
        c?.resume(Unit)
    }
}

private suspend fun awaitNextFrame(): Unit = suspendCoroutine { cont ->
    pendingNextFrame = cont
    // Nothing else. The wart-host already calls our @WasmExport
    // renderFrame every frame; that IS our wake.
}

private var frameCount: Long = 0L

fun main() {
    WitCanvas.Import.logMessage(
        "leak-repro: starting tightened repro — no WasiScheduler, no lambda alloc, " +
        "no HashMap. Pure suspendCoroutine + direct render-frame driver."
    )

    suspend {
        while (true) {
            awaitNextFrame()
            frameCount++
            if (frameCount % 600L == 0L) {
                WitCanvas.Import.logMessage("leak-repro: tick #${frameCount}")
            }
        }
    }.startCoroutine(object : Continuation<Unit> {
        override val context = EmptyCoroutineContext
        override fun resumeWith(result: Result<Unit>) {
            WitCanvas.Import.logMessage(
                "leak-repro: coroutine ended unexpectedly: ${result.exceptionOrNull()}"
            )
        }
    })

    WitCanvas.Import.logMessage(
        "leak-repro: initial coroutine launched, awaiting first render-frame as wake"
    )
}
