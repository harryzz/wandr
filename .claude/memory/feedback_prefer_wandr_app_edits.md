---
name: prefer-wandr-app-edits
description: "Strongly prefer wandr-app/ edits over compose-multiplatform-core/ edits for tests, repros, and instrumentation. Only edit compose-multiplatform-core when no wandr-app-side alternative exists, and ASK the user first before doing so."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 3d303796-d18c-429f-816f-2a415ff40ff3
---

When writing test harnesses, bisect cards, repros, smoke tests, or
instrumentation for the wandr project, prefer changes in
`/home/harry/wandr/wandr-app/` over `/home/harry/wandr/compose-multiplatform-core/`.

**Why:** the compose-multiplatform-core tree is the upstream Compose
source (with wasi-adapter overlays via the compose-*-wasi bundle
projects — see [[compose-wasi-srcdirs]]). Edits there are heavier:
they require a `compose-*-wasi` republish cycle (15-25 min) and risk
drifting from the upstream Compose tree. Edits in `wandr-app/` only
need a wandr-app recompile (~2 min) and stay local to this project.
User-stated preference 2026-05-19.

**How to apply:**

- For Compose-side bisect/repro work, default to:
  1. A new or modified file under `wandr-app/src/wasmWasiMain/kotlin/`
     (e.g. `TooltipInspectionCard.kt`-style smoke cards).
  2. Using public Compose APIs available to the wandr-app, even if it
     means hand-replicating a private wrapper structure rather than
     calling the real one.

- Only edit `compose-multiplatform-core/` when:
  1. The behaviour can't be observed or modified from outside (e.g.
     internal Modifier.Node lifecycle hooks, private state machines
     in `BasicTooltipState` / `ClickableNode`).
  2. The bug requires logging *inside* an upstream API to diagnose.

- **Even then, ASK FIRST.** A short message explaining (a) what
  observation requires the core edit, (b) what minimum-touch change
  would suffice, (c) whether it lands in commonMain or in a
  wasmWasi-actual override under `compose-*-wasi/src/wasmWasiActuals/`.

- If the user declines, prefer a black-box workaround in wandr-app
  (e.g. wrap with a custom Modifier that observes pointer events
  externally; use `Snapshot.observe` from outside; add log lines in
  the surrounding composable rather than the internal API).
