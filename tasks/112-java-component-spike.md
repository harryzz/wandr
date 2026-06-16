# Task 112 — Java → Component-Model spike (go/no-go)

> Scoped 2026-06-16. The **cheapest possible answer to "is the AOSP-Java-as-wasm
> dream real?"** — a time-boxed spike, NOT a commitment to revive the whole
> toolchain. Decision driver: with the runtime PoC rich, the highest-value next
> direction is *reach* (run the existing Java world), not *substrate* (Redox).
> See `docs/java-framework-reuse-via-wasm.md` (the thesis + Vision-vs-achievable),
> `docs/wasm-component-language-support.md` (JVM→wasm toolchain status, currently
> dormant), `tasks/111-native-iradio-client.md` (the eventual marquee target).

## Goal (one sentence)

Get **one shallow-native, self-contained pure-Java unit** compiled with TeaVM →
core wasm module → wandr's existing **P1→P2 adapter** → a **component that exports
a custom WIT**, and **instantiate + call it from wandr-host** (desktop dev loop,
then on device). Prove the *toolchain* end-to-end; defer framework entanglement.

## Why this exact shape

- **Isolate the variable.** The spike tests the **toolchain** (TeaVM + adapter +
  custom-WIT export on wandr), NOT the framework-entanglement question (Binder /
  Looper / deep JNI). So the target must be **pure Java with zero `android.*`,
  zero native, zero Binder, zero Looper** — every obstacle from the design note
  removed except "can Java become a wandr component at all."
- **Reuse what exists.** wandr already wraps Kotlin/Wasm core modules into reactor
  components via the P1→P2 adapter, with **hand-written canonical-ABI bindings**
  (the skiko binding). The spike does the same for a TeaVM core module — so a
  missing `wit-bindgen-teavm-java` generator is **not** a blocker (hand-write the
  ABI; the removed generator is a later convenience, not a gate).
- **Bounded + go/no-go.** Days, not months. A hard wall is a *useful* result.

## Target unit

Pick the cleanest non-trivial pure-Java logic — recommended order:

1. **GSM 7-bit / UCS-2 SMS PDU encode/decode** (hand-written as plain Java, no
   `android.*`) — real, non-trivial bit-twiddling, and **on the path to telephony**
   (reused by the future RILJ work). Best signal-to-effort.
2. Fallback if (1) drags: **`org.json`** parse/serialize (well-known
   TeaVM-compatible pure Java) — proves the toolchain with near-zero target risk.

Deliberately **not** an AOSP framework class yet (those carry the entanglement the
spike is trying to exclude).

## Milestones (each a go/no-go gate)

- **M0 — TeaVM core module.** Stand up a TeaVM build (upstream `konsoletyper`
  linear-memory backend, or revive the dormant `teavm-wasi` fork) that compiles the
  target unit to a **core wasm module** exporting named functions. Gate: a `.wasm`
  exists and runs a trivial export under `wasmtime`/`wasm-tools`.
  - **Kill if:** TeaVM cannot emit a usable WASI core module with named exports
    within the box → Java→CM is not near-term viable; stop.
- **M1 — wrap into a component (desktop).** Run the core module through wandr's
  P1→P2 adapter (`wasm-tools component embed` + `component new --adapt`) → a
  reactor component; instantiate in `wandr-host` on the **desktop dev loop**; call
  one exported function via hand-written canonical-ABI bindings; verify output.
  - **Kill if:** the adapter can't wrap a TeaVM module (ABI mismatch the Kotlin
    fork's State-pin doesn't cover, unrecoverable) → stop, document.
- **M2 — custom WIT export.** Define a real `wit` for the unit (e.g.
  `wandr:sms-pdu` `encode`/`decode`) and make the component **export** it
  (hand-written ABI on both sides); a Rust/host test drives it through the WIT.
  Gate: the WIT round-trips real data (encode→decode identity).
- **M3 — on device.** Same component instantiated + called by wandr on the Pixel.
  Gate: identical result on-device. (No UI needed — a `--run-once`/headless call
  is enough.)

## Decision at the end

- **PASS (M3 green):** Java→component is real on wandr. Spin up the follow-on:
  (a) the **Looper-on-step-executor** shim + **AIDL2WIT** Binder bridge, then
  (b) **RILJ as the first entangled service** (the task-111 alternative). The
  liftability-pipeline idea becomes a real workstream.
- **FAIL (any kill):** record the precise wall in `docs/java-framework-reuse-via-
  wasm.md`; Java→CM parked; revisit only if the toolchain landscape changes.
  Either way the time spent is small and the answer is definitive.

## Known unknowns / watch-items

- **TeaVM export mechanism** — confirm TeaVM can export arbitrary named wasm
  functions (the analog of Kotlin's `@WasmExport`) that the adapter/ABI can target.
- **TeaVM Java coverage** of the target unit (keep it simple to avoid reflection /
  dynamic-class-loading walls).
- **Adapter compatibility** — the wandr P1→P2 adapter was tuned for Kotlin/Wasm
  (KT-86415 State-pin, fixed linear-memory partition); a TeaVM module's memory
  layout may need its own check (this is itself useful signal).
- **WasmGC vs linear-memory** — the spike uses linear-memory TeaVM (the WASI path);
  WasmGC TeaVM→WASI doesn't exist (see `docs/wasm-component-language-support.md`).

## Status

🔲 Scoped 2026-06-16, not started. Time-boxed go/no-go; start at M0 (target unit:
GSM SMS PDU codec, fallback `org.json`).
