// Hand-written Kotlin/Wasm binding for the `my:skiko-gfx/theme@0.1.0`
// WIT import. wit-bindgen has no Kotlin generator
// (see [[wit-bindgen-no-kotlin-generator]]); modeled on the existing
// FontsImports / AssetsImports lifts.
//
// Tiny surface: just the two getters. No records, no lists, no
// canonical-ABI gymnastics — get-night-mode returns the enum
// discriminant as i32, get-accent-color returns u32.

@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.ExperimentalWasmInterop::class,
)

package testapp.theme

import kotlin.wasm.*

enum class NightMode { AUTO, OFF, ON }

@WasmImport("my:skiko-gfx/theme@0.1.0", "get-night-mode")
private external fun __wasm_import_theme_get_night_mode(): Int

@WasmImport("my:skiko-gfx/theme@0.1.0", "get-accent-color")
private external fun __wasm_import_theme_get_accent_color(): Int

fun getNightMode(): NightMode = when (__wasm_import_theme_get_night_mode()) {
    0 -> NightMode.AUTO
    1 -> NightMode.OFF
    2 -> NightMode.ON
    else -> NightMode.AUTO
}

/// ARGB. 0 = unavailable (caller picks fallback).
fun getAccentColor(): UInt = __wasm_import_theme_get_accent_color().toUInt()
