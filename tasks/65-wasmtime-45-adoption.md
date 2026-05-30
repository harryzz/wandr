# Task 65 — Adopt wasmtime 45.0.0 (DRC auto-GC: leak bounded, ANR gone)

✅ **Device-verified 2026-05-30.** Bumps the host runtime `wasmtime`/`wasmtime-wasi`
44 → 45.0.0, which carries the DRC auto-GC fix for our long-standing
Kotlin/Wasm continuation leak (task 24, [[wasmtime-drc-no-autoschedule]]).

## Why now

May-21 we tested fitzgen's [#13422](https://github.com/bytecodealliance/wasmtime/pull/13422)
(auto-GC trigger when the over-approx-stack-roots list doubles — "Fixes #13403",
our issue) cherry-picked onto wasmtime 44. It **bounded the leak but reintroduced
an ANR**: the inline `force_gc` fired constantly on the 60 fps Compose guest and
each GC's `trace_vmctx_roots` blocked the render thread. We reverted.

Three things changed since:
1. **45.0.0 carries #13422 + #13450** — the `array.copy` stack-map correctness
   fix that #13422 *required* (a GC mid-copy could free arrays). We ran #13422
   *without* it in May.
2. **45.0.0 improves the grow-vs-collect heuristic (#12942)** + DRC tracing-memory
   (#13192).
3. **Task 64 (on-demand rendering)** caps the Compose guest at ~1 fps idle instead
   of 60, so the auto-GC fires far less in normal use — directly attacking the
   GC-frequency × root-scan-cost product that caused the ANR.

## Test campaign (all passed)

| Test | wasmtime 44 | **45.0.0** |
|---|---|---|
| Desktop `wart-leak-repro` (`wasmtime run -Wgc,… -Ccollector=drc`), max churn | RSS 71→568 MB+ climbing (→ 4 GB ceiling) | **flat 39 MB** |
| Host `cargo` bump 44→45 | — | **zero API breaks** |
| Device idle wart-app | — | flat ~253 MB, ~7% CPU (no regression) |
| Device 60 fps + active scrolling, 3 min (frame-pacing forced to 0) | unbounded | **RSS flat ~220 MB, no ANR, smooth (minor glitches)** |

The May-21 render-thread-blocking ANR is gone. There is **no active `Store::gc`
mitigation to remove** — the deferred-gc trigger was always `#[cfg(feature =
"profile")]`-gated; production simply leaked on 44 and is now bounded on 45.

## Changes

- `runtime/wart-host/Cargo.toml` + `Cargo.lock`: `wasmtime`/`wasmtime-wasi` 44 → 45.
- `runtime/wart-host/src/app_loader.rs`: **dependency-cwasm self-heal** — the
  blocker found during this work. `load_installed` re-precompiles the app's own
  component on engine drift, but `load_dep_components` only `deserialize_file`d
  the dep cwasms, so a runtime bump left them AOT'd by an incompatible engine →
  `deserialize_file` fails "incompatible version '44'" → consumer drops to the
  test-frame fallback (blank screen + corner rectangle). New
  `deserialize_dep_or_reprecompile` recovers like the app does: on any
  `deserialize_file` failure, re-precompile from the dep's source `.wasm`,
  overwrite the cwasm, retry. Verified on device (corrupted a dep cwasm → loader
  logged `re-precompiling from source wasm` → recovered → real scene rendered).
  The preload path already skipped-and-logged on drift (`preload.rs`), so this
  completes the intended lazy re-precompile.

## Adoption notes / follow-ups

- **Upgrading the runtime re-AOTs every cwasm.** The installer/loader cache-key
  includes `wasmtime_version`, so apps self-heal on first load — now including
  deps. (Before this fix: reinstall all apps after a bump.)
- **Adapter fork rebased onto 45.0.0** (`external/wasmtime`, branch
  `kt-86415-option-b`): the single KT-86415 State-pin commit replayed cleanly onto
  the upstream `v45.0.0` tag (no conflict — it only touches the adapter `lib.rs`),
  giving `377cd917af Release Wasmtime 45.0.0` + `1b99ec60e2 …pin State…`. The
  **State-pin is still required** — it fixes a guest-side use-after-free between
  the WASI-P1 adapter `State` and Kotlin's `freeAllComponentModelReallocAllocatedMemory`
  (KT-86415), orthogonal to wasmtime's host-runtime GC; stays until KT-86415 lands
  in the Kotlin stdlib. Adapter rebuilt + a re-adapted wart-app validates. The
  rebase **rewrote the fork's history**, so publishing it needs a force-push to
  `codeberg/harryzz/wasmtime` (held for explicit go-ahead). Device guests keep
  their existing 44-built adapter (ABI version-independent, runs under host 45);
  the 45-based adapter applies to future guest builds.
- CLAUDE.md's task-24 row still reads "reverted to 44" — update when convenient.
