# ~/wandr — Project state on session handoff

> **2026-05-15 — RESOLVED.** The "Option A port" described in this document
> has shipped. Full Compose-on-WASM PoC works end-to-end on device. This
> file is kept as historical record of the strategy that was decided
> mid-port. For current state, see:
>
> - `CLAUDE.md` — overall guide + task table (all ✅)
> - `docs/repository-layout.md` — canonical "where does X live" reference (post-monorepo-merge, task 52 + 53)
> - `docs/architecture-runtime.md`, `architecture-ime.md`, `architecture-host-guest-boundary.md` — design docs
> - `runtime/wandr-host/README.md` + `BUILD.md` — Rust host
> - `apps/user/wandr-app/README.md` + `BUILD.md` — Kotlin/Compose guest
> - `external/skiko/README-wasmWasi.md` + `BUILD-wasmWasi.md` — skiko fork (submodule)
> - `external/compose-multiplatform-core/README-wasmWasi.md` + `BUILD-wasmWasi.md` — the port itself (submodule)
> - `.task-state` — last checkpoint
> - project memory `project_wasm_runtime.md`

**Updated:** 2026-05-14, mid-session checkpoint during Option A port.

## TL;DR

The Option A port for compose-multiplatform-core is much larger than the initial STATE.md suggested. Key new findings this session:
1. The JetBrains fork uses **compatibility-stubs** for 16 modules (collection, lifecycle-*, savedstate-*, navigation*, runtime, runtime-saveable, annotation, window-core, navigationevent-compose) that redirect to published `androidx.*` artifacts on maven. **No source build inside the fork** for these.
2. wasmJs works for stubbed modules because **JetBrains publishes wasmJs variants** to maven. wasmWasi has **no published variant** — so for wasmWasi, we have to build from source.
3. Kotlin 2.3.20 (the fork's version) doesn't reliably register `compileKotlinWasmWasi` tasks. **Bumped to 2.4.0-RC** this session.
4. Strategy decided: **add wasmWasi target to each compatibility-stub**, pulling source from the corresponding real source dir via `kotlin.srcDirs`. Sibling-pattern moved INSIDE the fork.

## Decided strategy — "Stub + wasmWasi srcDirs"

For each compatibility-stub:
- Keep all existing targets unchanged (ios/jvm/js/wasmJs/etc. continue to resolve maven artifacts)
- Add `wasmWasi()` to androidXMultiplatform block
- Add intermediate sourceset `wasmWasiUpstreamCommon` (so upstream commonMain has its own scope, separate from stub's commonMain)
- `wasmWasiUpstreamCommon.dependsOn(commonMain)` and contains `srcDirs` for the upstream module's `commonMain` + annotation stubs
- `wasmWasiMain.dependsOn(wasmWasiUpstreamCommon)` and contains `srcDirs` for upstream's `nonJvmMain` + `webMain` (the actuals)
- `configurations.matching { it.name.startsWith("wasmWasi") }.all { exclude group: "androidx.X", module: "X" }` to drop the maven dep that propagates from commonMain

## Status

### What's done this session

1. **Kotlin 2.4.0-RC bump** ✅
   - `gradle/libs.versions.toml`: composeCompilerPlugin, kotlin, added kotlin24, kotlinGradlePluginAnnotations, kotlinGradlePluginApi, kotlinNativeUtils, kotlinToolingCore — all bumped to 2.4.0-RC
   - `buildSrc/public/.../JetBrainsCompatibilityVersions.kt`: `JETBRAINS_COMPILE_KOTLIN_VERSION = KOTLIN_2_3`
   - `buildSrc/public/.../AndroidXConfiguration.kt`: added `KOTLIN_2_4(KotlinVersion.KOTLIN_2_4, "kotlin24")`, `LATEST(KOTLIN_2_4)`
   - `buildSrc/shared.gradle`: languageVersion KOTLIN_2_2 (was KOTLIN_2_1, deprecated in 2.4)
   - `buildSrc/private/.../TestSourceSetsHelper.kt`: `@Suppress("DEPRECATION")` added for kotlin-android sourceSets deprecation

2. **AndroidXMultiplatformExtension.kt** ✅ (was already partially done from previous session)
   - `fun wasmWasi(block)` registers PlatformIdentifier.WASM_WASI and calls `kotlinExtension.wasmWasi { binaries.library() }`
   - `applyAndroidXDefaultHierarchyTemplate` includes `group("wasmWasi") { withWasmWasi() }` under nonJvm
   - Earlier bug fix: `project.buildFeatures` should be `buildFeatures` (extension property, not project property)

3. **Proof-of-concept: collection-compatibility-stub** 🟡 IN PROGRESS
   - Added `wasmWasi()` to androidXMultiplatform block
   - Added `wasmWasiUpstreamCommon` intermediate sourceset
   - srcDirs configured to pull `collection/collection/src/{commonMain,nonJvmMain,webMain}`
   - exclude block for `androidx.collection:collection` (since maven variant doesn't exist for wasmWasi)
   - Annotation stubs copied to `src/wasmWasiUpstreamCommon/kotlin/androidx/annotation/Annotations.kt`
   - **Still failing** with:
     - **ABI mismatch**: "Kotlin/Wasm standard library has the ABI version (2.3.0) ... compiler's current ABI compatibility level (2.4)". Some maven dep is forcing kotlin-stdlib-wasm-wasi to 2.3.0. Need to investigate or force-pin to 2.4.0-RC.
     - **Annotation visibility**: Annotations.kt declarations need `public` prefix (collection has explicit-api mode enabled). Half-done via sed, but inner companion/enum classes still need it.
     - **ExperimentalContracts opt-in**: collection's commonMain uses `kotlin.contracts.ExperimentalContracts` but our stub's wasmWasi language settings don't opt in.

### What's NOT done

| # | Module/Task | Type | Status |
|---|-------------|------|--------|
| 1 | `:annotation:annotation` (stub) | wasmWasi via srcDirs | ⏳ |
| 2 | `:collection:collection` (stub) | wasmWasi via srcDirs | 🟡 in-progress |
| 3 | `:compose:runtime:runtime` (stub) | wasmWasi via srcDirs | ⏳ |
| 4 | `:compose:runtime:runtime-saveable` (stub) | wasmWasi via srcDirs | ⏳ |
| 5 | `:lifecycle:lifecycle-common` (stub) | wasmWasi via srcDirs | ⏳ |
| 6 | `:lifecycle:lifecycle-runtime` (stub) | wasmWasi via srcDirs | ⏳ |
| 7 | `:lifecycle:lifecycle-runtime-compose` (stub) | wasmWasi via srcDirs | ⏳ |
| 8 | `:lifecycle:lifecycle-viewmodel` (stub) | wasmWasi via srcDirs | ⏳ |
| 9 | `:lifecycle:lifecycle-viewmodel-compose` (stub) | wasmWasi via srcDirs | ⏳ |
| 10 | `:lifecycle:lifecycle-viewmodel-savedstate` (stub) | wasmWasi via srcDirs | ⏳ |
| 11 | `:navigation:navigation-common` (stub) | wasmWasi via srcDirs | ⏳ |
| 12 | `:navigation:navigation-runtime` (stub) | wasmWasi via srcDirs | ⏳ |
| 13 | `:navigationevent:navigationevent-compose` (stub) | wasmWasi via srcDirs | ⏳ |
| 14 | `:savedstate:savedstate` (stub) | wasmWasi via srcDirs | ⏳ |
| 15 | `:savedstate:savedstate-compose` (stub) | wasmWasi via srcDirs | ⏳ |
| 16 | `:window:window-core` (stub) | wasmWasi via srcDirs | ⏳ |
| 17 | `:compose:ui:ui-util` | wasmWasi to real-src project | ⏳ |
| 18 | `:compose:ui:ui-geometry` | wasmWasi to real-src project | ⏳ |
| 19 | `:compose:ui:ui-unit` | wasmWasi to real-src project | ⏳ |
| 20 | `:compose:ui:ui-graphics` | wasmWasi to real-src project | ⏳ |
| 21 | `:compose:ui:ui-text` | wasmWasi to real-src project | ⏳ |
| 22 | `:compose:ui:ui-backhandler` | wasmWasi to real-src project | ⏳ |
| 23 | `:compose:ui:ui` | wasmWasi to real-src project | ⏳ |
| 24 | `:compose:foundation:foundation-layout` | wasmWasi to real-src project | ⏳ |
| 25 | `:compose:foundation:foundation` | wasmWasi to real-src project | ⏳ |
| 26 | `:compose:animation:animation-core` | wasmWasi to real-src project | ⏳ |
| 27 | `:compose:animation:animation` | wasmWasi to real-src project | ⏳ |
| 28 | `:compose:material:material-ripple` | wasmWasi to real-src project | ⏳ |
| 29 | `:compose:material3:material3` | wasmWasi to real-src project | ⏳ |
| 30 | Wire test-app to new Option A modules | | ⏳ |

## Specific blocker for collection PoC

The current `collection/collection-compatibility-stub/build.gradle` and `src/wasmWasiUpstreamCommon/kotlin/androidx/annotation/Annotations.kt` produce:
- ABI mismatch on kotlin-stdlib-wasm-wasi (2.3.0 vs compiler 2.4)
- Explicit-api visibility errors in Annotations.kt (companion/enum need `public`)
- `kotlin.contracts.ExperimentalContracts` opt-in missing

**Next-session recommended first step:** investigate where kotlin-stdlib-wasm-wasi 2.3.0 comes from. Check `~/.m2/`, gradle caches, and whether some maven dep chain pulls in old version. Likely need explicit `implementation("org.jetbrains.kotlin:kotlin-stdlib-wasm-wasi:2.4.0-RC")` in wasmWasiMain dependencies, OR add it to `configurePinnedKotlinLibraries` in AndroidXMultiplatformExtension.kt (which currently only handles JS/WASM_JS).

## Underlying observations

- The fork has **artifactRedirection** (in `JetBrainsExtensions.substituteForRedirectedPublishedDependencies()`) that substitutes project deps with maven deps for native Konan targets. wasmWasi is not handled by this — we want the opposite: substitute maven dep with srcDirs source.
- The `kotlinExtension.wasmWasi { ... }` DSL is available in 2.4.0-RC, and so is `KotlinWasmWasiTargetDsl`. The hierarchy template's `withWasmWasi()` is available.
- `compileKotlinWasmWasi` tasks DO register correctly once Kotlin is bumped to 2.4.0-RC.
- Diagnostic dead-end during this session: spent significant time trying to figure out why `wasmWasi()` in `collection/collection/build.gradle` didn't trigger a println. **Cause**: `:collection:collection` maps to `collection/collection-compatibility-stub` in settings.gradle, not to `collection/collection/`. Editing the wrong file. Same applies to runtime, runtime-saveable, and the AndroidX deps.

## Files modified this session

```
compose-multiplatform-core/
  gradle/libs.versions.toml                       # Kotlin bumps
  buildSrc/shared.gradle                          # languageVersion KOTLIN_2_2
  buildSrc/public/.../JetBrainsCompatibilityVersions.kt  # KOTLIN_2_3
  buildSrc/public/.../androidx/build/AndroidXConfiguration.kt  # added KOTLIN_2_4, LATEST=KOTLIN_2_4
  buildSrc/private/.../AndroidXMultiplatformExtension.kt  # buildFeatures fix (probe code reverted)
  buildSrc/private/.../testConfiguration/TestSourceSetsHelper.kt  # @Suppress("DEPRECATION")
  collection/collection-compatibility-stub/build.gradle  # wasmWasi() + wasmWasiUpstreamCommon + exclude block
  collection/collection-compatibility-stub/src/wasmWasiUpstreamCommon/kotlin/androidx/annotation/Annotations.kt  # NEW (annotation stubs)
```

(Other files edited then reverted: `collection/collection/build.gradle`, `settings.gradle`. Probe/debug code in `AndroidXMultiplatformExtension.kt` and `collection/collection/build.gradle` was reverted.)

## ~~Cumulative state from previous sessions (still applicable)~~ — superseded 2026-05-29

The points below were "still applicable" at 2026-05-14 mid-port.
They are no longer current. Documented post-hoc:

- **Skiko fork.** Still publishes `org.jetbrains.skiko:skiko-wasm-wasi`
  to `~/.m2/`. Now lives as a submodule at `external/skiko/`
  (was `~/wandr/skiko/`, then a clone of
  `codeberg.org/harryzz/skiko`).
- **Sibling wasi modules (Option B fallback).** Abandoned. The
  11 `compose-*-wasi/` bundler dirs are not used anymore — the
  Option A port (the topic of this STATE.md) shipped via
  Compose's in-fork `wasmWasi()` targets pulled in through the
  `external/compose-multiplatform-core/` submodule. The
  bundlers have been gone since the monorepo reorg (task 52 +
  53, 2026-05-28).
- **Host, WIT, scripts, test-app.** All since touched many times.
  Current locations: `runtime/wandr-host/`, `wit/`,
  `tools/scripts/`, `apps/user/wandr-app/`. See
  `docs/repository-layout.md`.
