---
name: reference_host_aot_codegen_corruption
description: "Host AOT (precompile_component) emitting bad code after a rebuild → freshly-installed guests SIGSEGV \"execute non-executable memory\"; + the ps|grep guest-liveness trap that made it hard to diagnose."
metadata: 
  node_type: memory
  type: reference
  originSessionId: a2edab94-9d77-4289-807e-6fabf67af25c
---

# Host AOT codegen corruption — freshly-AOT'd guests crash (2026-06-10, task 90 M4)

**Symptom.** After rebuilding `wandr-host`, every guest the host **freshly AOT-compiles
at `--install`** (via `wasmtime::Engine::precompile_component` → Cranelift) crashes
instantly on launch:
```
F libc: Fatal signal 11 (SIGSEGV), code 2 (SEGV_ACCERR)
F DEBUG: Cause: trying to execute non-executable memory.
        pc → [anon:scudo:primary]   (a heap addr, pc==lr, 1 frame)
```
The **already-running stack is unaffected** (it runs cwasm compiled by an *earlier*
host; same wasmtime compatibility hash → never re-precompiled). So only NEW installs
break. Reproduced across unrelated guests (settings.wifi, dioxus.demo, taskmanager) →
it's the host's compiler, not guest code.

**Cause (high confidence).** A corrupted **incremental-build artifact in the Cranelift
codegen path** (regalloc2 / cranelift-codegen / wasmtime-internal-cranelift). Tell: the
prior *fuller* host build produced working cwasm; the next *incremental* rebuild (on top
of the same target dir, trivial unrelated source change) produced broken cwasm — from
source that cannot affect codegen. "Jump to non-executable heap" is the classic
bad-register-allocation / corrupted-codegen signature.

**Fix.** Clean the codegen crates + rebuild (no source change):
```
cargo clean -p wasmtime -p wasmtime-environ -p wasmtime-internal-cranelift \
  -p cranelift-codegen -p cranelift-frontend -p cranelift-native -p regalloc2 \
  --target aarch64-linux-android --release
cargo build --target aarch64-linux-android --release
```
(`clean -p wasm-android-host` ALONE is NOT enough — it doesn't recompile the dep
codegen crates. A full `cargo clean` also works but rebuilds skia too, ~15-20 min.)

**‼️ DETECTION TRAP that wasted hours — `ps | grep <app-id>` NEVER matches a guest.**
Every guest process's `comm` is `wandr-host` (zygote-forked), so
`ps -A | grep settings.wifi` always returns nothing → reads as "CRASHED" even when the
app is alive and rendering. This produced false negatives that made a *fixed* host look
broken. Compounded by (a) **stale arbiter entries** — `arbiter launch` re-foregrounds a
dead tracked pid instead of forking fresh; (b) **stale `logcat` tail** — `grep
on_child_exit...signal | tail -1` returns an OLD crash, not the pid under test.

**Verify guest liveness the RIGHT way:**
- `adb shell '[ -d /proc/<pid> ] && echo ALIVE'` (the pid from `arbiter list`).
- The arbiter's own log: `on_child_exit pid=<P> app=<id> (signal=11)` is a REAL crash;
  match the *exact* pid you just launched.
- A **screenshot** (`screencap`) is ground truth — if it renders, it's alive.
- Clear stale state first: `arbiter task-kill <id>` before a fresh `launch`.

Related: this is the `app-installer-triage` agent's domain (precompile_component /
deserialize_file cache-key / AOT-cache). See [[reference_wandr_apps_root_install]].
