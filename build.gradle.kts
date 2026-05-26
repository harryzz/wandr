@file:OptIn(ExperimentalWasmDsl::class)

import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

// Tiny Kotlin/Wasm consumer for task 36 step 6 — validates the
// cross-app dep resolution + same-Store composition wired in wart-host's
// app_loader. NO Compose, NO skiko — just a `main()` that calls the
// imported `war:markdown/renderer.render` and prints the result via
// WASI stderr.
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
