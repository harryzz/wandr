---
name: compose-wasi-srcdirs
description: "compose-*-wasi/ dirs (compose-foundation-wasi, compose-ui-wasi, etc.) are gradle bundle projects that srcDirs into compose-multiplatform-core/. The actual source code lives ONLY in compose-multiplatform-core/. Edit there; rebuild the wasi bundle to pick up changes."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 3d303796-d18c-429f-816f-2a415ff40ff3
---

When editing Compose source on the wandr project, the source-of-truth
is `/home/harry/wandr/compose-multiplatform-core/`. The sibling
`compose-*-wasi/` directories (compose-runtime-wasi,
compose-foundation-wasi, compose-ui-wasi, compose-material3-wasi,
etc., 11 in total) do NOT hold copies of the source — their
`build.gradle.kts` uses `srcDirs` to source-link into
compose-multiplatform-core and package the result into a single fat
klib for fast linking.

**Why:** correction 2026-05-19. I described Step 2's instrumentation
work as editing `compose-foundation-wasi`'s `Clickable.kt`. That is
WRONG — `compose-foundation-wasi/src/` contains only
`commonReplacements/` and `wasmWasiActuals/` overrides, no
`Clickable.kt`. The real `Clickable.kt` lives at:

```
/home/harry/wandr/compose-multiplatform-core/compose/foundation/foundation/
  src/commonMain/kotlin/androidx/compose/foundation/Clickable.kt
```

**How to apply:**

- To MODIFY a Compose composable / Modifier / runtime API: edit in
  `compose-multiplatform-core/`, then rebuild the corresponding
  `compose-*-wasi` bundle to republish the fat klib to mavenLocal.
- To find a Compose source file: `find compose-multiplatform-core
  -name "FileName.kt"` (never search `compose-*-wasi/`).
- To find wasi-specific overrides (actual decls, behavioural shims):
  THAT is what lives in `compose-*-wasi/src/wasmWasiActuals/` and
  `commonReplacements/`. Use these for the wasi-specific tail of an
  expect/actual or for replacing a non-wasi-compatible source file.
- After editing core source, the rebuild script that walks all 11
  bundle dirs is `scripts/rebuild-compose-wasi-skiko-depend.sh` (or
  just `./gradlew publishWasmWasiPublicationToMavenLocal` in the
  specific bundle you touched — much faster if only one module is
  affected).

Related: [[rebuild-compose-after-skiko]] for the cross-dependency
rebuild order; [[kotlin-version-bump]] for the version-pin sites
which exist in both trees.
