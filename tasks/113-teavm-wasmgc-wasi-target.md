# Task 113 — TeaVM `WEBASSEMBLY_GC_WASI` target (JS-free WasmGC for WASI hosts)

> Scoped 2026-06-16. The build-out of **task 112's Path A**, done cleanly: add a
> **new TeaVM target/mode** that emits a **WasmGC module with no JavaScript
> dependency** — a standard `wasi_snapshot_preview1` floor, no `teavmJso`, no
> `wasm:js-string` — so pure-Java compiles to a module that runs on wandr's
> wasmtime + the existing **P1→P2 adapter → component**. Java→WasmGC+WASI exists
> nowhere today; this fills that exact gap. See `tasks/112-java-component-spike.md`
> (the spike that located every coupling point), `docs/java-framework-reuse-via-wasm.md`.

## Why a new target, not a patch

TeaVM's existing `WEBASSEMBLY_GC` target is **browser-oriented**: it installs the
JSO plugin and routes `Throwable`/`Object` through JS (`JSString`/`TimerHandler`),
so the module needs a JS engine. **Don't mutate it** (would break the browser use
case). Instead add a sibling — exactly as TeaVM's **C backend is already "WasmGC's
non-JS sibling"** (char-array `String`, no JSO, WASI via `tests.wasi`). Additive,
non-breaking, and **upstreamable** (a genuinely new capability TeaVM lacks).

Key fact from 112: **`java.lang.String` needs NO work** — on WasmGC it's already
char-array-backed (`getPlatformTags()` = `{WEBASSEMBLY_GC}` → `isJavaScript()` is
false → `TString` uses `fastCharArray()`, not `substringJS`). The JS imports come
from JSO + `Throwable`/`Object`, not String.

## Work breakdown (the four located coupling points + plumbing)

1. **Target registration / mode plumbing.** Add `WEBASSEMBLY_GC_WASI` to
   `TeaVMTargetType` (or a `hostType: JS|WASI` flag on `WasmGCTarget`), thread it
   through the maven plugin's `targetType` param → constructs `WasmGCTarget` in WASI
   mode. (Confirm how `TeaVMCompileMojo` builds the target from the enum.)
2. **Gate the JSO plugin off in WASI mode.** `JSOPlugin.install` does
   `if (wasmGCHost != null) WasmGCJso.install(...)`. In WASI mode, don't expose the
   WasmGC-JSO extension / have `WasmGCJso.install` no-op. → removes `teavmJso.*`, the
   `WasmGCJsoCommonGenerator` module initializer, and `wasm:js-string`.
3. **`Throwable` stack-trace, non-JS** (`classlib/.../java/lang/TThrowable.java:121,
   134,160`). Replace `JSString.valueOf(getClass().getName())` + `takeWasmGCStack(
   JSObject)` + `initNativeExceptionJS(JSObject)` with a non-JS WasmGC capture (or a
   minimal/empty trace to start). Model on the C backend's non-JS stack path.
4. **`Object` timer** (`TObject.java:32` `TimerHandler`). Provide a non-JS path /
   stub — `wait`/`notify` on a single-thread WASI guest are trivial.
5. **Floor → WASI** (the mechanical redirects from 112): `WasmGCSupport.putchar
   Stdout/Stderr` → `wasi_snapshot_preview1.fd_write`; `SystemIntrinsic.current
   TimeMillis` → `clock_time_get`; `WasmGCTarget` heap globals (`heapOffset`/
   `maxSize`/`notifyHeapResized`) → module-internal/self-managed (crib the C
   backend's standalone heap).

## Acceptance test (already built)

Re-run `repros/java-wasm-spike/stub-host` (the wasmtime harness from 112) after each
step:
- After steps 1–4: `wasm-tools dump spike.wasm` shows **zero `teavmJso` / `wasm:
  js-string` imports** (only the floor + `env`).
- After step 5: the module's only imports are `wasi_snapshot_preview1.*` (+ self-
  managed memory); it **instantiates + runs `main` under wasmtime** (prints
  `spike packedLen: …`) with no trapping stub fired.
- Then: pipe through the **P1→P2 adapter** (`wasm-tools component embed` + `component
  new --adapt`) → a component; **export a custom WIT** (e.g. `wandr:sms-pdu`) via
  hand-written canonical-ABI (skiko pattern); instantiate + call from wandr-host on
  desktop, then device. That closes task 112's M1–M3 on the clean target.

## Milestones

- **M1 — target mode wiring** (`WEBASSEMBLY_GC_WASI` selectable from maven; builds,
  even if still JS-coupled). Gate.
- **M2 — JSO gated off** (step 2): module has no `teavmJso`; but `Throwable`/`Object`
  may still fail to compile/analyze → drives M3/M4.
- **M3 — `Throwable` + `Object` de-JSO'd** (steps 3–4): pure-Java compiles with
  **zero `teavmJso`/`js-string`** imports.
- **M4 — floor → WASI** (step 5): only `wasi_snapshot_preview1` imports; **runs
  under wasmtime** (harness prints, no traps).
- **M5 — component + custom WIT** (P1→P2 adapter + hand-ABI export) instantiated +
  called from wandr-host, desktop → device. **= task 112 PASS on the clean target.**

## Risks / unknowns

- **TeaVM internals depth** — `core` (~154K LOC) + `classlib` (~176K LOC); the patch
  is concentrated (4 coupling points + target plumbing) but in an unfamiliar,
  actively-developed compiler. Keep rebased on upstream.
- **JSO-gate ripple** — some classlib code may assume the WasmGC-JSO extension is
  present; M2 will surface any (each likely has a C-backend non-JS precedent).
- **Heap self-management** — getting `heapOffset`/grow right without the JS loader
  (the C backend already does standalone heap — crib it).
- **Upstream stance** — worth a TeaVM issue early; if `konsoletyper` is receptive, a
  contributed `WEBASSEMBLY_GC_WASI` target beats a private fork.

## Status

🔲 Scoped 2026-06-16, not started — the funded build-out of task 112's Path A.
Acceptance harness (`repros/java-wasm-spike/stub-host`) + the `spike.wasm` build
(`repros/java-wasm-spike`) already exist. Effort: a real TeaVM fork, bounded to the
five items above.
