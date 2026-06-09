// Hand-written Kotlin/Wasm binding for the `my:skiko-gfx/launcher@0.1.0`
// WIT import (task 57 launcher). wit-bindgen has no Kotlin generator
// (see [[wit-bindgen-no-kotlin-generator]]); modeled on AssetsImports.kt
// (same `my:skiko-gfx` package — string param + caller-allocated return
// area). Flat-string wire format keeps the canonical ABI simple: no
// list<record> lift.

@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
    kotlin.wasm.ExperimentalWasmInterop::class,
)

package testapp.launcher

import kotlin.wasm.*
import kotlin.wasm.unsafe.*

/// `list-apps() -> string`. Importer-side canonical ABI: caller-allocated
/// 8-byte return area holding the returned string's (ptr, len).
@WasmImport("my:skiko-gfx/launcher@0.1.0", "list-apps")
private external fun __wasm_import_launcher_list_apps(returnAreaPtr: Int)

/// `launch-app(app-id: string)`. Importer-side: (ptr, len), no return.
@WasmImport("my:skiko-gfx/launcher@0.1.0", "launch-app")
private external fun __wasm_import_launcher_launch_app(idPtr: Int, idLen: Int)

/// One installed app: its id (for launch) and display label.
data class AppEntry(val appId: String, val label: String)

/// Enumerate installed user-launchable apps. The host returns
/// newline-delimited `app-id\tlabel` lines; we parse them. Each call
/// re-scans the host install dir, so call it once (e.g. in a
/// `LaunchedEffect`), not per frame.
fun listApps(): List<AppEntry> = withScopedMemoryAllocator { alloc ->
    val retArea = alloc.allocate(8).address.toInt()
    __wasm_import_launcher_list_apps(retArea)
    val ptr = Pointer(retArea.toUInt()).loadInt()
    val len = Pointer((retArea + 4).toUInt()).loadInt()
    if (len <= 0) {
        emptyList()
    } else {
        val bytes = ByteArray(len)
        for (i in 0 until len) bytes[i] = Pointer((ptr + i).toUInt()).loadByte()
        bytes.decodeToString()
            .split('\n')
            .filter { it.isNotBlank() }
            .map { line ->
                val tab = line.indexOf('\t')
                if (tab < 0) AppEntry(line, line)
                else AppEntry(line.substring(0, tab), line.substring(tab + 1))
            }
    }
}

/// Ask the arbiter (via the host) to launch / foreground an app.
fun launchApp(appId: String): Unit = withScopedMemoryAllocator { alloc ->
    val bytes = appId.encodeToByteArray()
    val ptr = alloc.allocate(bytes.size)
    var cur = ptr
    bytes.forEach { cur.storeByte(it); cur += 1 }
    __wasm_import_launcher_launch_app(ptr.address.toInt(), bytes.size)
}
