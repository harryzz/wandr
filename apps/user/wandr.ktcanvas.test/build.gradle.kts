@file:OptIn(ExperimentalWasmDsl::class)

import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

// Stage 1 of the Kotlin wasi:canvas migration (see wit/ktcanvas-test.wit).
// Built like every wandr Kotlin guest: Kotlin 2.4.0-RC compiler + the wandr
// 2.4.258-SNAPSHOT stdlib override (KT-86415 Tier-2 fix: fixed linear-memory
// partition + export-exit-pump guard) — must match the forked P1 adapter.

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

configurations.all {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.jetbrains.kotlin" &&
            requested.name == "kotlin-stdlib-wasm-wasi") {
            useVersion("2.4.258-SNAPSHOT")
            because("wandr stdlib (KT-86415 Tier-2 fix) — must match the forked P1 adapter")
        }
    }
}
