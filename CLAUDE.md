# WASM Android Runtime — Claude Code Master Guide

> Lean router (slimmed 2026-05-30). Detail lives in `docs/`, `tasks/`, and the
> project memory — read the relevant file on demand (see **Where to look**).

## ‼️ Working rules (BINDING — these override the default action loop)

1. **Read the source/docs FIRST — do not patch-and-cycle.** For any interop /
   protocol / library / external-system task, before the FIRST patch: locate the
   authoritative reference (clone the upstream source into `/tmp` or `repros/`,
   read the official docs, search the project's GitHub issues) and read the
   governing code path end-to-end. Instrument only to *confirm* a specific
   reading, never to fish. **If you are about to make a 2nd device-test / patch on
   the same problem without having read the source path that governs the
   behavior, STOP and go read it.** One source-grounded change beats ten
   speculative ones. (Cost of ignoring this: ~5h cycling on task #16 incoming
   calls; ringrtc source had the answers in minutes. See `[[feedback_read_source_first]]`.)

2. **Do NOT hardcode.** Derive values from first principles / real runtime inputs
   (screen geometry, density, row counts) — not magic numbers; keep it
   resolution/device-independent. A genuinely-needed constant must be ONE named,
   justified source of truth in the layer that owns the policy. See
   `[[feedback_no_hardcoding]]`.

## What this project is

Replace Android's ART runtime with a wasmtime-based host that:
- Runs Kotlin/Compose apps compiled to WASM components
- Renders via skia-safe (Skia C++) on real GPU hardware (EGL direct)
- Targets aarch64 Android (root via ADB, no system modification)
- Uses the WASM Component Model + WIT for all host/guest interfaces

**Working end-to-end on a Pixel 2 XL (Android 15 / API 35).** Real Compose
Multiplatform UIs render at ~10–20 ms/frame; a Hybrid zygote+arbiter runtime,
system launcher/status-bar/taskbar/IME chrome, cross-app deps, and on-demand
rendering are all shipped. Full per-task ledger: **`tasks/STATUS.md`**.

---

## FIRST THING: check the checkpoint file

Before doing ANYTHING else, run:

```bash
cat .task-state 2>/dev/null || echo "NO_STATE"
```

If it exists, read it and **resume from the recorded step** — do not restart
completed tasks. Format:

```
TASK=08
STEP=5a
STATUS=in-progress
LAST_SUCCESS=Task 07 verified OK — app renders on device
NOTES=WIT updated, canvas_impl.rs pending
```

If it doesn't exist, check git log to find where you are, then start at the
lowest incomplete task.

---

## Checkpoint protocol — follow this exactly

**After EVERY successful verify block:**

```bash
cat > .task-state << 'EOF'
TASK=<current task number>
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task <N> verified OK — <one line summary>
NOTES=
EOF
```

**When starting a step inside a task:**

```bash
cat > .task-state << 'EOF'
TASK=<current task number>
STEP=<step number or name>
STATUS=in-progress
LAST_SUCCESS=<copy from previous state>
NOTES=<what was done so far>
EOF
```

**When a step fails:** same block with `STATUS=failed` and `NOTES=<exact error,
first 3 lines>`; then fix using the task's **Known issues** section and set
`STATUS=in-progress` once fixing begins.

**On resume (STATUS=in-progress or failed):** read the file → identify
TASK/STEP/NOTES → check what already exists on disk before redoing anything →
continue from the recorded STEP only.

---

## Where to look (read on demand)

| Working on… | Read first |
|---|---|
| Task status / what's done / history | `tasks/STATUS.md` + `tasks/NN-*.md` |
| Build, WIT-sync rule, cwasm, adapter, deploy, dev env | `docs/build-pipeline.md` |
| Host rendering, `canvas_impl.rs`, skiko files, Kotlin→WIT→host data flow | `docs/host-rendering.md` |
| Runtime architecture (zygote / arbiter / host) | `docs/architecture-runtime.md` |
| What crosses a WIT call (host↔guest boundary) | `docs/architecture-host-guest-boundary.md` |
| IME + language plugins | `docs/architecture-ime.md` |
| Where a new thing lives (repo layout) | `docs/repository-layout.md` |
| dioxus guests (`launch!`, canvas renderer) | the dioxus memories + `crates/dioxus-canvas/` |
| Any gotcha (Kotlin/Wasm, binder, EGL, fonts, leak) | **project memory** — recalled by relevance; `MEMORY.md` is the index |

The longer-term north star is `post-art-roadmap.md`.

---

## Repository layout (brief)

Single git repo (`https://codeberg.org/harryzz/wart`), post-monorepo-merge.
Canonical map: **`docs/repository-layout.md`**. Top level:

```
apps/{system,user}/   wandrpkgs (system chrome + user apps); native = wandr-*, wandrpkg = wandr.*
runtime/              native Rust host stack — wandr-host (wasmtime+skia+EGL), wandr-arbiter
wit/                  canonical WIT (skiko-gfx.wit is the runtime contract)
crates/               shared guest-side Rust libs (dioxus-canvas)
external/             submodules: skiko, wasmtime, compose-multiplatform-core, kotlin, subsecond
tools/scripts/        build-system-wandrpkgs.sh, run-hybrid-stack.sh, build-host-android.sh, …
docs/ tasks/ repros/  architecture docs · task narrative · focused reproducers
.task-state           checkpoint — NEVER delete
.claude/memory/       project memory (this index loads each session)
```

**Naming:** native binaries `wandr-<kebab>`; wandrpkgs `wandr.<dot-id>` matching their
`app_id`; external forks keep their upstream name (no `-src`).

Fresh clone: `git clone --recurse-submodules …` (skip `external/kotlin` unless
rebuilding the stdlib — it's huge).

---

## Agents available

Spawn these to keep build/diagnostic output out of the main context (only when
the user asks or names one):

| Agent | When |
|---|---|
| `cargo-triage` | Rust/Cargo build fails (esp. Android cross-compile) |
| `gradle-triage` | Kotlin/Gradle build fails |
| `wasm-component-build` | Run the full Kotlin → cwasm pipeline + report |
| `libgui-shim-build` | C++ libgui shim build fails (task 33) |
| `rsbinder-triage` | binder runtime failures (AVC denials, parcelable drift) |
| `surfaceflinger-triage` | native display bring-up (SurfaceComposerClient, EGL-on-SurfaceControl) |
| `app-installer-triage` | installer / loader / precompile_component / cache-key failures |

---

## Do NOT (terse — detail in the linked memory/doc)

- **Delete or overwrite `.task-state`.**
- **Run AOT `.cwasm` on desktop** — arm64 only (JIT on desktop, AOT on device).
- **Skip `wasm-tools component embed`** before `component new` — it fails without it.
- **Remove the leading `freeAllComponentModelReallocAllocatedMemory()`** from any
  WIT binding — required forever (the `reallocAllocator`-must-be-null nesting
  constraint, bigger than KT-86415). See `[[wasi-realloc-allocator-pollution]]`.
- **Mismatch the WASI adapter + stdlib halves** — use the wandr-fork adapter
  (KT-86415 State-pin, now on wasmtime 45) paired with the `2.4.258` stdlib
  override; mismatched = State corruption / SIGILL. See `docs/build-pipeline.md`,
  `[[kotlin-wasm-scopedmemory-destroy-bug]]`.
- **Use `FontMgr::default().match_family_style()`** — returns zero-metrics
  typefaces; always `FontMgr::new_from_data(&ttf_bytes, None)`. See `[[feedback_android_fonts]]`.
- **Link Compose guests against the discarded `compose-*-wasi:0.0.0-wasi-local`
  bundles** — use the in-tree `*-wasm-wasi:9999.0.0-SNAPSHOT` modules. See `[[reference-compose-wasi-consumption]]`.
- **Push cwasm to Downloads** (scoped storage) — use the app-specific external dir.
- **Build wandr on ART-layer services** (system_server/WMS/AMS/launcher) — they're
  dropped post-ART; use HAL/binder signals that survive. See `[[feedback_no_art_layer_dependencies]]`.

---

## WIT sync rule (one-liner)

When `wit/skiko-gfx.wit` changes, mirror it to `external/skiko/skiko/wit/` and
every consumer's `wit/deps/skiko-gfx/`. Full command + binding-regen note:
`docs/build-pipeline.md`.
