@file:OptIn(ExperimentalWasmDsl::class)

import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

// Minimal standalone Kotlin/Wasm repro for the
// `ScopedMemoryAllocator.destroy()` doesn't-bump-parent-availableAddress
// bug. Demonstrates that after componentModelRealloc allocates a
// "long-lived" block and freeAllComponentModelReallocAllocatedMemory()
// destroys the realloc allocator's scope, a subsequent
// withScopedMemoryAllocator gives back overlapping memory.
//
// See feedback_kotlin_wasm_scopedmemory_destroy_bug.md.

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

// Toggle: comment/uncomment to switch between stock 2.4.0-RC stdlib
// (shows the bug) and our patched 2.4.255-SNAPSHOT (shows the fix).
val useKt86415Patch = false
if (useKt86415Patch) {
    configurations.all {
        resolutionStrategy.eachDependency {
            if (requested.group == "org.jetbrains.kotlin" &&
                requested.name == "kotlin-stdlib-wasm-wasi") {
                useVersion("2.4.255-SNAPSHOT")
                because("KT-86415 patched build from ~/xl/kotlin")
            }
        }
    }
}
