# Kotlin/Wasm multi-module compilation (KT-86919) — findings & the wasm-js gate

> **TL;DR.** Kotlin/Wasm can now emit a **separate wasm file per module** — Gradle
> property `kotlin.wasm.compilationMode = monolith | multimodule-open-world |
> multimodule-closed-world` (**KT-86919, Fixed ~mid-2026**). It's the first real
> crack in the "compiler whole-program-compiles the framework into each app" wall.
> **But it's wired for the wasm-*js* target only** (inter-module linking = JS
> ES-modules / `.mjs` glue), it's **motivated by build speed, not distribution**,
> and it does **not** work on our **`wasmWasi`** target. So it does **not** yet let
> us ship Compose once as a shared lib for wandr guests. Researched 2026-08-01;
> companion to [`wasm-dynamic-linking.md`](wasm-dynamic-linking.md) and
> [`shared-runtime-and-app-size.md`](shared-runtime-and-app-size.md).

## What it is

`kotlin.wasm.compilationMode` (in `gradle.properties`) selects one of three modes:

| Mode | Meaning |
|---|---|
| **`monolith`** (default) | Whole project + all deps compiled as one unit → one wasm module. Today's behavior. |
| **`multimodule-open-world`** | **Total module independence** — each klib compiled *independently* to its own wasm file. |
| **`multimodule-closed-world`** | In between — all modules compiled *together* (closed world) but emitted as *multiple* wasm files that stay linked. |

Each Kotlin "module" (klib) → a separate **`wasm` + `.mjs`** file (KT-82064).

**Motivation is build-time, not distribution.** KT-82064: *"compile all modules
separately and spend fewer resources… work is stretched out in time"* (incremental
/ parallel builds). KT-87258 adds a *"multimodule only for **development**"* value.
There is **no** stated goal of shipping a framework once as a shared library.

## Maturity (issue cluster: ~15 Fixed / 6 Open)

- **Fixed / working:** the `compilationMode` config (KT-86919), Gradle validation
  (KT-87565), **incremental compilation** (KT-84396), **JS module imports** between
  modules (KT-81564), pointer usages (KT-87639), IC-cache invalidation (KT-87583),
  and — notably — **new WasmGC RTTI** (KT-75871) + **new interface virtual calls**
  (KT-74992).
- **Still Open:** **cross-module wasm *exports*** (KT-81595), **closed-world KGP
  support** (KT-84108) + its tests (KT-84110), eager-initializers (KT-83579), some
  IC tests (KT-84599).

⇒ **open-world + incremental** is the more-complete path; **closed-world Gradle
support is unfinished**, and cross-module exports still have Open bugs.

**Version:** resolved ~mid-2026, so ~**Kotlin 2.4.x** era — but the exact fix
version wasn't exposed by the tracker, and the **public docs don't cover
`compilationMode` yet** (treated as new/internal).

## 🚩 The gate: it's wasm-**js** only, and the gate is entirely Kotlin-side

The whole multimodule pipeline targets **wasm-js**: inter-module references are
resolved as **JS module imports** (KT-81564), the output is `wasm + .mjs` glue,
and the surrounding work is `js-builtins.mjs` generation (KT-85075), Node.js
runners (KT-63400), `Index.html` (KT-68728). A search for `wasmWasi + multimodule`
returns **essentially nothing**. So:

**The "wire module A→B at load" step is implemented as the JS host's ES-module
loader.** That mechanism only exists on wasm-js. Where the gate sits, layer by
layer:

1. **Compiler backend — the output *format*.** It *can* emit separate modules, but
   expresses cross-module refs as **JS module imports** + `.mjs` glue, not as
   host-linkable **wasm imports**. So the multi-module *shape* is JS-wired. *(core gate)*
2. **Tooling (KGP).** Gradle multimodule support targets the **JS pipeline only**
   (nodejs/browser/Index.html). No `wasmWasi` multimodule pipeline exists.
3. **Runtime loader — why they chose JS.** On wasm-js the JS engine's ES-module
   loader links the modules for free. On `wasmWasi` there's **no ES-module loader**
   — linking would be done by the **WASI/component host**. Kotlin implemented the
   easy JS path.
4. **Strategic dependency.** Kotlin's `wasmWasi` target is **one core module wrapped
   as a single component** (P1 adapter). A multi-module *wasi* story lives in
   **component composition**, which is **downstream of Kotlin's Component Model
   (KT-64569, In Progress)** + wasi-0.2 (KT-64568). Not a config flip.

### The gate is NOT the wasm platform, and NOT wandr's host
wasm can link modules on any target, and **wandr's host already composes wasm
components** — `wire_dep_into_linker` (`runtime/wandr-host/src/app_loader.rs:486/658`)
wires a dependency into the consumer's linker, and a **`link.wac` multi-component
path** exists (currently gated to single-component apps, `app_loader.rs:760`). So
the host-side linker capability is on our side already. **If Kotlin emitted a
host-linkable multi-module form (wasm imports resolved by the host, not JS
`.mjs` imports), wandr could link it.** The missing piece is 100% Kotlin's
backend + KGP producing that wasi form.

## The one Wall-#2-relevant sub-finding
KT-75871 (new RTTI) + KT-74992 (new interface virtual calls) being Fixed means
Kotlin **did build a cross-module WasmGC type-info + virtual-dispatch mechanism** to
make multimodule work — i.e. the "WasmGC cross-module types" gap (Wall #2 in
[`wasm-dynamic-linking.md`](wasm-dynamic-linking.md)) is **partly solved, but as a
Kotlin-internal convention for the wasm-js world**, not a wasm-standard
`type-imports` mechanism, and not for wasi / cross-app sharing.

## What unlocking `wasmWasi` multimodule would take from Kotlin (not a flip)
1. Backend emits cross-module refs as **wasm imports** (host-resolved), not JS
   module imports — with a wasi equivalent for whatever JS-value bridging crosses
   module boundaries (the RTTI/virtual-dispatch rework may partly transfer).
2. A **`wasmWasi` multimodule pipeline in KGP** (host-linkable output, not
   nodejs/browser bundles).
3. Express it as **component composition / shared-everything core-module linking**
   → downstream of **KT-64569** (Component Model).

## wandr implications

- **Not testable/usable for our guests yet.** Our Compose guests are `wasmWasi`
  (wasip1 adapter → wasip2); multimodule is wasm-js-only, so there's no way to test
  "share Compose" through it in our pipeline today.
- **The actionable move** is upstream: ask on the Kotlin Slack / `discuss.kotlinlang.org`
  whether multimodule will come to the **`wasmWasi`** target and whether it's
  intended for **distribution / shared libraries** (vs just build speed). That's the
  exact gating question for us, and it's currently unanswered.
- **Ties to the size story:** even if it lands on wasi, it's the *compiler half*
  (Wall #1) — sharing Compose on-disk across apps still needs the *runtime/spec
  half* (WasmGC cross-module type identity + a GC linking model). Near-term the only
  lever remains the **framework-base zygote** (share the runtime in RAM,
  [`shared-runtime-and-app-size.md`](shared-runtime-and-app-size.md)).

## Tracking

| Issue | What | State |
|---|---|---|
| **KT-86919** | `kotlin.wasm.compilationMode` (the 3 modes) | **Fixed** (~mid-2026) |
| KT-82064 | multi-module support in the Gradle plugin (umbrella) | In Progress |
| KT-81595 | cross-module wasm exports | **Open** |
| KT-84108 | closed-world KGP support | **Open** |
| KT-75871 / KT-74992 | new WasmGC RTTI / interface virtual calls (cross-module dispatch) | Fixed |
| KT-64569 / KT-64568 | Component Model / wasi-0.2 (the wasi-multimodule dependency) | In Progress / Open |

## See also
- [`wasm-dynamic-linking.md`](wasm-dynamic-linking.md) — the general shared-lib
  mechanism + the linear-vs-WasmGC × runtime-vs-compiler matrix.
- [`shared-runtime-and-app-size.md`](shared-runtime-and-app-size.md) — why apps are
  big + the zygote-COW sharing story.
- Memory: `[[reference_kotlin_wasm_component_model_status]]`,
  `[[reference_wasm_dynamic_linking_shared_libs]]`.
