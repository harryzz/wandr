---
name: task-36-step-7-pending
description: Task 36 cross-app deps step 7 — device-verified 2026-05-26; full runtime call-through-proxy chain validated on Pixel 2 XL via Rust smoke consumer
metadata: 
  node_type: memory
  type: project
  originSessionId: d9451151-9116-4c95-a45d-8758673104ce
---

Task 36 step 7 — device-verified 2026-05-26. Full cross-app dep chain runtime-validated end-to-end on Pixel 2 XL.

**Final result (logcat):**

```
loader: cache fresh for com.example.md-smoke-rust 0.0.1
loader: loaded dep `markdown` (wandr:markdown/renderer@0.1.0) from
  /data/.../system-apps/wandr.markdown.renderer/0.1.0/cache/renderer.cwasm
loader: dep `markdown` instantiated; wiring wandr:markdown/renderer@0.1.0
  → consumer linker
run_once: command instantiated — calling wasi:cli/run.run()
md-smoke-rust: render() returned [N block(s)]
run_once: call_run returned Ok — guest exited cleanly
```

EXIT=0. Consumer `main()` called `render()` through `wire_markdown_dep`'s linker closure → dep instance's `call_render` → returned non-empty Document → `exit(0)`.

**What landed this session:**

1. `wandr-host/src/run_once.rs` — new entry path for `wasi:cli/command` consumers; mirrors `standalone.rs` setup, swaps render-loop for one-shot `Command::instantiate` + `call_run`.
2. `LoadedApp::instantiate_command` in `app_loader.rs` — adds `Command::instantiate` alongside the existing `SkikoUi::instantiate`. `wire_dep_into_linker` runs unchanged (consumer-shape-agnostic).
3. `wandr-host --run-once <app-id>` CLI in `main.rs`, adjacent to `--install` / `--standalone`.
4. `md-smoke-rust/` — new Rust `wasm32-wasip2` smoke consumer. Created because the existing Kotlin smoke (`wandr-app-md-smoke/`) hits a pre-existing Kotlin/Wasm + WASI-command-adapter throw at module init that's unconditional on-device (wandr-host's `wasi_stderr` doesn't help — bug isn't stderr-related). Rust on wasm32-wasip2 produces a clean wasi:cli/command shape with no such bug. Pattern is reusable for any future Rust CLI consumer.
5. `scripts/smoke-markdown.sh` — full device pipeline (build → install both packages → `--run-once` → grep logcat).
6. `docs/architecture-host-guest-boundary.md` — captures the host-driven cardinality-1 framing (one-shot CLI is the same primitive as `renderFrame`, just N=1 instead of N=60×/sec). Built in response to a session question on what `renderFrame(nanos)` actually is and whether it's inlined.

**What's still deferred:**

- `HostState.renderer: Option<SkiaRenderer>` cleanup — `--run-once` builds a real SF surface (~1s screen flash) to avoid the ~222-site refactor across `canvas_impl.rs` + `paragraph_impl.rs`. Revisit if more CLI shapes appear.
- Kotlin/Wasm + WASI-command-adapter module-init throw bug — see [[kotlin-wasm-println-throws-under-wasmtime]] (now confirmed orthogonal to dep wiring + unaffected by stderr routing). Separate investigation if Kotlin CLI consumers become important.
- Separate-Store composition mode — markdown driver is same-Store; wait for a service-shaped dep before building this.
- Q5b (signing) from `post-art-roadmap.md` §9 — still open at the broader packaging level.

**Why both Kotlin + Rust smokes exist:**

- Kotlin smoke proves the install/load/linker layer end-to-end through `Command::instantiate` (last log line: `command instantiated — calling wasi:cli/run.run()`). Then trips the Kotlin/Wasm bug — orthogonal to dep wiring.
- Rust smoke completes the chain: `call_run` succeeds, consumer calls `render()` through the proxy, gets a real Document, exits 0.

Together they prove both halves: dep wiring is consumer-shape-agnostic (works for both); only the consumer's own guest runtime (Kotlin vs Rust) determines whether `main()` can complete.

**Related:** [[rust-component-as-cli-smoke]] (the reusable pattern from this session), [[kotlin-wasm-println-throws-under-wasmtime]], [[task-36-cross-app-deps]], [[project-app-lifecycle-and-packaging]], `tasks/36-cross-app-deps.md`, `docs/architecture-host-guest-boundary.md`.
