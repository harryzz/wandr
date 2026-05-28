// war:ime/ime exports — task 49 step 1b. Hand-written @WasmExport wrappers
// for the two `on-editor-attached` / `on-editor-detached` functions the
// host calls when an editor focuses or defocuses. Parallels the
// auto-generated `RendererExports.kt` next to this file.
//
// The host (wart-host) sends `editor-attached <input-type> <hint>
// <initial-text> <sel-start> <sel-end>` over `/data/local/tmp/
// wart-host-<ime-pid>.sock` (task 49 step 1a), the inbound drain
// (ime_inbound.rs) parses + queues, the render loop dispatches via
// the second `ime_bindings::ImeEvents` bindgen wrapper in lib.rs.
//
// On the wasm side the canonical-ABI lowering of `on-editor-attached(
// info: editor-info)` flattens the record's fields into 7 i32 params:
//
//   p0  input-type enum tag (0..6)
//   p1  hint ptr in linear memory
//   p2  hint length (utf-8 bytes)
//   p3  initial-text ptr
//   p4  initial-text length
//   p5  selection-start (u32)
//   p6  selection-end   (u32)
//
// Strings are utf-8 encoded; the lift uses `loadString(Pointer, Int)`
// from skiko's ComponentSupport.kt.

@file:OptIn(UnsafeWasmMemoryApi::class, ExperimentalWasmInterop::class, ComponentModelInternalApi::class)

package org.jetbrains.skiko.wasi.wit

import kotlin.wasm.unsafe.*
import testapp.ImeEventsImpl

/// Mirror of `war:ime/types.input-type`. Kotlin-side enum we hand into
/// the app layer. The wire's bare tag (0..6) maps to these by index;
/// any unknown tag falls back to TEXT (defensive).
enum class ImeInputType {
    TEXT,
    NUMBER,
    PHONE,
    EMAIL,
    URL,
    PASSWORD,
    MULTILINE_TEXT,
}

// Step 1b bisect — minimal exports (just store the bare inputType
// tag). Once we confirm the cross-process plumbing reaches the guest,
// we can grow the body. The current minimal form avoids:
//   - allocator scope (which seemed to throw on first call before
//     the runtime is fully booted)
//   - string lift from linear memory
//   - host import calls (logMessage)
// Just sets a single Int field on a Kotlin object — Kotlin object
// init is lazy and runtime-safe.
@WasmExport("war:ime/ime@0.1.0#on-editor-attached")
fun __wasm_export_onEditorAttached(p0: Int): Unit {
    // p0 is the input-type enum tag (0..6). See ImeInputType. No
    // strings / records on the wire — see comment in wit/ime.wit
    // about avoiding cabi_realloc lowering.
    ImeEventsImpl.recordInputTypeTag(p0)
}

@WasmExport("war:ime/ime@0.1.0#on-editor-detached")
fun __wasm_export_onEditorDetached(): Unit {
    ImeEventsImpl.recordDetached()
}
