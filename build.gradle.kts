@file:OptIn(ExperimentalWasmDsl::class)

import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

// Task 24 step 1 — minimal Kotlin/Wasm + kotlinx-coroutines repro to
// see if the WasmGC-heap leak measured in task 23 reproduces below
// Compose. No Compose runtime, no Compose UI. Just skiko-wasm-wasi
// (for the WIT bindings + RendererImpl that the wart-host calls) +
// kotlinx-coroutines-core for the suspend loop.
//
// If THIS leaks, the bug is in Kotlin/Wasm continuation codegen
// and/or kotlinx-coroutines-wasmWasi — file upstream.
// If THIS does NOT leak, narrow further by adding Compose runtime
// (task 24 step 2).

plugins {
    kotlin("multiplatform") version "2.4.0-RC"
}

repositories {
    mavenLocal()
    mavenCentral()
    maven("https://maven.pkg.jetbrains.space/kotlin/p/kotlin/dev")
}

kotlin {
    wasmWasi {
        binaries.executable()
    }

    sourceSets {
        wasmWasiMain.dependencies {
            implementation("org.jetbrains.skiko:skiko-wasm-wasi:0.0.0-SNAPSHOT")
            // No kotlinx-coroutines dep in step 1. Main.kt uses ONLY
            // stdlib's `suspendCoroutine` + `startCoroutine` from
            // kotlin.coroutines. If THIS leaks, it's purely
            // Kotlin/Wasm continuation codegen. Step 2 will add the
            // kotlinx dep + `withFrameNanos` + BroadcastFrameClock
            // if step 1 comes back clean.
        }
    }
}
