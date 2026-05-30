---
name: reference-kotlin-wasm-component-model-status
description: "Kotlin 2.4 'WASM Component Model support' does NOT drop the preview1 adapter — it's the same embed + component-new --adapt flow we use. No escape from the adapter / KT-86415 / freeAll from it. The one to WATCH is KT-64568 (WASI Preview 2 target switch), still Planned."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 2b58a1a7-2e85-4748-b34a-e9b89ab2de87
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

Related: [[wit-bindgen-no-kotlin-generator]], [[wasi-realloc-allocator-pollution]],
[[kotlin-wasm-scopedmemory-destroy-bug]].
