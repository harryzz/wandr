// Self-driving minimal reproducer for the wasmtime DRC sweep-cost
// issue (bytecodealliance/wasmtime#13403).
//
// Standalone: no host imports, no component model, no skiko, no
// Compose, no kotlinx-coroutines. `main()` itself drives a bare
// `suspendCoroutine` suspend/resume cycle in a tight unbounded loop,
// so a plain
//
//   wasmtime run -Wgc,function-references,exceptions wart-leak-repro.wasm
//
// reproduces the unbounded WasmGC-garbage accumulation directly — no
// embedder, no external driver, no exported function to call.
//
// Each iteration: the coroutine suspends at `suspendCoroutine`
// (allocating a continuation / state-machine instance), then `tick()`
// resumes it. Once resumed, that instance is unreachable — collectable
// garbage. With wasmtime's DRC collector nothing sweeps until a GC-heap
// grow fails, so the garbage piles up and each successive sweep walks
// an ever-larger over-approximated-roots list.

package testapp

import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.resume
import kotlin.coroutines.startCoroutine
import kotlin.coroutines.suspendCoroutine

private var pendingNextFrame: Continuation<Unit>? = null

private fun tick() {
    val c = pendingNextFrame
    pendingNextFrame = null
    c?.resume(Unit)
}

private suspend fun awaitNextFrame(): Unit = suspendCoroutine { cont ->
    pendingNextFrame = cont
}

private var frameCount: Long = 0L

fun main() {
    println("leak-repro: self-driving repro starting — pure suspendCoroutine, no host imports")

    suspend {
        while (true) {
            awaitNextFrame()
            frameCount++
        }
    }.startCoroutine(object : Continuation<Unit> {
        override val context = EmptyCoroutineContext
        override fun resumeWith(result: Result<Unit>) {
            println("leak-repro: coroutine ended unexpectedly: ${result.exceptionOrNull()}")
        }
    })

    // Self-drive: each tick advances the suspend loop one iteration.
    // Unbounded so the WasmGC heap fills and DRC sweeps repeatedly —
    // let it run a few minutes and watch sweep cost climb, then Ctrl-C.
    while (true) {
        tick()
        if (frameCount % 1_000_000L == 0L) {
            println("leak-repro: tick #${frameCount}")
        }
    }
}
