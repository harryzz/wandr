---
name: reference-kotlin-wasm-component-model-status
description: "Kotlin 2.4 'WASM Component Model support' does NOT drop the preview1 adapter — it's the same embed + component-new --adapt flow we use. No escape from the adapter / KT-86415 / freeAll from it. 2026-07-28: native-WASI-0.2 path now has live subtasks (KT-87801 native P2 export In Progress, KT-87723 wasm-tools-in-KGP) but the KT-86415 blocker is UNCHANGED, nothing usable yet; caveat: KT-87801 is CLI/main, our guests are reactors."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 2b58a1a7-2e85-4748-b34a-e9b89ab2de87
  modified: 2026-08-01T13:36:30.717Z
---

Investigated 2026-05-30 (after adopting wasmtime 45, task 65) whether new Kotlin
makes our guest pipeline simpler. **Conclusion: no — don't re-chase this.**

**The "Component Model support" (KT-64569, experimental in Kotlin 2.4.0-Beta2/RC2)
is NOT native single-step component production.** Verified against the official
sample `github.com/Kotlin/sample-wasi-http-kotlin` Makefile — its pipeline is
*exactly ours*:
1. `wit-bindgen kotlin …` (JetBrains fork `github.com/Kotlin/wit-bindgen` `--branch kotlin`)
2. `./gradlew compile…KotlinWasmWasi`  → core wasm module
3. `wasm-tools component embed`
4. `wasm-tools component new … --adapt wasi_snapshot_preview1=wasi_snapshot_preview1.reactor.wasm`  ← **the P1 reactor adapter, same as us**
5. `wasmtime serve/run`

So there is **NO** way from Kotlin 2.4's "component model" to drop the WASI-P1
adapter, the **KT-86415** State-pin patch, the 2.4.258 stdlib override, or the
`freeAllComponentModelReallocAllocatedMemory()` dance ([[wasi-realloc-allocator-pollution]]).
My initial roadmap-headline read ("could collapse our pipeline") was wrong —
reading the actual build steps disproved it; the user's skepticism was correct.

Our own build: `external/kotlin` is master `~2.4.255-SNAPSHOT` (checkout
2026-05-19) and has **zero** new component-model / WASI-P2 codegen — only the
pre-existing `componentModelRealloc`/`cabi_realloc` machinery the adapter flow
already uses. The 2.4 "support" adds nothing native for us.

**The one to WATCH: KT-64568 — "switch the wasm-wasi target of libraries to WASI
Preview 2 / 0.2."** Feb-2026 roadmap status = **In Focus / Planned (NOT done)**.
That is the *only* path that would eventually yield native WASI-0.2 components and
retire the P1 adapter (and with it KT-86415 + the freeAll). Re-check it before
the next big guest-pipeline rework; nothing actionable until it ships.
(Sibling planned items: KT-64569 Component Model, KT-82064 multi-module compilation.)

**Free wins available now (low risk, unrelated to the above):** incremental
Kotlin/Wasm compilation is stable + default (faster rebuilds — `kotlin.incremental.wasm=false`
to disable), and intra-module `.klib` inlining is on by default (2.4.0-RC2). Worth
confirming our build picks these up given the slow Compose compile.

**UPDATE 2026-06-11 (internet re-check, post Kotlin 2.4.0 STABLE release
June 2026):** conclusions unchanged at the pipeline level — the official
sample still ships `wasm-tools component embed` + `component new --adapt`
with the STOCK P1 reactor adapter; no native wasip2 target. New facts:
(1) **KT-86415 is still UNRESOLVED** ("To be discussed", affects
2.4.0-RC and master; master MemoryAllocation.kt still doesn't advance the
parent's availableAddress on child destroy) → our adapter State-pin +
2.4.258 stdlib override stay mandatory. (2) The **Kotlin/wit-bindgen
fork now emits the freeAll discipline itself** (see
[[wit-bindgen-no-kotlin-generator]] update) — our rule became the
official pattern. (3) KT-64569 (Component Model meta) = In Progress,
**planned 2.5.0-Beta1**; the named blocker is the **GC ABI**
(component-model issue #525 pre-proposal: pass values via WasmGC memory
instead of linear) — the long-term path that retires cabi_realloc
pressure for WasmGC languages entirely. (4) KT-64568 (libraries → WASI
0.2) was PAUSED Aug 2025 to prioritize the Wasm Beta (KT-75370).
Watch list: KT-86415, 2.5.0-Beta1, component-model#525.

**UPDATE 2026-07-28 (YouTrack REST re-check — real movement, still nothing
usable yet).** The native-WASI-0.2 path finally has concrete engineering
subtasks (the vague meta-issues sprouted `KT-87xxx`/`KT-88xxx` children), but
**our blocker `KT-86415` is unchanged** so the pipeline stays exactly as above.
Verified via `youtrack.jetbrains.com/api/issues/<ID>?fields=…State…`:
- **KT-86415** (realloc UAF, our adapter State-pin reason) = **To be discussed** —
  UNCHANGED. Adapter State-pin fork + 2.4.258 stdlib override + `freeAll` all
  stay mandatory.
- **KT-64569** (Component Model, meta) = **In Progress**, now with children:
  ✅ KT-87207 "upgrade kotlinx libs & stdlib to WASI 0.2" = **Fixed**;
  ✅ KT-85008 "publish a demo app using an early CM version" = **Fixed**
     (a Kotlin-team CM demo now EXISTS — check whether it still uses the P1
     adapter before trusting any "native" read);
  🔨 KT-87224 "Component Model: Explore GC ABI" = Open (this IS the
     component-model#525 GC-ABI work);
  🔨 KT-87950 "how to handle uncaught exceptions" = Open.
- **KT-64568** (libraries → WASI 0.2) = **Open** (was Paused; now live), children:
  🔨 **KT-87801** "Generate `wasi:cli/run` export in main module for WASI
     preview 2" = **In Progress** ← the actual native-P2 codegen;
  🔨 KT-87723 "Integrate `wasm-tools component` with KGP" = Open (folds our
     manual `embed`+`component new --adapt` INTO the Gradle plugin);
  🔨 KT-88027 "Export `cabi_realloc` from stdlib for CM" = Open.
- KT-75370 (Wasm-js → Beta, which had paused KT-64568) = **Fixed** (Sep 2025) →
  KT-64568 unblocked.
- Latest Kotlin: **2.4.10 stable** (2026-07-14) + **2.4.20-Beta2** (2026-07-22);
  **no 2.5.0-Beta1 yet**, so KT-64569's mooted 2.5.0-Beta1 target hasn't opened.

**Meaning for wandr:** the escape trio (KT-87801 native P2 export + KT-87723
wasm-tools-in-KGP + KT-88027 stdlib `cabi_realloc`) is finally being built — when
it lands, Kotlin emits a component natively and the P1 adapter (+ KT-86415 +
`freeAll`) can go. NOT there yet. **Caveat: KT-87801 targets `wasi:cli/run` in a
MAIN module (a CLI export); our guests are REACTORS (cdylib, no `main`,
host-driven via WIT imports)** — first-cut native P2 may only serve CLI/main apps,
so a reactor export mode is the thing to confirm. **Re-check trigger:** KT-87801
+ KT-87723 → Fixed; then verify reactor support AND that the adapter is actually
gone. New watch list: KT-86415, KT-87801, KT-87723, KT-88027, KT-87224.

**UPDATE 2026-08-01 (multi-module compilation — relevant to the shared-lib/app-size
story, NOT the adapter).** **KT-86919 = Fixed** (~mid-2026, ~2.4.x): Gradle
`kotlin.wasm.compilationMode = monolith | multimodule-open-world |
multimodule-closed-world` → a separate wasm(+mjs) file per klib. First crack in the
"whole-program-compiles the framework into each app" wall. **BUT wasm-JS-target
ONLY** — inter-module linking = **JS ES-modules** (`.mjs` glue, "Js module imports"
KT-81564, js-builtins.mjs, nodejs, Index.html); **zero `wasmWasi` support**; motivated
by **build speed**, not distribution/shared-libs. The gate is 100% Kotlin-side
(backend emits JS-wired multimodule + KGP JS-pipeline-only + wasi target = single
component, downstream of Component Model KT-64569) — NOT the wasm platform and NOT
wandr's host (which already composes components via `wire_dep_into_linker` / `link.wac`).
Maturity: ~15 Fixed / 6 Open (cross-module exports KT-81595 + closed-world KGP KT-84108
Open). Sub-finding: KT-75871 (new RTTI) + KT-74992 (interface virtual calls) = Kotlin
DID build cross-module WasmGC dispatch, but Kotlin-internal + JS-target. Full writeup:
`docs/kotlin-wasm-multimodule.md`. Not usable/testable for wandr guests yet; actionable
= ask JetBrains re: wasmWasi multimodule + distribution intent.

Related: [[reference_wasm_dynamic_linking_shared_libs]], [[wit-bindgen-no-kotlin-generator]], [[wasi-realloc-allocator-pollution]],
[[kotlin-wasm-scopedmemory-destroy-bug]].
