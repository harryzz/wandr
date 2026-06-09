# Task 37 — diagnose the Kotlin/Wasm + WASI command-adapter init throw

> **Status:** 🔲 scoped — created 2026-05-26 as a follow-up spin-out of
> task 36 step 7. Not blocking any current direction; pick up when a
> Kotlin CLI consumer becomes important (or for the cleanliness of
> closing a known orthogonal bug).
>
> **Not blocking visuals.** Compose consumers use the *reactor* adapter,
> which is known-good. This bug is specific to the *command* adapter,
> which only matters for one-shot `wasi:cli/run`-shaped guests. The
> Rust smoke (`md-smoke-rust/`) sidesteps it cleanly; cross-app dep
> wiring is fully proven without it.

## What we know

Confirmed on Pixel 2 XL 2026-05-26 via `wandr-host --run-once`:

- A Kotlin/Wasm 2.4.0-RC component, adapted with
  `wasi_snapshot_preview1.command.wasm` (the *command* variant), throws
  "thrown Wasm exception" from `wasi:cli/run.call_run(store)` —
  **regardless of guest body**.
- `fun main() { val x = render("..."); ... }` — throws.
- `fun main() {}` (empty body) — throws identically.
- The throw fires *before* `main()` runs (no log output reaches the
  guest's first instruction), so it's at module init / `__wasm_call_ctors`
  / Kotlin's static-initializer chain or in the adapter's `_start` shim.
- Behavior is identical under:
  - `wasmtime run` on dev box (logged in
    [[kotlin-wasm-println-throws-under-wasmtime]])
  - `wandr-host --run-once` on device with `wasi_stderr` routing fd 2
    to logcat — same throw, no stderr output before the trap.
- The *reactor* adapter has different init semantics (`_initialize`
  only, no `_start`) and does NOT trip this — wandr-app uses it daily
  with no issues.
- The wasmtime trap message is opaque: "thrown Wasm exception". The
  actual wasm-exception payload isn't surfaced; would need a patched
  wasmtime to capture it.

## What we don't know (the investigation surface)

1. **Where exactly in the wasm execution does the throw fire?** Module
   init (`__wasm_call_ctors`), Kotlin's static initializers, the
   command adapter's `_start` shim, or the wasi:cli/run.run entry?
2. **What's the wasm exception payload?** Kotlin/Wasm uses
   wasm-exceptions for `throw`; the payload should be the Kotlin
   `Throwable` struct. Reading it requires
   `Store::take_pending_exception` + walking the structref fields (the
   wandr-app code in `lib.rs:354-403` shows how — same recipe applies
   here, just from `run_once.rs`).
3. **Is it a Kotlin codegen bug or a command-adapter interaction?**
   Possibilities:
   - Kotlin's static init calls `stdio.write` to fd 1 or 2 via WASI;
     the command adapter's stdout/stderr setup behaves differently from
     reactor.
   - Kotlin's runtime probes a clock / random / environment WASI
     interface that the command adapter doesn't satisfy the way Kotlin
     expects.
   - The command adapter's `_start` shim itself throws (less likely —
     it's well-tested in Rust ecosystem).
   - Kotlin/Wasm regression: maybe earlier Kotlin versions don't trip
     this; could bisect.

## Investigation plan (when picked up)

Five steps, ordered to maximize learning per hour:

| # | What | Tooling |
|---|---|---|
| 1 | **Capture the exception payload from `run_once.rs`** — add the same `Store::take_pending_exception` + Throwable-struct walk that `lib.rs:354-403` does on the render-frame error path. Log message + cause from the Kotlin Throwable. Likely surfaces a clear root cause. | Rust edit only; ~30 min |
| 2 | **wasm-tools dump** the composed `md-smoke-component.wasm`'s start function and module-init chain — see what runs before `main()`. | `wasm-tools print --skeleton` + grep for `(start` + `__wasm_call_ctors` |
| 3 | **Bisect Kotlin versions** — try 2.3.x → does the bug exist there? If only 2.4.0-RC, it's a regression worth filing on YouTrack. | rebuild wandr-app-md-smoke against older `kotlin("multiplatform") version` |
| 4 | **Bisect adapter versions** — try older `wasi_snapshot_preview1.command.wasm` builds. wandr's wasmtime-src fork is at a specific commit; pre-fork versions of the adapter may or may not trip this. | rebuild command adapter from upstream wasmtime tags |
| 5 | **File upstream** — if it's reproducible with stock Kotlin + stock adapter, this is a JetBrains / wasmtime issue worth tracking. Reference the wandr-leak-repro pattern (task 24/25) — a minimal Kotlin file that throws under `wasmtime run`. | YouTrack issue + a minimal repro tarball |

Most likely outcome: step 1 surfaces a Kotlin `Throwable` with a
human-readable message that names the failing operation (e.g. "stderr
not connected" or "expected fd 0 stdin pollable"). That probably
identifies the root cause and points at either a Kotlin/Wasm fix or a
workaround (e.g. pre-creating specific WASI handles before `call_run`).

## How to apply once fixed

- `wandr-app-md-smoke/` Kotlin consumer's `main()` will run cleanly →
  full Kotlin-side cross-app-dep validation (currently only
  Rust-side, via `md-smoke-rust/`).
- Future Kotlin CLI consumers (build tools, codegen, single-shot
  utilities) become viable.
- The orthogonality marker in
  [[task-36-step-7-pending]] /
  [[rust-component-as-cli-smoke]] /
  [[kotlin-wasm-println-throws-under-wasmtime]] can be downgraded /
  removed.

## Out of scope

- Restructuring wandr-host to avoid command-adapter consumers (we already
  support both; `--standalone` uses reactor for Compose, `--run-once`
  uses command for CLI).
- A general fix for any guest-language WASI command-adapter issue —
  this task only targets the Kotlin/Wasm + command-adapter pair.
- Adding more Kotlin-side WASI plumbing in wandr-host as a workaround
  — the right fix is in Kotlin/Wasm's codegen or the command-adapter's
  shim, not in wandr-host.

## Related

- `tasks/36-cross-app-deps.md` (step 7 surfaced this on-device).
- `wandr-app-md-smoke/` — the blocked Kotlin consumer.
- `md-smoke-rust/` — the Rust workaround that sidesteps it.
- `docs/architecture-host-guest-boundary.md` — adapter-shape framing.
- `wandr-leak-repro/` — pattern reference for a minimal Kotlin/Wasm
  repro tarball.
- Memories: [[kotlin-wasm-println-throws-under-wasmtime]],
  [[task-36-step-7-pending]],
  [[rust-component-as-cli-smoke]].
