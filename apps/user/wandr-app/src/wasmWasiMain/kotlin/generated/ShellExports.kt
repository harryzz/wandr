// Phase B (task 105) — the app-side export glue: wasi:input-handlers@0.0.2
// (frames + pointer + key) and wandr:ui-shell (shell-events + frame-pacing),
// replacing the retired my:skiko-gfx/renderer exports. Hand-placed in the
// FINAL EXECUTABLE (not a library KLIB) so the @WasmExport survive DCE —
// same constraint as the old RendererExports.kt; lowering bodies mirror the
// Kotlin wit-bindgen fork's generated stubs (skiko generated/uishell).
//
// ‼️ Memory contract (repros/kt-export-record-spike, 100k/100k JIT+AOT):
// for exports whose args arrive via host cabi_realloc (strings, spilled
// records): freeAll FIRST, then lift ALL args (pure reads), and only then
// open a scoped allocator. The interleaved order corrupts 100%.

@file:OptIn(UnsafeWasmMemoryApi::class, ExperimentalWasmInterop::class, ComponentModelInternalApi::class)

package testapp.exports

import kotlin.wasm.unsafe.*
import org.jetbrains.skiko.wasi.shell.KeyHandlerImpl
import org.jetbrains.skiko.wasi.shell.FrameHandlerImpl
import org.jetbrains.skiko.wasi.shell.PointerHandlerImpl
import org.jetbrains.skiko.wasi.shell.ShellEventsImpl
import org.jetbrains.skiko.wasi.shell.KeyHandler
import org.jetbrains.skiko.wasi.shell.Lifecycle
import org.jetbrains.skiko.wasi.shell.PointerHandler
import testapp.main as appMain

@WasmExport
fun cabi_realloc(ptr: Int, oldSize: Int, align: Int, newSize: Int): Int =
    componentModelRealloc(ptr, oldSize, newSize)

private var booted = false

// ── frame-handler (the render driver — boots appMain on first frame) ───────

@WasmExport("wasi:input-handlers/frame-handler@0.0.2#on-frame")
fun __wasm_export_onFrame(p0: Long): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    if (!booted) {
        booted = true
        appMain()
        // appMain runs initial composition which can pull in Random.Default →
        // wasiRandomGet → cabi_realloc → reallocAllocator polluted. Clean up
        // again before the per-frame allocator scope.
        freeAllComponentModelReallocAllocatedMemory()
    }
    withScopedMemoryAllocator { _ ->
        FrameHandlerImpl.onFrame(p0.toULong())
    }
}

@WasmExport("wasi:input-handlers/frame-handler@0.0.2#on-resize")
fun __wasm_export_onResize(p0: Int, p1: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        FrameHandlerImpl.onResize(p0.toUInt(), p1.toUInt())
    }
}

// ── pointer-handler ─────────────────────────────────────────────────────────
// The 0.0.2 union record (17 fields) spills past the 16-flat-arg limit, so
// it arrives as ONE linear-memory pointer. All loads are pure reads — done
// BEFORE the allocator scope opens, per the memory contract.

private fun lb(addr: Int): Int = Pointer(addr.toUInt()).loadByte().toInt() and 0xFF
private fun li(addr: Int): Int = Pointer(addr.toUInt()).loadInt()
private fun lf(addr: Int): Float = Float.fromBits(Pointer(addr.toUInt()).loadInt())

@WasmExport("wasi:input-handlers/pointer-handler@0.0.2#on-pointer")
fun __wasm_export_onPointer(p0: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    val ev = PointerHandler.PointerEvent(
        li(p0 + 0).toUInt(),
        PointerHandler.Kind.values()[lb(p0 + 4)],
        PointerHandler.PointerDevice.values()[lb(p0 + 5)],
        lf(p0 + 8),
        lf(p0 + 12),
        lf(p0 + 16),
        lf(p0 + 20),
        lf(p0 + 24),
        lf(p0 + 28),
        lf(p0 + 32),
        lf(p0 + 36),
        PointerHandler.Button.values()[lb(p0 + 40)],
        org.jetbrains.skiko.wasi.shell.pointerButtonsOf(lb(p0 + 41).toLong()),
        lb(p0 + 42) != 0,
        lb(p0 + 43) != 0,
        lb(p0 + 44) != 0,
        lb(p0 + 45) != 0,
    )
    try {
        withScopedMemoryAllocator { _ ->
            PointerHandlerImpl.onPointer(ev)
        }
    } catch (t: Throwable) {
        try { testapp.logMessage("ih-onPointer FAILED: ${t::class.simpleName}: ${t.message}") } catch (_: Throwable) {}
    }
}

// ── key-handler ─────────────────────────────────────────────────────────────
// record key-event { down, repeat: bool, code, text: string, 4×bool } →
// 10 flat params, TWO host-lowered strings. ‼️ Requires the wandr stdlib
// fix (2.4.258-SNAPSHOT, internalCallback.kt): the pump is skipped while
// canonical-ABI realloc memory is pending.

private fun loadString(addr: Int, len: Int): String {
    val base = Pointer(addr.toUInt())
    return ByteArray(len) { i -> (base + i).loadByte() }.decodeToString()
}

@WasmExport("wasi:input-handlers/key-handler@0.0.2#on-key")
fun __wasm_export_onKey(
    down: Int, repeat: Int,
    codePtr: Int, codeLen: Int,
    textPtr: Int, textLen: Int,
    alt: Int, ctrl: Int, meta: Int, shift: Int,
): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    try {
        // Lift BOTH strings before any allocator scope opens (the spike contract).
        val code = loadString(codePtr, codeLen)
        val text = loadString(textPtr, textLen)
        withScopedMemoryAllocator { _ ->
            KeyHandlerImpl.onKey(KeyHandler.KeyEvent(
                down != 0, repeat != 0, code, text,
                alt != 0, ctrl != 0, meta != 0, shift != 0,
            ))
        }
    } catch (t: Throwable) {
        try { testapp.logMessage("ih-onKey FAILED: ${t.message}") } catch (_: Throwable) {}
    }
}

// ── shell-events (lifecycle + scheduler callbacks) ──────────────────────────

@WasmExport("wandr:ui-shell/shell-events@0.1.0#on-scheduled-callback")
fun __wasm_export_onScheduledCallback(p0: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        ShellEventsImpl.onScheduledCallback(p0.toUInt())
    }
}

@WasmExport("wandr:ui-shell/shell-events@0.1.0#on-lifecycle-changed")
fun __wasm_export_onLifecycleChanged(p0: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        ShellEventsImpl.onLifecycleChanged(Lifecycle.State.values()[p0])
    }
}

// ── frame-pacing (task 64 on-demand rendering) ──────────────────────────────
// No params/returns through linear memory, so no scoped allocator — just
// the standard freeAll.

@WasmExport("wandr:ui-shell/frame-pacing@0.1.0#next-frame-delay")
fun __wasm_export_nextFrameDelay(): Int {
    freeAllComponentModelReallocAllocatedMemory()
    return testapp.nextFrameDelayMillis()
}
