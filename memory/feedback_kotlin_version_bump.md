---
name: Kotlin Gradle Plugin version bump procedure for the wasi build chain
description: Bumping the Kotlin Gradle Plugin pin (e.g. 2.4.0-Beta2 → 2.4.0-RC) requires updating 13 files and republishing 11 wasi modules → skiko → test-app. `KotlinWasmWasiTargetDsl` is bundled INSIDE `kotlin-gradle-plugin`, not a separate artifact, so the KGP version is the single knob. RC bump didn't fix the continuation-retention leak.
type: reference
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
## What `KotlinWasmWasiTargetDsl` actually is

The class `org.jetbrains.kotlin.gradle.targets.js.dsl.KotlinWasmWasiTargetDsl` lives inside the main `kotlin-gradle-plugin-<ver>-gradle813.jar` artifact (the multiplatform plugin). It is NOT a standalone library — there's no separate `kotlin-wasm-wasi-dsl` artifact to bump. The Kotlin Gradle Plugin version is the single knob for the entire wasmWasi target DSL surface.

Confirmed via:
```bash
find ~/.gradle/caches/modules-2/files-2.1/org.jetbrains.kotlin -name "kotlin-gradle-plugin-*.jar" \
  -exec sh -c 'unzip -l "$1" 2>/dev/null | grep -i WasiTargetDsl' _ {} \;
```

## The 13 version-pin sites in the repo

A bump = `Edit` with `replace_all` on each of these (all use the literal `kotlin("multiplatform") version "X"` and `id("org.jetbrains.kotlin.plugin.compose") version "X"` pattern at lines 6-7):

1. `/home/harry/skiko/dependencies.toml` (skiko's top-level `kotlin =` pin)
2. `/home/harry/skiko/test-app/build.gradle.kts` (compose plugin pin only; KGP is inherited via `kotlin("multiplatform")` without an explicit version → resolves to skiko's `dependencies.toml` value)
3. `/home/harry/wasm-android-runtime/compose-runtime-wasi/build.gradle.kts`
4. `/home/harry/wasm-android-runtime/compose-ui-base-wasi/build.gradle.kts`
5. `/home/harry/wasm-android-runtime/compose-animation-core-wasi/build.gradle.kts`
6. `/home/harry/wasm-android-runtime/compose-ui-graphics-wasi/build.gradle.kts`
7. `/home/harry/wasm-android-runtime/compose-ui-text-wasi/build.gradle.kts`
8. `/home/harry/wasm-android-runtime/compose-foundation-layout-wasi/build.gradle.kts`
9. `/home/harry/wasm-android-runtime/compose-ui-wasi/build.gradle.kts`
10. `/home/harry/wasm-android-runtime/compose-animation-wasi/build.gradle.kts`
11. `/home/harry/wasm-android-runtime/compose-foundation-wasi/build.gradle.kts`
12. `/home/harry/wasm-android-runtime/compose-material-ripple-wasi/build.gradle.kts`
13. `/home/harry/wasm-android-runtime/compose-material3-wasi/build.gradle.kts`

Each `compose-*-wasi/build.gradle.kts` has TWO occurrences of the version string (KGP itself + the compose plugin). Use `replace_all=true` so both are caught.

## Build order after a bump (critical — they depend on each other)

The modules form a DAG via `api("androidx.compose...:compose-*-wasi:0.0.0-wasi-local")` deps in mavenLocal. Republish in dependency order so each module finds its dependencies' fresh klibs:

```bash
for m in compose-runtime-wasi compose-ui-base-wasi compose-animation-core-wasi \
         compose-ui-graphics-wasi compose-ui-text-wasi compose-foundation-layout-wasi \
         compose-ui-wasi compose-animation-wasi compose-foundation-wasi \
         compose-material-ripple-wasi compose-material3-wasi; do
  echo "=== $m ==="
  cd /home/harry/wasm-android-runtime/$m && ./gradlew publishToMavenLocal --console=plain --no-daemon
done
```

Then skiko + test-app:
```bash
cd /home/harry/skiko/skiko && ../gradlew publishToMavenLocal --console=plain --no-daemon
cd /home/harry/skiko/skiko && ../gradlew :test-app:compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon
```

Then the standard wasm-tools → wasmtime → adb chain (per `CLAUDE.md`).

Wall-clock for a fresh bump: roughly 20-25 min total (each module takes 1-3 min to republish; skiko ~4 min; test-app ~2 min; wasmtime AOT ~30 s; APK push and launch ~10 s).

## What does NOT need rebuilding after a Kotlin bump

- **Rust host** (`host/src/*.rs`, `host/cpp/*`) — independent of Kotlin version.
- **Host APK** — same. No reinstall needed unless host code changed.
- **WIT** (`wit/skiko-gfx.wit`) — independent.

So the bump touches only the Kotlin/Compose chain, not the Rust/C++/WIT half.

## How to find the latest Kotlin version

Maven Central only lists stable releases (up to 2.2.0 at time of writing). Pre-releases (Beta/RC) come from the Kotlin team's snapshot/dev repo. The authoritative list of all releases is the JetBrains/kotlin GitHub releases page:

```
https://github.com/JetBrains/kotlin/releases
```

Verified 2026-05-13:
- Latest **stable**: `2.3.21` (2026-04-23)
- Latest **pre-release**: `2.4.0-RC` (2026-05-13)

## What the 2.4.0-Beta2 → 2.4.0-RC bump did NOT fix (and why we did it)

The bump was driven by the indeterminate-ProgressIndicator wasm linear-memory leak (see `feedback_indeterminate_progress_leak.md`). Hypothesis: Kotlin/Wasm coroutine-state-machine retention might have a fix in a newer prerelease.

Soak result (120 s, indeterminate Material3 progress):
- Beta2: ~2.0 MB/s leak
- RC:    ~2.57 MB/s leak

No improvement. The leak is structural to Kotlin/Wasm 2.4 line, not a Beta2-specific regression. Wait for KT- tracker fix for continuation retention OR find an alternative (e.g., compose-multiplatform's experimental Wasm GC tuning).

## Gotchas observed

- **Don't use `sed -i`/`awk` redirect/`python -c '… open(…, "w") …'`** for the bump. Each .gradle.kts is small; `Edit` with `replace_all` is safer and avoids the kind of mass-edit accidents that previously corrupted files in the host crate.
- **`build/` and `.gradle/` directories** carry stale `2.4.0-Beta2` references in cache files — ignore those; the build process refreshes them on next compile.
- **Maven Local cache (`~/.m2/repository/`)** retains old klibs of `0.0.0-wasi-local`. Republishing OVERWRITES them — no clean step needed.
- **Test-app does NOT pin KGP version directly** — only the compose plugin. Test-app's `kotlin("multiplatform")` resolves to whatever skiko's `dependencies.toml` says. So `dependencies.toml` is load-bearing.
