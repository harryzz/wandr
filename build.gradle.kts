@file:OptIn(ExperimentalWasmDsl::class)

import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

plugins {
    kotlin("multiplatform") version "2.4.0-RC"
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.0-RC"
}

repositories {
    mavenLocal()
    mavenCentral()
    maven("https://maven.pkg.jetbrains.space/kotlin/p/kotlin/dev")
}

// Apply only to wasmWasi configurations; `configurations.all` eagerly realizes
// every KMP configuration and balloons configure time from seconds to minutes.
configurations.matching { it.name.startsWith("wasmWasi") }.configureEach {
    // Real-source compose modules publish only target-specific *-wasm-wasi
    // klibs (no umbrella). Their .module files reference an umbrella URL that
    // doesn't exist; without these substitutes Gradle fails resolution.
    // Same trick for the legacy androidx.* coords that pin maven versions
    // with no wasi variant.
    resolutionStrategy.dependencySubstitution {
        substitute(module("org.jetbrains.compose.ui:ui-util")).using(
            module("org.jetbrains.compose.ui:ui-util-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.ui:ui-geometry")).using(
            module("org.jetbrains.compose.ui:ui-geometry-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.ui:ui-unit")).using(
            module("org.jetbrains.compose.ui:ui-unit-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.ui:ui-graphics")).using(
            module("org.jetbrains.compose.ui:ui-graphics-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.ui:ui-text")).using(
            module("org.jetbrains.compose.ui:ui-text-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.ui:ui-backhandler")).using(
            module("org.jetbrains.compose.ui:ui-backhandler-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.ui:ui")).using(
            module("org.jetbrains.compose.ui:ui-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.foundation:foundation-layout")).using(
            module("org.jetbrains.compose.foundation:foundation-layout-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.foundation:foundation")).using(
            module("org.jetbrains.compose.foundation:foundation-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.animation:animation-core")).using(
            module("org.jetbrains.compose.animation:animation-core-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.animation:animation")).using(
            module("org.jetbrains.compose.animation:animation-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.material:material-ripple")).using(
            module("org.jetbrains.compose.material:material-ripple-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.material3:material3")).using(
            module("org.jetbrains.compose.material3:material3-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.compose.runtime:runtime-saveable")).using(
            module("org.jetbrains.compose.runtime:runtime-saveable-wasm-wasi:9999.0.0-SNAPSHOT"))
        // graphics-shapes published 1.1.0-rc01 but upstream commonMain pins 1.1.0.
        substitute(module("androidx.graphics:graphics-shapes")).using(
            module("androidx.graphics:graphics-shapes-wasm-wasi:1.1.0-rc01"))
        // skiko maven 0.148.0 has no wasi variant; redirect to our shim.
        substitute(module("org.jetbrains.skiko:skiko")).using(
            module("org.jetbrains.skiko:skiko-wasm-wasi:0.0.0-SNAPSHOT"))
        // JetBrains-fork stub umbrellas that exist on m2 — point them at
        // 9999.0.0-SNAPSHOT (the version we published) when transitive deps
        // ask for older pins (1.3.6 / 2.9.6 / 2.11.0-beta01 etc.).
        substitute(module("org.jetbrains.androidx.savedstate:savedstate-compose")).using(
            module("org.jetbrains.androidx.savedstate:savedstate-compose-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.savedstate:savedstate")).using(
            module("org.jetbrains.androidx.savedstate:savedstate:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.lifecycle:lifecycle-common")).using(
            module("org.jetbrains.androidx.lifecycle:lifecycle-common:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.lifecycle:lifecycle-runtime")).using(
            module("org.jetbrains.androidx.lifecycle:lifecycle-runtime:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.lifecycle:lifecycle-runtime-compose")).using(
            module("org.jetbrains.androidx.lifecycle:lifecycle-runtime-compose-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.lifecycle:lifecycle-viewmodel")).using(
            module("org.jetbrains.androidx.lifecycle:lifecycle-viewmodel:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.lifecycle:lifecycle-viewmodel-savedstate")).using(
            module("org.jetbrains.androidx.lifecycle:lifecycle-viewmodel-savedstate:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.lifecycle:lifecycle-viewmodel-compose")).using(
            module("org.jetbrains.androidx.lifecycle:lifecycle-viewmodel-compose-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.navigation:navigation-common")).using(
            module("org.jetbrains.androidx.navigation:navigation-common:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.navigation:navigation-runtime")).using(
            module("org.jetbrains.androidx.navigation:navigation-runtime:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.navigationevent:navigationevent-compose")).using(
            module("org.jetbrains.androidx.navigationevent:navigationevent-compose-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("org.jetbrains.androidx.window:window-core")).using(
            module("org.jetbrains.androidx.window:window-core:9999.0.0-SNAPSHOT"))
        substitute(module("androidx.compose.runtime:runtime-retain")).using(
            module("androidx.compose.runtime:runtime-retain-wasm-wasi:1.12.0-alpha02"))
        // Legacy maven coords picked up as transitives from older fork pins.
        substitute(module("androidx.navigationevent:navigationevent-compose")).using(
            module("org.jetbrains.androidx.navigationevent:navigationevent-compose-wasm-wasi:9999.0.0-SNAPSHOT"))
        substitute(module("androidx.lifecycle:lifecycle-viewmodel-compose")).using(
            module("org.jetbrains.androidx.lifecycle:lifecycle-viewmodel-compose-wasm-wasi:9999.0.0-SNAPSHOT"))
    }
    exclude(group = "androidx.collection", module = "collection")
    exclude(group = "androidx.collection", module = "collection-ktx")
    exclude(group = "androidx.annotation", module = "annotation")
    exclude(group = "androidx.compose.ui", module = "ui")
    exclude(group = "androidx.compose.ui", module = "ui-graphics")
    exclude(group = "androidx.compose.ui", module = "ui-text")
    exclude(group = "androidx.compose.ui", module = "ui-unit")
    exclude(group = "androidx.compose.ui", module = "ui-util")
    exclude(group = "androidx.compose.runtime", module = "runtime")
    exclude(group = "androidx.compose.runtime", module = "runtime-saveable")
    exclude(group = "androidx.compose.animation", module = "animation")
    exclude(group = "androidx.compose.animation", module = "animation-core")
    exclude(group = "androidx.compose.foundation", module = "foundation")
    exclude(group = "androidx.compose.foundation", module = "foundation-layout")
    exclude(group = "androidx.compose.material", module = "material-ripple")
    exclude(group = "androidx.compose.material3", module = "material3")
    exclude(group = "androidx.lifecycle", module = "lifecycle-runtime-compose")
    exclude(group = "androidx.lifecycle", module = "lifecycle-runtime")
    exclude(group = "androidx.lifecycle", module = "lifecycle-common")
    exclude(group = "androidx.lifecycle", module = "lifecycle-viewmodel")
    exclude(group = "androidx.lifecycle", module = "lifecycle-viewmodel-savedstate")
    exclude(group = "androidx.lifecycle", module = "lifecycle-viewmodel-compose")
    exclude(group = "androidx.savedstate", module = "savedstate")
    exclude(group = "androidx.savedstate", module = "savedstate-compose")
    exclude(group = "androidx.navigationevent", module = "navigationevent-compose")
    exclude(group = "androidx.window", module = "window")
    exclude(group = "androidx.window", module = "window-core")
    exclude(group = "androidx.activity", module = "activity")
    exclude(group = "androidx.activity", module = "activity-compose")
    exclude(group = "androidx.activity", module = "activity-ktx")
    exclude(group = "androidx.fragment", module = "fragment")
    exclude(group = "androidx.core", module = "core")
    exclude(group = "androidx.appcompat", module = "appcompat")
    exclude(group = "androidx.autofill", module = "autofill")
    exclude(group = "androidx.customview", module = "customview-poolingcontainer")
    exclude(group = "androidx.emoji2", module = "emoji2")
    exclude(group = "androidx.profileinstaller", module = "profileinstaller")
    exclude(group = "androidx.transition", module = "transition")
}

kotlin {
    wasmWasi {
        binaries.executable()
    }

    sourceSets {
        wasmWasiMain.dependencies {
            // Sibling-published "fat" wasi klibs at /home/harry/wart/compose-*-wasi/.
            // Each bundles multiple upstream modules' srcDirs from
            // /home/harry/wart/compose-multiplatform-core/ — i.e. in-tree
            // sources, but baked into 11 klibs instead of 32, so WASM
            // whole-world lowering is ~24× faster (~5 min vs ~2 h).
            implementation("org.jetbrains.skiko:skiko-wasm-wasi:0.0.0-SNAPSHOT")
            implementation("androidx.compose.runtime:compose-runtime-wasi:0.0.0-wasi-local")
            implementation("androidx.compose.ui:compose-ui-base-wasi:0.0.0-wasi-local")
            implementation("androidx.compose.ui:compose-ui-graphics-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
            implementation("androidx.compose.ui:compose-ui-text-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
            implementation("androidx.compose.ui:compose-ui-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
            implementation("androidx.compose.foundation:compose-foundation-layout-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
            implementation("androidx.compose.animation:compose-animation-core-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
            implementation("androidx.compose.animation:compose-animation-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
            implementation("androidx.compose.foundation:compose-foundation-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
            implementation("androidx.compose.material:compose-material-ripple-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
            implementation("androidx.compose.material3:compose-material3-wasi:0.0.0-wasi-local") {
                exclude(group = "org.jetbrains.skiko", module = "skiko-wasm-wasi")
            }
        }
    }
}
