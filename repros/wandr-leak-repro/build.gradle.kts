@file:OptIn(ExperimentalWasmDsl::class)

import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

// Self-driving minimal reproducer for the wasmtime DRC sweep-cost
// issue (bytecodealliance/wasmtime#13403).
//
// No dependencies at all — Main.kt uses ONLY the Kotlin stdlib's
// `suspendCoroutine` + `startCoroutine` from `kotlin.coroutines`. No
// skiko, no Compose, no kotlinx-coroutines, no component model, no
// host imports. A plain `wasmtime run` of the output reproduces the
// unbounded WasmGC-garbage accumulation directly.

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
}
