// WIT renderer exports and component-model realloc hook.
// These MUST live in the final executable (not a library KLIB) so they survive DCE.

@file:OptIn(UnsafeWasmMemoryApi::class, ExperimentalWasmInterop::class, ComponentModelInternalApi::class)

package org.jetbrains.skiko.wasi.wit

import org.jetbrains.skiko.wasi.RendererImpl
import kotlin.wasm.unsafe.*
import testapp.main as appMain

@WasmExport
fun cabi_realloc(ptr: Int, oldSize: Int, align: Int, newSize: Int): Int =
    componentModelRealloc(ptr, oldSize, newSize)

private var booted = false

@WasmExport("my:skiko-gfx/renderer@0.1.0#render-frame")
fun __wasm_export_renderFrame(p0: Long): Unit {
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
        RendererImpl.renderFrame(p0.toULong())
    }
}

@WasmExport("my:skiko-gfx/renderer@0.1.0#on-pointer-event")
fun __wasm_export_onPointerEvent(p0: Int, p1: Float, p2: Float): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        RendererImpl.onPointerEvent(Renderer.PointerKind.values()[p0], p1, p2)
    }
}

@WasmExport("my:skiko-gfx/renderer@0.1.0#on-key-event")
fun __wasm_export_onKeyEvent(p0: Int, p1: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        RendererImpl.onKeyEvent(Renderer.KeyKind.values()[p0], p1.toUInt())
    }
}

@WasmExport("my:skiko-gfx/renderer@0.1.0#on-resize")
fun __wasm_export_onResize(p0: Int, p1: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        RendererImpl.onResize(p0.toUInt(), p1.toUInt())
    }
}

@WasmExport("my:skiko-gfx/renderer@0.1.0#on-scheduled-callback")
fun __wasm_export_onScheduledCallback(p0: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        RendererImpl.onScheduledCallback(p0.toUInt())
    }
}

@WasmExport("my:skiko-gfx/renderer@0.1.0#on-pointer-event-v2")
fun __wasm_export_onPointerEventV2(p0: Int, p1: Int, p2: Float, p3: Float, p4: Float): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        RendererImpl.onPointerEventV2(
            p0.toUInt(),
            Renderer.PointerKind.values()[p1],
            p2, p3, p4,
        )
    }
}

@WasmExport("my:skiko-gfx/renderer@0.1.0#on-key-event-v2")
fun __wasm_export_onKeyEventV2(p0: Int, p1: Int, p2: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        RendererImpl.onKeyEventV2(
            Renderer.KeyKind.values()[p0],
            p1.toUInt(),  // code-point
            p2.toUInt(),  // key-id
        )
    }
}

@WasmExport("my:skiko-gfx/renderer@0.1.0#on-lifecycle-changed")
fun __wasm_export_onLifecycleChanged(p0: Int): Unit {
    freeAllComponentModelReallocAllocatedMemory()
    withScopedMemoryAllocator { _ ->
        RendererImpl.onLifecycleChanged(p0.toUInt())
    }
}
