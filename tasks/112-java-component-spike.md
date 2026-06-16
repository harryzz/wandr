# Task 112 — Java → Component-Model spike (go/no-go)

> Scoped 2026-06-16. The **cheapest possible answer to "is the AOSP-Java-as-wasm
> dream real?"** — a time-boxed spike, NOT a commitment to revive the whole
> toolchain. Decision driver: with the runtime PoC rich, the highest-value next
> direction is *reach* (run the existing Java world), not *substrate* (Redox).
> See `docs/java-framework-reuse-via-wasm.md` (the thesis + Vision-vs-achievable),
> `docs/wasm-component-language-support.md` (JVM→wasm toolchain status, currently
> dormant), `tasks/111-native-iradio-client.md` (the eventual marquee target).

## Goal (one sentence)

Get **one shallow-native, self-contained pure-Java unit** through a Java → core
wasm module → wandr's **P1→P2 adapter** → a **component that exports a custom WIT**,
**instantiated + called from wandr-host** (desktop dev loop, then device) — by
**assembling the pipeline from whatever existing tools fit and writing the missing
glue** where they don't. Prove the *toolchain is buildable* end-to-end; defer
framework entanglement.

## Nature of the task: investigate → integrate → build

There is **no off-the-shelf Java→Component-Model toolchain** (confirmed:
`docs/wasm-component-language-support.md`). So this is **not** "adopt a tool" — it's
**R&D**: survey the available pieces, **stitch the usable ones together**, and
**write the parts that are missing**, time-boxed per part. Three phases:

1. **Investigate** — establish, hands-on, what each candidate actually produces and
   where it stops (see the inventory below). Cheap, first.
2. **Integrate** — wire the viable pieces into one path to a wandr component
   (compiler → core module → P1→P2 adapter → custom-WIT export → instantiate).
3. **Build the missing parts** — only what the integration proves is absent (e.g. a
   binding shim, a WASI-host shim for JS-env imports, a revived bindgen). Each
   missing part is its own bounded go/no-go; if one is too deep, that's the wall.

### Why this exact shape

- **Isolate the variable.** Tests the **toolchain** (compile + adapter + custom-WIT
  export on wandr), NOT framework entanglement (Binder / Looper / deep JNI). So the
  target must be **pure Java with zero `android.*`, zero native, zero Binder, zero
  Looper** — every obstacle from the design note removed except "can Java become a
  wandr component at all."
- **Reuse what exists.** wandr already wraps Kotlin/Wasm core modules into reactor
  components via the P1→P2 adapter with **hand-written canonical-ABI bindings** (the
  skiko binding). Do the same for the Java module — so a missing
  `wit-bindgen-teavm-java` generator is **not** a blocker (hand-write the ABI).
- **Bounded + go/no-go per part.** Days, not months. A hard wall is a *useful*
  result.

## Toolchain inventory (mix-and-match)

| Piece | Provides | Gap / status |
|---|---|---|
| **TeaVM (upstream `konsoletyper`)** | Java → core wasm; **linear-mem backend → WASI** + a separate WasmGC→JS backend; actively maintained | the **WASI** path is the linear-mem one; component wiring lapsed |
| **teavm-wasi forks (golem/fermyon)** | the WASI + CM glue + `wit-bindgen-teavm-java` | **dormant** (2022–23); revive/port onto live upstream TeaVM |
| **J2CL / J2WASM (Google + Chrome)** | Java → **WasmGC** (same shape as wandr's Kotlin path → reuses the adapter) ; actively maintained (commit 2026-06) | **JS-host only (jsinterop), NO WASI/standalone** — same gate as dart2wasm. Usable only if a standalone mode lands *or* we shim its JS env as WASI host fns (large) → **watch / preferred-if-standalone** |
| **wandr P1→P2 adapter** | core module → reactor component (KT-86415 State-pin, fixed linear-mem partition) | tuned for Kotlin/Wasm; **verify** it wraps a TeaVM module (its own check) |
| **hand-written canonical-ABI bindings** | custom-WIT import/export without a generator (skiko precedent) | manual; fine for the spike's one interface |
| **AIDL2WIT** (user's project) | Binder → WIT (for the *later* entangled-service phase) | not needed for the pure-Java spike |
| **Looper-on-step-executor shim** | Handler/Looper (for the *later* phase) | not needed for the pure-Java spike |

**Working hypothesis to test first:** **TeaVM linear-mem → WASI core module → wandr
P1→P2 adapter → hand-written ABI** is the only path that targets the right host
today; J2WASM (WasmGC, maintained) is the better *future* base but JS-locked. The
investigate phase confirms or refutes this before any integration effort.

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

## Investigate findings (2026-06-16)

Hands-on, ground-truth (built + inspected a real module — `repros/java-wasm-spike/`):

- **Environment ready:** JDK 21, Maven 3.9.9, `wasm-tools` 1.245.1 + `wasmtime` 45
  (wandr's exact versions), Maven Central reachable.
- **TeaVM latest = 0.15.0; target enum is now `{ JAVASCRIPT, WEBASSEMBLY_GC, C }`**
  — the old **linear-memory `WEBASSEMBLY` backend was removed upstream**. So
  "revive the dormant linear-mem teavm-wasi fork on live upstream" is *not* viable
  as-is; its backend no longer exists.
- **Built `spike.wasm`** from trivial pure Java (`teavm-maven-plugin` `compile`,
  `targetType=WEBASSEMBLY_GC`) — valid WasmGC, 36 KB.
- **Its imports prove WasmGC is JS-host-targeted, not WASI:** `wasm:js-string`
  (JS-string builtins) + `teavmJso.*` (JS interop) + a small named runtime
  contract: `teavmConsole.putcharStdout`, `teavmDate.currentTimeMillis`,
  `teavmMemory.{heapOffset,maxSize,notifyHeapResized}`, `teavm.{takeStackTrace,
  decorateException}`. **No `wasi_snapshot_preview1`.** (Same shape as J2WASM.)
- **…but the contract is shallow + named, and a WASI host for it already exists:**
  TeaVM's upstream **`-Pteavm.tests.wasi=true`** mode runs WasmGC under a non-JS
  host → there is a reference implementation of the `teavm*` import contract to
  study. `teavmConsole.putcharStdout` ↔ WASI `fd_write` is the tell.

### Revised integration picture (the fork for the integrate phase)

The hypothesis "TeaVM → WASI core module → P1→P2 adapter" is **wrong in detail**:
TeaVM doesn't emit a WASI module; it emits a **WasmGC module with a custom `teavm*`
import contract**. Two real paths:

- **Path A — WasmGC + custom host imports (no WASI/adapter).** wandr's wasmtime
  instantiates the WasmGC module directly, providing the ~6-8 `teavm*` funcs (+
  js-string builtins, or disable them) as host functions; component/custom-WIT
  wrapping is done by embedding the core module + hand-wiring exports (NOT the
  P1→P2 adapter, which is WASI-preview1-shaped). **Pro:** WasmGC (wandr's Kotlin
  shape), maintained, small shim. **Con:** not the existing component path; js-string
  builtins need wasmtime support or a shim.
- **Path B — TeaVM `C` backend → wasi-sdk → WASI core module → P1→P2 adapter.** The
  classic linear-memory WASI path, fits wandr's *existing* component flow. **Pro:**
  reuses the adapter; clean WASI. **Con:** Java→C→wasm (wasi-sdk + clang setup),
  no WasmGC, heavier; untested here.

**Next (integrate phase): evaluate Path A first** — study TeaVM's `tests.wasi` host
impl, provide the `teavm*` contract from wandr-host, instantiate `spike.wasm`, call
an export. Fall back to Path B if the WasmGC+js-string host surface proves deep.
J2WASM stays a watch-item (WasmGC + maintained, but same JS-host gap, no `teavm*`
test-host to crib).
