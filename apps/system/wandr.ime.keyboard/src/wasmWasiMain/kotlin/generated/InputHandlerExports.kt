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
// NOT exported. Lowering the key-event record's strings into a live Compose
// guest throws an exception that Kotlin catch(Throwable) cannot intercept
// (escapes even from inside cabi_realloc's catch), poisoning the instance.
// The clean-room spike (repros/kt-export-record-spike) passes 100k/100k, so
// the trigger is Compose-app allocator/runtime state — under investigation.
// Until resolved, keys ride the legacy primitive on-key-event-v2 path (the
// host falls back automatically when key-handler is unbound).
