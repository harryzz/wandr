// wasi:input-handlers exports — the push-model input contract the host
// routes to EXCLUSIVELY once bound (legacy my:skiko-gfx/renderer verbs are
// then only used for lifecycle/scheduled-callback). Hand-written like
// RendererExports.kt; must live in the final executable (not a library
// KLIB) so the @WasmExport survives DCE.
//
// ‼️ Memory contract for the key handler (records-with-strings arrive
// host→guest, the host cabi_realloc's them into OUR linear memory before
// the body runs): freeAll FIRST, then lift ALL strings (pure reads), and
// only then open a scoped allocator. Proven by
// repros/kt-export-record-spike (100k/100k desktop JIT + device AOT; the
// interleaved order corrupts 100%).

@file:OptIn(UnsafeWasmMemoryApi::class, ExperimentalWasmInterop::class, ComponentModelInternalApi::class)

package org.jetbrains.skiko.wasi.wit

import org.jetbrains.skiko.wasi.RendererImpl
import kotlin.wasm.unsafe.*

// ── frame-handler ─────────────────────────────────────────────────────────
// Pure delegation: the legacy renderer exports already carry the freeAll +
// first-frame boot + scoped-allocator discipline, and they're plain Kotlin
// functions — calling them keeps ONE implementation of that logic.

@WasmExport("wasi:input-handlers/frame-handler@0.0.1#on-frame")
fun __wasm_export_ih_onFrame(p0: Long): Unit {
    __wasm_export_renderFrame(p0)
}

@WasmExport("wasi:input-handlers/frame-handler@0.0.1#on-resize")
fun __wasm_export_ih_onResize(p0: Int, p1: Int): Unit {
    __wasm_export_onResize(p0, p1)
}

// ── pointer-handler ───────────────────────────────────────────────────────
// record pointer-event { id: u32, kind: enum{down,up,move,scroll,cancel},
//   x,y,pressure,scroll-dx,scroll-dy: f32, alt,ctrl,meta,shift: bool }
// → 11 flat params, no strings (no lift hazard).

@WasmExport("wasi:input-handlers/pointer-handler@0.0.1#on-pointer")
fun __wasm_export_ih_onPointer(
    id: Int, kind: Int, x: Float, y: Float, pressure: Float,
    scrollDx: Float, scrollDy: Float,
    alt: Int, ctrl: Int, meta: Int, shift: Int,
): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    try {
        withScopedMemoryAllocator { _ ->
            // kind indexes 0..3 match the legacy enum; 4 (cancel) has no legacy
            // counterpart — treat as up so gestures terminate cleanly.
            val k = if (kind >= 4) Renderer.PointerKind.values()[1]
                    else Renderer.PointerKind.values()[kind]
            // BOTH legacy entries, mirroring the legacy host's v1+v2 fanout:
            // v1 (onPointerEvent) is the path that actually feeds the Compose
            // scene (currentSkiaLayer); v2 goes to the OPT-IN WasiInput
            // handler, which is silently discarded when nothing registers.
            RendererImpl.onPointerEvent(k, x, y)
            RendererImpl.onPointerEventV2(id.toUInt(), k, x, y, pressure)
        }
    } catch (t: Throwable) {
        Canvas.Import.logMessage("ih-onPointer FAILED: ${t::class.simpleName}: ${t.message}")
    }
}

// ── key-handler ───────────────────────────────────────────────────────────
// record key-event { down: bool, repeat: bool, code: string, text: string,
//   alt,ctrl,meta,shift: bool } → 10 flat params, TWO host-lowered strings.
// ‼️ Requires the wandr stdlib fix (2.4.258-SNAPSHOT, internalCallback.kt):
// cabi_realloc is itself an @WasmExport, so its compiler-generated epilogue
// fired invokeOnExportedFunctionExit (kotlinx-coroutines' pump) WHILE the
// host-lowered args were pending → "Can't create new allocators…" escaped
// uncatchably and poisoned the instance. The stdlib now skips the pump
// while realloc memory is pending.

private fun w3cCodeToKeyId(code: String): UInt = when (code) {
    "Backspace" -> 8u
    "Tab" -> 9u
    "Enter", "NumpadEnter" -> 13u
    "Escape" -> 27u
    "Space" -> 32u
    "PageUp" -> 33u
    "PageDown" -> 34u
    "End" -> 35u
    "Home" -> 36u
    "ArrowLeft" -> 37u
    "ArrowUp" -> 38u
    "ArrowRight" -> 39u
    "ArrowDown" -> 40u
    "Insert" -> 45u
    "Delete" -> 46u
    else -> 0u
}

private fun firstCodePoint(s: String): UInt {
    if (s.isEmpty()) return 0u
    val c0 = s[0]
    return if (c0.isHighSurrogate() && s.length > 1 && s[1].isLowSurrogate()) {
        ((((c0.code - 0xD800) shl 10) or (s[1].code - 0xDC00)) + 0x10000).toUInt()
    } else {
        c0.code.toUInt()
    }
}

private fun __ih_loadString(addr: Int, len: Int): String {
    val base = Pointer(addr.toUInt())
    return ByteArray(len) { i -> (base + i).loadByte() }.decodeToString()
}

@WasmExport("wasi:input-handlers/key-handler@0.0.1#on-key")
fun __wasm_export_ih_onKey(
    down: Int, repeat: Int,
    codePtr: Int, codeLen: Int,
    textPtr: Int, textLen: Int,
    alt: Int, ctrl: Int, meta: Int, shift: Int,
): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    try {
        // Lift BOTH strings before any allocator scope opens (the spike contract).
        val code = __ih_loadString(codePtr, codeLen)
        val text = __ih_loadString(textPtr, textLen)
        withScopedMemoryAllocator { _ ->
            val kind = if (down != 0) Renderer.KeyKind.values()[0] else Renderer.KeyKind.values()[1]
            RendererImpl.onKeyEventV2(kind, firstCodePoint(text), w3cCodeToKeyId(code))
        }
    } catch (t: Throwable) {
        try {
            Canvas.Import.logMessage("ih-onKey FAILED: ${t.message}")
        } catch (_: Throwable) {}
    }
}
