@file:OptIn(ExperimentalWasmDsl::class)

import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

// Guest half of the export-record spike (see ../wit/spike.wit). Built like a
// wandr app: Kotlin 2.4.0-RC compiler + the wandr 2.4.258-SNAPSHOT stdlib
// override (KT-86415 Tier-2 fix: persistent realloc allocator + watermark
// freeAll, fixed linear-memory partition) — the exact production pairing the
// spike is meant to validate.

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
