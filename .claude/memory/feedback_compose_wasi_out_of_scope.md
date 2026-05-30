---
name: compose-wasi-out-of-scope
description: "The compose-*-wasi/ directories (compose-runtime-wasi, compose-material3-wasi, all 11 bundle modules) are OUT OF SCOPE for new work. They will be deleted by the user. Do not develop new files, patterns, or overrides in them. Wasi-specific code goes in compose-multiplatform-core/.../wasmWasiMain/."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 3d303796-d18c-429f-816f-2a415ff40ff3
---

The 11 `compose-*-wasi/` directories at `/home/harry/wart/` are
transient build-glue that bundles `compose-multiplatform-core` source
into fat klibs for fast linking. The user plans to **delete them
after the current task finishes**. They have noticed I keep looking
at them and explicitly flagged this.

**Why:** correction 2026-05-19. I proposed placing a wasi-specific
`BasicTooltip.kt` override in `compose-material3-wasi/src/
wasmWasiActuals/` (where two existing files,
`PlatformDateFormat.wasi.kt` and `IdentityHashCode.wasi.kt`, already
live as duplicates of files in upstream `wasmWasiMain/`). The user
pointed out (a) `compose-*-wasi` is going away, so new work there is
wasted, and (b) `compose-multiplatform-core/.../wasmWasiMain/` is
ALREADY the existing wasi-specific source set (created by our own
`2595b5e6f5e poc wasmWasi port` commit) and is the right long-term
home.

**How to apply:**

- For any **wasi-specific Kotlin code** (overrides for upstream
  files, actual decls for expect/actual pairs, wasi-flavored
  reimplementations): the right destination is
  `compose-multiplatform-core/<module>/src/wasmWasiMain/kotlin/`.
  Example: a wasi BasicTooltip override goes at
  `compose-multiplatform-core/compose/material3/material3/src/
  wasmWasiMain/kotlin/androidx/compose/material3/internal/
  BasicTooltip.wasi.kt`.

- For **tests, repros, smoke harnesses**: still wart-app per
  [[prefer-wart-app-edits]]. compose-*-wasi is not the right home
  for those either.

- For **build wiring** that points to upstream `wasmWasiMain/`
  source dirs: minimally OK to edit `compose-*-wasi/build.gradle.kts`
  if needed during this task, but treat any such edit as scaffolding
  that will get cleaned up when compose-*-wasi is deleted. Don't
  develop new patterns there.

- The duplicate files (`PlatformDateFormat.wasi.kt`,
  `IdentityHashCode.wasi.kt`) that exist in both
  `compose-multiplatform-core/.../wasmWasiMain/` and
  `compose-material3-wasi/src/wasmWasiActuals/` are byte-identical.
  The compose-*-wasi copies are the ones currently compiled (per
  the existing build.gradle.kts srcDirs). The upstream copies are
  presently orphaned but are the canonical home. They will be
  consolidated when compose-*-wasi is deleted.

Related: [[compose-wasi-srcdirs]] (the bundler vs. source
distinction — but supersede with this note for the "where do new
files go" question).
