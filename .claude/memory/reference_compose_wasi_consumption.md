---
name: reference-compose-wasi-consumption
description: "Compose-wasi guests must depend on the in-tree org.jetbrains.compose.*:*-wasm-wasi:9999.0.0-SNAPSHOT modules (BUILD-wasmWasi.md), NOT the discarded out-of-tree compose-*-wasi:0.0.0-wasi-local fat bundles"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 2b58a1a7-2e85-4748-b34a-e9b89ab2de87
---

**A Compose/Kotlin wasi guest's `wasmWasiMain` deps must point at the in-tree
`org.jetbrains.compose.*:*-wasm-wasi:9999.0.0-SNAPSHOT` modules**, published from
`external/compose-multiplatform-core` per its `BUILD-wasmWasi.md`. The
authoritative doc:
- **Step 3** publishes the 13 real-source modules:
  `:compose:ui:ui-util/ui-geometry/ui-unit/ui-graphics/ui-text/ui-backhandler/ui`,
  `:compose:foundation:foundation-layout/foundation`,
  `:compose:animation:animation-core/animation`, `:compose:material:material-ripple`,
  `:compose:material3:material3` (+ runtime/runtime-saveable stubs), each via
  `:X:Y:publishWasmWasiPublicationToMavenLocal`.
- **Step 4 (consuming)** = depend on `org.jetbrains.compose.*:*-wasm-wasi:9999.0.0-SNAPSHOT`.
- **Step 5 (incremental)** = after editing one module, republish just it:
  `:compose:ui:ui:publishWasmWasiPublicationToMavenLocal`. An ADDITIVE change
  (new public method, not inline) needs ONLY that module — downstream klibs
  reference unchanged symbols and link fine; the consumer recompiles fresh.

**TRAP (cost me a long detour 2026-05-30):** `wart-app` + `war.ime.keyboard`
`build.gradle.kts` were still depending on the **OLD DISCARDED out-of-tree fat
bundles** `androidx.compose.*:compose-*-wasi:0.0.0-wasi-local` (produced by
`/home/harry/wasm-android-runtime/compose-*-wasi`, which srcDirs a stale
out-of-tree `wasm-android-runtime/compose-multiplatform-core` — NOT the in-tree
`external/compose-multiplatform-core` where edits go). So an in-tree compose edit
+ correct `:compose:ui:ui` republish produced `ui-wasm-wasi:9999.0.0-SNAPSHOT`
with the change, but the guest kept linking the May-13 bundle → `Unresolved
reference`. **Fix (done 2026-05-30):** swapped both guests' `wasmWasiMain.dependencies`
from the `compose-*-wasi:0.0.0-wasi-local` bundles to the explicit 15
`*-wasm-wasi:9999.0.0-SNAPSHOT` modules. Don't MIX bundle + module klibs (→ "same
unique_name in more than one library"). The dependency-substitution + exclude
block in each guest's `configurations.matching { wasmWasi }` already lists these
module coords for transitive redirect.

How to tell which klib a guest really links: `find ~/.m2 ~/.gradle/caches -name
"*.klib"` and grep each for the symbol + class. Supersedes the "11 fat klibs /
~24× faster" comment that used to be in the guests' build.gradle.kts (that was
the discarded approach). Related: [[feedback_rebuild_compose_after_skiko]],
[[feedback_compose_wasi_srcdirs]], [[reference_on_demand_rendering]].
