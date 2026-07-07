# wandr — architecture docs

Living technical docs for the wandr runtime: a **portable UI runtime for
WASM apps** — guests compiled once against OS-agnostic WIT contracts, run
natively on any OS wandr has a backend for. Android is the production
backend; Linux is a working desktop/dev backend.

These docs answer the questions that come up repeatedly while
reading the code. They are NOT user-facing reference. For setup +
build instructions see `~/wandr/CLAUDE.md` and each subproject's
`BUILD.md`. For the task narrative see `~/wandr/tasks/`.

The index below is organized by the **layer model** (guests → contracts →
runtime → OS backends). If you're new, start with the overview.

## Start here

- [`overview.md`](overview.md) — **the front-door doc**: what wandr is, the
  four-layer model, and an honest per-backend / per-framework maturity matrix.
- 🎥 [demo video](https://youtube.com/shorts/rR4TG-I5Y58) — screen recording of
  wandr running on a Pixel 2 XL.

## Runtime & host — the core

| Doc | Audience | What it answers |
|---|---|---|
| [`architecture-host-guest-boundary.md`](architecture-host-guest-boundary.md) | Anyone touching a WIT contract or `wandr-host/src/*_impl.rs` | What is `renderFrame(nanos)` — is it inlined? What does "host-driven" mean? How does a single frame flow across the WASM Component-Model boundary? |
| [`architecture-runtime.md`](architecture-runtime.md) | Anyone touching `wandr-host` / `wandr-arbiter`, or debugging app launch / focus / lifecycle | What are the three processes (zygote, arbiter, host child) and how do they talk? Full transport tables for the three UNIX sockets + the three signals. End-to-end trace of `wandr-arbiter launch <app>`. |
| [`architecture-ime.md`](architecture-ime.md) | Anyone touching `wandr.ime.keyboard`, the lang plugins, or `ime_inbound` / `keyboard_host_impl` | How does a soft-keyboard tap become a Compose `KeyEvent` in the focused TextField? How are lang plugins (`wandr.lang.bg` / `.fr`) loaded, and what's TODO (task 51) to make plugin loading dynamic? |
| [`repository-layout.md`](repository-layout.md) | Anyone adding a new app, system component, native binary, or vendored upstream | Where does it live? What do I name it? Canonical top-level categories (`apps/`, `runtime/`, `wit/`, `external/`, `tools/`, `repros/`) and the `wandr-*` vs `wandr.*` naming rule. Mid-migration as of 2026-05-28 — see task 52. |
| [`build-pipeline.md`](build-pipeline.md) | Anyone building a guest or the host | Build pipeline, WIT-sync rule, cwasm/adapter/deploy, dev environment. |
| [`host-rendering.md`](host-rendering.md) | Anyone touching `canvas_impl.rs` / skiko files / the Kotlin→WIT→host data flow | Host rendering + architecture notes. |
| [`shared-runtime-and-app-size.md`](shared-runtime-and-app-size.md) | Anyone asking "why are apps so big?" / "can we share Compose across apps?" | Why each app bundles its whole framework, why you can't publish a framework as a linked shared component, and the framework-base-zygote path to sharing a runtime in RAM. |

## Contracts & rendering — the portable ABI

The OS-agnostic layer: the WIT the guest imports, and how it maps to Skia.

| Doc | What it covers |
|---|---|
| [`skia-wit-mapping.md`](skia-wit-mapping.md) | Skia ↔ `my:skiko-gfx` mapping — the canonical rendering contract. |
| [`ui-shell-consolidation.md`](ui-shell-consolidation.md) | `wandr:ui-shell` + the consolidation event — retiring my:skiko-gfx AND wasi:canvas@0.0.1. |
| [`skiko-gfx-vs-wasi-gfx.md`](skiko-gfx-vs-wasi-gfx.md) | `my:skiko-gfx` vs wasi-gfx/wasi:webgpu — relationship, differences, standardization question. |
| [`surface-convergence-proposal.md`](surface-convergence-proposal.md) | Converging wasi-gfx and the wandr canvas stack — both directions, one vocabulary. |
| [`visual-sizing-design-patterns.md`](visual-sizing-design-patterns.md) | Visual sizing — design patterns, and where wandr differs. |

## OS backends — Android subsystems & portability

The OS-specific layer. Android is the production backend; the last two docs
cover plugging wandr into other OSes.

| Doc | What it covers |
|---|---|
| [`artless-native-service-model.md`](artless-native-service-model.md) | The `--no-art` native-service model — why the bring-up is a mess, and the clean shape. |
| [`sensor-access-conflicts-no-art.md`](sensor-access-conflicts-no-art.md) | Sensor access under `--no-art` — paths, Android cross-check, conflicts. |
| [`device-hal-inventory.md`](device-hal-inventory.md) | Device HAL inventory — HIDL vs AIDL across wandr test devices. |
| [`binder-abi-portability.md`](binder-abi-portability.md) | Binder ABI portability — generating rsbinder bindings across Android versions & devices. |
| [`call-engine.md`](call-engine.md) | Call engine (`wandr-call`) — pure-Rust WebRTC internals. |
| [`wandr-os-portability.md`](wandr-os-portability.md) | **How the runtime plugs into a new OS** — what a backend must provide. |
| [`redox-wandr-feasibility.md`](redox-wandr-feasibility.md) | Redox OS as a wandr host target — feasibility notes. |

## Guest languages & UI-framework feasibility

| Doc | What it covers |
|---|---|
| [`wasm-component-language-support.md`](wasm-component-language-support.md) | Guest-language support for WASI components (language neutrality). |
| [`java-framework-reuse-via-wasm.md`](java-framework-reuse-via-wasm.md) | Reusing Android's Java framework logic as wasm components. |
| [`swift-openswiftui-wandr-feasibility.md`](swift-openswiftui-wandr-feasibility.md) | Swift + OpenSwiftUI on wandr — feasibility memo (SPIKE WORKING). |
| [`avalonia-wandr-feasibility.md`](avalonia-wandr-feasibility.md) | Avalonia on wandr — feasibility memo (SHIPPED). |
| [`egui-wandr-feasibility.md`](egui-wandr-feasibility.md) | egui on wandr — mapping memo (belongs on wasi-webgpu). |
| [`flutter-wandr-feasibility.md`](flutter-wandr-feasibility.md) | Flutter/Dart on wandr — feasibility memo (+ the Go UI question). |
| [`qt-wandr-feasibility.md`](qt-wandr-feasibility.md) | Qt on wandr — feasibility memo (NOT practical). |
| [`ruby-wandr-feasibility.md`](ruby-wandr-feasibility.md) | Ruby on wandr — feasibility memo (viable-but-DIY). |

## Feature designs & proposals

| Doc | What it covers |
|---|---|
| [`audio-player-design.md`](audio-player-design.md) | Feature-rich audio player on wandr — layered capability-negotiated design (task 108). |
| [`wandr-media-scope.md`](wandr-media-scope.md) | `wandr:media` — scope decision (2026-06-12, not designed yet). |

## Bug notes

| Doc | What it covers |
|---|---|
| [`bug-keyguard-ime-overlay-and-audio.md`](bug-keyguard-ime-overlay-and-audio.md) | keyguard/IME overlay corruption + wandr-app audio UI-block + IME audio leak. |
| [`kotlin-wasm-export-exit-pump-bug.md`](kotlin-wasm-export-exit-pump-bug.md) | K/Wasm: `onExportedFunctionExit` fires inside `cabi_realloc`, running user code while canonical-ABI realloc memory is pending. |

## Conventions

- Docs are append-only narrative — when something changes, prefer
  editing the relevant section over deleting it. Keep historical
  rationale where it explains *why* the current code looks the
  way it does.
- Link liberally to `tasks/<N>-*.md` for the historical task
  context, and to `~/wandr/.claude/memory/*.md` via double-bracket
  `[[memory-slug]]` syntax for design notes that aren't task-scoped.
- Each doc opens with a TL;DR + an ASCII process / boundary
  diagram so the reader can build mental model before reading
  prose.

## See also

- [`~/wandr/CLAUDE.md`](../CLAUDE.md) — master setup, status
  table, repo layout, current task list.
- [`~/wandr/post-art-roadmap.md`](../post-art-roadmap.md) —
  long-term direction (Hybrid §9 runtime model + beyond).
- [`~/wandr/tasks/`](../tasks/) — per-task notes (scoping +
  results, e.g. `45-wandr-zygote-spike.md`, `46-wandr-arbiter-mvp.md`,
  `49-ime-content-control.md`).
