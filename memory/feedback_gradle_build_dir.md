---
name: Gradle build directory for test-app
description: The correct working directory for Kotlin wasmWasi builds of the test-app
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
Always run Gradle builds from `/home/harry/skiko/skiko/`, not `/home/harry/skiko/`.

The root `/home/harry/skiko/settings.gradle.kts` includes `:skiko` as an external build and has no sub-projects itself — running tasks from there will fail with "project 'test-app' not found".

**Why:** The `test-app` project is declared in `/home/harry/skiko/skiko/settings.gradle.kts` via `include("test-app")`.

**How to apply:** When building the WASM component, use:
```
cd /home/harry/skiko/skiko
./gradlew :skiko:wasmWasiJar ...
./gradlew :test-app:compileProductionExecutableKotlinWasmWasi ...
```
