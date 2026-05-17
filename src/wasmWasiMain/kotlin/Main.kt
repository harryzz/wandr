@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
)

// Task 24 step 1 — Minimal Kotlin/Wasm + kotlinx-coroutines repro.
// NO Compose runtime, NO Compose UI, NO SkiaLayer.
//
// The wart-host calls `renderer.render-frame(nanos)` per frame via
// RendererExports.kt → RendererImpl. RendererImpl routes to
// `currentSkiaLayer?.doFrame(...)` — we never set `currentSkiaLayer`,
// so render frames are null-safe no-ops on the draw side.
//
// We hook our own per-frame tick by replacing the default `appMain`
// + by having a dispatcher that gets ticked from RendererImpl's
// scheduler path. Concretely:
//
//   1. main() launches a coroutine doing `while(true) { tick() }`
//      where `tick()` is `suspendCoroutine { storeCont(it) }`.
//   2. We piggyback on WasiScheduler — schedule a callback for "1 ms
//      from now" in `tick()`, the scheduler fires it, resumes our
//      continuation, we go around again.
//
// If THIS leaks (same ~8 MB/min rate seen in task 23), the bug is
// in Kotlin/Wasm continuation codegen or in WasiScheduler /
// kotlinx-coroutines-wasmWasi. If NOT, the leak is above this layer
// — Compose Snapshot / Recomposer / etc.

package testapp

import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.resume
import kotlin.coroutines.startCoroutine
import kotlin.coroutines.suspendCoroutine
import org.jetbrains.skiko.wasi.WasiScheduler
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

// Single-slot park for the next-frame continuation. Mirrors the
// minimal kotlinx-coroutines pattern for `suspendCancellableCoroutine`
// but without any of kotlinx's machinery — just bare stdlib.
private var pendingNextFrame: Continuation<Unit>? = null

// Step 1b — reused-lambda variant: ONE instance of the resume
// callback, allocated once at startup. If the OOM seen in the first
// step-1 run was caused by per-tick lambda allocation, switching to a
// single shared lambda kills it. If the OOM persists, the leak is
// purely in Kotlin/Wasm's `suspend` state-machine codegen — nothing
// else left in this scope.
private val resumeLambda: () -> Unit = {
    val c = pendingNextFrame
    pendingNextFrame = null
    c?.resume(Unit)
}

private suspend fun awaitNextScheduledTick(): Unit = suspendCoroutine { cont ->
    pendingNextFrame = cont
    // Schedule a wake 1 ms from now; the wart-host's scheduler will fire
    // back via on-scheduled-callback → WasiScheduler.fire → resumeLambda.
    WasiScheduler.schedule(1u, resumeLambda)
}

private var frameCount: Long = 0L

fun main() {
    WitCanvas.Import.logMessage(
        "leak-repro: starting bare-coroutine loop — no Compose, no Snapshot, no Recomposer"
    )

    // Launch the suspend loop as a free coroutine (no parent Job, no
    // CoroutineScope). Each iteration parks the continuation in
    // `pendingNextFrame`, schedules a 1ms wake via WasiScheduler, and
    // resumes on the wake. If Kotlin/Wasm continuation codegen retains
    // the old continuation between iterations, the WasmGC heap will
    // grow without bound.
    suspend {
        while (true) {
            awaitNextScheduledTick()
            frameCount++
            if (frameCount % 600L == 0L) {
                // Log every ~600 ticks (~10 s at 1 ms tick rate, but
                // wart-host throttles to vsync so this is more like
                // every 10 s of wall clock).
                WitCanvas.Import.logMessage(
                    "leak-repro: tick #${frameCount}"
                )
            }
        }
    }.startCoroutine(object : Continuation<Unit> {
        override val context = EmptyCoroutineContext
        override fun resumeWith(result: Result<Unit>) {
            // The loop is infinite; this is only reached if something
            // throws or main scope dies. Log so we notice.
            WitCanvas.Import.logMessage(
                "leak-repro: coroutine ended unexpectedly: ${result.exceptionOrNull()}"
            )
        }
    })

    WitCanvas.Import.logMessage(
        "leak-repro: initial coroutine launched, scheduler primed at tick=0"
    )
}
