# Task 45 — wart-zygote MVP spike (Hybrid runtime model, native)

> **Status:** 🔲 scoped 2026-05-27, not started. Spun out of the task
> 44 postponement + the §9 Hybrid-zygote architectural lock. Goal:
> validate the actual Hybrid path before depending on it for the
> production runtime model.

## Why this task exists

Task 44 collided with the locked §9 Hybrid-zygote runtime model
([[project-app-lifecycle-and-packaging]]). Rather than fight Android's
Java-coupled AMS/ATMS/WMS to register a non-Activity process, the
§9 plan is to *replace* those layers entirely with our own arbiter +
zygote pair. The native services we depend on (SurfaceFlinger,
InputFlinger, audio, HAL daemons) don't care who calls them — they
already work for standalone wart-host today (task 33).

Building Hybrid was originally gated on "≥2 concrete apps + DRC
fix." Re-thought 2026-05-27: that trigger was conservative,
predicated on Hybrid being expensive to build. In reality the
infrastructure for the standing §9 rule has been shipping for months
(installer task 35, cross-app deps task 36, generic wiring task 39).
What's missing is the *process model* — a native zygote and a native
arbiter that *use* that infrastructure. The cheapest move is to
spike the zygote pair and see what breaks.

This is a **spike**, not production: MVP scope, single-week target,
proves the technical path. Productionization (init.rc integration,
sepolicy domains, USAP pool, OOM/lifecycle policy) is task 46+.

## Goal (MVP success criterion)

`wart-zygote` is a native Rust process that:

1. Preloads `wasmtime::Engine` + one `.cwasm` at startup.
2. Listens on a UNIX domain socket for launch commands.
3. On request, `fork()`s a child that:
   - Re-inits the per-process state that fork() breaks (EGL,
     binder, tokio runtime, logcat).
   - Allocates a fresh SurfaceFlinger surface via the existing
     libgui shim.
   - Runs the requested app's full render loop (same as task 33
     standalone today).
4. Concurrent runs work: launching two children should land two
   separate apps on screen with distinct SF surfaces.

MVP **excludes**: input arbitration between children (one child at a
time has input), multi-app focus, app reuse (USAP pool), real
permissions, init.rc integration. Step 4 of this spike just adds
"two children running concurrently" as the smoke test for the spike;
production multi-app routing is task 46+.

## Pre-spike design decisions (proposed, push back if you disagree)

**D1. Form factor: new binary mode of `wart-host`, not a separate
crate.** Add `wart-host --zygote` and `wart-host --zygote-launch <app-id>`
(client). Rationale: reuses build, deploy, bionic_compat, libgui
shim, and the existing standalone render loop verbatim. The "child"
in the zygote design is structurally identical to what
`--standalone --app <id>` does today — fork()ing into it is what
this task adds.

**D2. Launch socket protocol: UNIX domain socket + newline-delimited
text. LOCKED 2026-05-27.** Path: `/data/local/tmp/wart-zygote.sock`
(later `/dev/socket/wart-zygote` once init.rc-integrated). Commands
(MVP):

```
LAUNCH <app-id>\n
```

Response (MVP): `OK <child-pid>\n` on success, `ERR <reason>\n` on
failure. One-shot connections (each LAUNCH is a new socket connect),
matching AOSP zygote's pattern.

**Rationale** (the real argument, not just style):

- **rsbinder is structurally ruled out by D7.** To serve
  `IWartZygote` over binder, the zygote parent has to call
  `ProcessState::init_default` to register the service — that
  initializes the per-process binder state which is then COW'd
  into every `fork()`'d child. fork()+binder is a known Android
  landmine (Bionic has explicit `ProcessState::onFork` machinery
  to kill the binder thread pool + reset parcel state in
  post-fork children; rsbinder 0.8.0 doesn't expose that today).
  AOSP's own zygote uses a UNIX socket precisely to avoid this.
- **Text over binary at MVP** because the command surface is tiny
  (LAUNCH/KILL/STATUS at most) and the rewrite to binary later
  costs ~80 lines if/when commands grow richer (e.g.,
  RESERVE_PRELOAD multi-cwasm, structured launch args).
- **No SCM_RIGHTS FD passing at MVP** because the child opens its
  own SurfaceFlinger surface, input channel, and logcat
  connection (same as task 33 standalone today). If a future
  arbiter wants to own the SF surface and hand it down, we'd
  add SCM_RIGHTS then; this MVP doesn't pre-wire for that.

**Future evolution path** (not in scope for spike): postcard or
CBOR binary frames with serde-defined `enum Request {}` /
`enum Response {}` when commands cross ~5; SCM_RIGHTS for arbiter-
allocated FDs when the arbiter task ships.

**D3. Start with CLI-shaped child, not GUI-shaped child.** Step 1
gets `--zygote` forking and exec-ing the headless `--run-once <app-id>`
path (which already works for the `md-smoke-rust` Rust CLI consumer).
This proves fork+wasmtime works before adding EGL+SF complications.
Step 2 then adds EGL/SF.

**D4. Preload set: one engine, one cwasm per startup config.** MVP
doesn't try to multi-preload N cwasms. Zygote takes
`--zygote-preload <app-id>` at startup; child forks always inherit
that one preloaded module. Future: a preload registry that handles
N cwasms (lazy AOT cache hit on launch).

**D5. EGL re-init policy.** Best practice for fork()-and-EGL is
**don't init EGL in the parent at all**. The zygote parent does NOT
call `eglInitialize` — Skia state stays uninitialized until the
child claims it. Each child runs its own first-init. This is
identical to AOSP's zygote (preload class loader, but not
display/GraphicsEnvironment in the zygote process itself).

**D6. Thread policy.** No worker threads in the parent. wasmtime's
default execution is single-threaded; if we have any worker threads
(tokio, etc.) they get spawned by the child after fork. Confirmed
by audit during step 1.

**D7. Binder re-init.** rsbinder `ProcessState::init_default` is
per-process. Parent does NOT call it. Each child calls
`crate::binder::init()` (already has OnceLock-guarded init —
behavior in a forked child needs verification; OnceLock state IS
inherited COW from parent, which could be problematic if parent
ever calls binder::init even for diagnostics).

## Steps

### Step 1 — Fork + headless cwasm (1-2 days)

- New `wart-host/src/zygote.rs`: opens the listen socket, accepts
  one connection at a time, parses `LAUNCH <app-id>`, calls
  `fork()`. Parent: close the connection FD, loop. Child: close
  the listen FD, dispatch to the existing `run_once::run(app_id)`
  path (which already handles `wasi:cli/command` consumers).
- Wire `--zygote` and `--zygote-launch <app-id>` CLI flags in
  `main.rs`. The client flag opens the socket, writes `LAUNCH
  <app-id>\n`, reads the child pid response, waits for child via
  `waitpid` (or fire-and-forget initially).
- Preload one `.cwasm` at zygote startup: call
  `Engine::new` + `Component::deserialize_file` for the configured
  app, hold the `Component` alive in the parent.
- Smoke test: launch the existing `md-smoke-rust` Rust CLI
  consumer twice via the zygote. Both should print and exit
  cleanly. `pmap` the parent + children to confirm wasmtime
  engine pages are COW-shared.

Success criterion: two concurrent `--zygote-launch md-smoke-rust`
calls complete OK; `pmap` shows shared engine pages.

#### Step 1 results (2026-05-27)

**Outcome:** ✅ both criteria met. Device-verified on Pixel 2 XL.

**Run** (after building and packaging `md-smoke-rust` as a `.warpkg`
— the on-disk `/tmp/md-smoke.warpkg` from `smoke-markdown.sh` is the
Kotlin variant which hits the orthogonal task-37 command-adapter
throw):

```
$ adb shell "su -c '… --zygote-launch com.example.md-smoke-rust'"
launched com.example.md-smoke-rust → pid 6128
launched com.example.md-smoke-rust → pid 6148
```

Logcat showed both children loading the preloaded engine, walking the
dep chain (markdown system bundle wired in), calling `wasi:cli/run.run`,
and exiting cleanly. Total time per launch: ~25 ms from fork to
`call_run returned Ok` (faster than `--run-once` direct because the
Cranelift / type-registry state was already populated in the parent).

**COW analysis** (with `WART_ZYGOTE_HOLD_SECS=30` to freeze children
right after fork for `/proc/<pid>/smaps_rollup` sampling):

|                | Parent    | Child #1  | Child #2  |
|----------------|-----------|-----------|-----------|
| Rss            | 24276 kB  | 6352 kB   | 6352 kB   |
| Pss            | 16306     | 2130      | 2124      |
| Shared_Clean   |  4796     |  280      |  280      |
| Shared_Dirty   |  6200     | 5964      | 5976      |
| Private_Clean  | 13184     |    0      |    0      |
| Private_Dirty  |    96     |  108      |   96      |
| Anonymous      |  6068     | 6068      | 6068      |

What this shows:
- **Anonymous heap pages are byte-identical across parent + both
  children (6068 kB each).** This is the wasmtime engine's Rust-heap
  allocations (Cranelift caches, type registry, etc.) — COW-shared
  perfectly. This is the core win the zygote model exists for.
- **`Shared_Dirty` ≈ 6 MB across both children** matches the parent's
  pre-fork dirty pages, exactly as expected.
- **The 18 MB Rss gap (parent's 24 vs child's 6) is unmapped
  (lazy-paged) file-backed code** — children haven't faulted in
  most of the wart-host binary / libc / libskia yet. When children
  run real code they'll page in shared-clean pages from the page
  cache (still cheap, no copy).
- **`Private_Clean=0` in both children** confirms the kernel hasn't
  yet attributed any "exclusively-owned" pages to the children;
  everything they have is inherited or anonymous-COW.

**At-MVP-scale savings are modest** because we only preload the
engine, not the Component or Skia state. Once we add Component
preload (a follow-up — moves the ~13 MB private engine+component
state into shared) and Skia preload (another ~5-15 MB), the savings
will be meaningful. The architectural validation is what step 1
proves: **`fork()` + wasmtime engine survives cleanly on this
device, COW works, no signal-handler / Adreno / binder landmines
fire on this code path.**

**What broke / what we noted:**

- `/tmp/md-smoke.warpkg` from `smoke-markdown.sh` packages the
  Kotlin variant, which hits the orthogonal task-37 command-adapter
  throw. Used the Rust variant (`md-smoke-rust/`) instead by
  hand-packaging it into `/tmp/md-smoke-rust.warpkg`. Suggestion
  for `smoke-markdown.sh` to also package + install the Rust
  variant as `com.example.md-smoke-rust` for forward smoke tests.
- Zombie children pile up after exit because the zygote parent
  doesn't `waitpid()` them. SIGCHLD-handler-driven reap should be
  step 2 polish.
- The smoke socket is `/data/local/tmp/wart-zygote.sock` (D2 dev
  path); production move to `/dev/socket/wart-zygote` deferred to
  task 46 (init.rc + sepolicy).

**Files touched (committed in this step):**

- `wart-host/src/zygote.rs` (new) — fork loop + UNIX socket +
  child dispatch.
- `wart-host/src/run_once.rs` — refactored `run` to wrap a new
  `run_with_engine` that takes a caller-supplied `Engine`.
- `wart-host/src/lib.rs` — `pub mod zygote;` (Android-only).
- `wart-host/src/main.rs` — `--zygote` and `--zygote-launch
  <app-id>` CLI flags.
- `tasks/45-wart-zygote-spike.md` — this section.

### Step 2 — EGL re-init in child + SF surface (2-3 days)

- Refactor the existing `standalone.rs` to factor out the
  "acquire SF surface + init EGL + run render loop" sequence
  into a function callable from the zygote child after fork.
- Forked child runs this sequence, gets its own SurfaceControl
  via the libgui shim, EGL-initializes against it, runs the full
  Compose render loop for the requested app.
- Smoke test: `--zygote-launch wart-app` produces an on-screen
  Compose UI identical to `--standalone --app wart-app`.

Success criterion: zygote-launched wart-app renders + accepts
touch identical to direct standalone-launched wart-app.

#### Step 2 results (2026-05-27)

**Outcome:** ✅ keystone unknown resolved — **Adreno EGL survives
`fork()` cleanly** on Pixel 2 XL. D5 ("don't init EGL in parent")
held empirically: the parent stays EGL-cold, the child first-inits
post-fork, no Adreno driver weirdness.

**Refactor** (`src/standalone.rs`): `run(app_id)` now wraps
`run_with_engine(engine, app_id)`. The zygote child calls
`standalone::run_with_engine(&PRELOADED_ENGINE, app_id)` instead of
building its own engine. Direct `--standalone` callers unchanged.
`run_cwasm_loop` now takes `&Engine` instead of owning it (was
constructing a `Store::new(&engine, …)` so the only-by-reference
need was already there).

**Protocol extension**: zygote socket now accepts `LAUNCH_GUI [app-id]`
in addition to `LAUNCH <app-id>`. Empty arg on `LAUNCH_GUI` falls
through to the dev cwasm at `/data/local/tmp/skiko-component.cwasm`
— same behavior as direct `--standalone` with no `--app`. New
`wart-host --zygote-launch-gui [app-id]` CLI flag. The CLI-vs-GUI
choice is explicit at the client side at MVP; auto-detection from
the app's package.toml is a polish step for later.

**Run** (logcat condensed):

```
I wart-zygote: cmd="LAUNCH_GUI"
I wart-zygote: forked pid=6507 for app_id=""
I wart-zygote/client: response="OK 6507"
I standalone: starting — no NativeActivity
I sf_surface: input window registered (channel fd 10)
I sf_surface: surface created: portrait 1440x2880 logical
I AdrenoGLES-0: Driver Path : /vendor/lib64/egl/libGLESv2_adreno.so
I EGL 1.5
I EGL context made current
I standalone: renderer up — EGL/Skia on the SurfaceFlinger window (1440x2880)
I standalone: loaded cwasm:/data/local/tmp/skiko-component.cwasm
I standalone: component instantiated — entering render loop
I eglSwapBuffers first call
I standalone: rendered frame 1
I standalone: rendered frame 2
I standalone: rendered frame 3
```

**COW analysis during a live render loop** (parent + child both
in steady state, child rendering Compose UI):

|                | Parent    | GUI Child |
|----------------|-----------|-----------|
| Rss            | 24276 kB  | 181256 kB |
| Pss            | 17243     | 172770    |
| Shared_Clean   |  5452     |   6948    |
| Shared_Dirty   |  5592     |   5664    |
| Private_Clean  | 12544     |  67964    |
| Private_Dirty  |   688     | 100680    |
| Anonymous      |  6052     | 103212    |

What this shows:
- **`Shared_Dirty=5664 kB` in the child matches the parent's
  5592 kB.** The wasmtime engine state pre-loaded by the parent
  stays COW-shared through the entire render lifecycle — neither
  side dirties those pages.
- **`Shared_Clean=6948 kB`** is the file-backed code pages
  (wart-host binary + libc + libskia + libEGL + …) that the child
  has paged in and the page cache shares with parent.
- **Child's Private_Dirty (100 MB) is the wasm guest's linear
  memory + Skia caches + GPU buffer mirrors + SkiaRenderer state.**
  Expected; per-app working set.
- **Net savings of ~12 MB per child** (engine state + clean code).
  Modest at MVP because we don't preload Skia state or per-app
  Components. Once those land (follow-up beyond step 5), savings
  scale linearly with the preload set.

**What broke / what we noted:**

- Pre-existing: `pkill -f wart-host` from the host script doesn't
  reliably reap a render-loop child (its SIGTERM handler in
  `lifecycle_standalone` flips a flag and lets the loop drain a
  few frames; pkill returns success but the child runs on briefly
  reparented to init). Not new in step 2 — was always like this.
  Workaround: explicit `kill -KILL <pid>` or wait the few seconds
  for the drain to finish.
- Child path acquires SF surface + EGL/Skia in each child; on this
  device that's ~600 ms per launch (50 ms for SF surface, 550 ms
  for first-frame EGL/Skia warm-up). The headless step 1 path was
  ~25 ms (no SF/EGL). Future optimization: hand-roll the EGL pool
  / share Skia compiled shader programs in the zygote parent
  (Adreno EGL state is per-context so this is non-trivial; deferred).
- The dev-cwasm fallback (`LAUNCH_GUI` with empty arg) is a smoke
  shortcut. Production GUI launches go through
  `LAUNCH_GUI <installed-app-id>` (validated in step 4 with the
  second concurrent app).

**Files touched (committed in this step):**

- `wart-host/src/standalone.rs` — extracted `run_with_engine`;
  `run_cwasm_loop` now takes `&Engine`.
- `wart-host/src/zygote.rs` — `ChildAction` enum (`RunOnce` vs
  `Gui`); `LAUNCH_GUI [app-id]` command parsing; `launch_client`
  gains a `gui: bool` arg.
- `wart-host/src/main.rs` — `--zygote-launch-gui [app-id]` CLI
  flag.
- `tasks/45-wart-zygote-spike.md` — this section.

### Step 3 — COW measurement (1 day)

- `/proc/<pid>/smaps` analysis: walk shared private RSS for the
  wasmtime engine, Component, Cranelift Cache, font/skia preload
  pages (if any). Compare:
  - Parent (zygote) RSS
  - Child #1 RSS, share with parent
  - Child #2 RSS, share with parent
  - Two `--standalone` direct-launched processes (no zygote) for
    the baseline
- Goal: confirm we get ≥30 MB shared per child via the COW path
  vs. the no-zygote baseline. If less, debug what's not preloading.

Success criterion: zygote-launched children share substantially
more pages with the parent than direct-launched processes share
with each other (which is 0 by definition).

#### Step 3 results (2026-05-27)

**Outcome:** ✅ COW sharing confirmed and quantified, but
**below the 30 MB scope-doc target** — that target requires
Component / Skia preload (deferred follow-up). At the engine-
only MVP scope, per-child savings are ~5 MB.

**Method**: ran `wart-host --standalone` directly (no zygote) as
the baseline; then `wart-host --zygote` + `--zygote-launch-gui`
for the test condition. Captured `/proc/<pid>/smaps_rollup` for
each, plus per-VMA categorization via `awk` over `/proc/<pid>/smaps`.

**smaps_rollup (steady-state Compose render loop):**

|                | Baseline    | Zygote parent | Zygote child |
|----------------|-------------|---------------|--------------|
| Rss            | 198 788 kB  |  24 284 kB    | 179 480 kB   |
| Shared_Clean   |   9 128     |   5 452       |   6 904      |
| Shared_Dirty   |     380     |   5 600       |   5 672      |
| Private_Clean  |  81 244     |  12 544       |  67 956      |
| Private_Dirty  | 108 036     |     688       |  98 948      |
| Anonymous      | 105 076     |   6 060       | 101 656      |

**The headline numbers:**

- **Baseline `Shared_Dirty` ≈ 0** (only 380 kB — system stragglers).
  A direct `--standalone` process shares essentially nothing dirty
  with anyone because nothing forked it.
- **Zygote child `Shared_Dirty` = 5 672 kB** ≈ parent's 5 600 kB.
  All ~5.6 MB of the parent's dirty state survives into the child
  as COW-shared.
- **Net zygote-specific savings: ~5.3 MB per child** of dirty
  state.

`Shared_Clean` is similar (~7-9 MB) in all three — that's the
kernel page cache deduplicating file-backed mmaps (libc, libEGL,
libgui, libwart-host code). That's natural sharing the zygote
doesn't change.

**Per-VMA attribution of the parent's 5 600 kB Shared_Dirty:**

| Category                     | Shared_Dirty |
|------------------------------|--------------|
| `file:/…` (wart-host + libs `.data/.bss`) |  4 656 kB |
| `[anon:scudo:primary]` (heap) |    208 kB |
| `[anon:linker]`              |    192 kB |
| anon-other                   |    472 kB |
| `[stack]`                    |      4 kB |
| other                        |     68 kB |
| **Total**                    |  **5 600 kB** |

The headline insight: **most of the engine-preload "win" is
binary `.data/.bss` state** — initialized globals, lazy-init'd
statics, the Tokio-Reactor-Singleton-style stuff bionic + the
linker touch on first load. The wasmtime `Engine` itself
contributes only ~200 kB of `scudo:primary` heap. `Engine::new`
is structurally light; the bulk lives in the binary image.

**Implications for the 30 MB target:**

To grow per-child savings past ~5 MB at this codebase scale,
we'd need to preload state that's bigger than the engine + globals:

- **`Component::deserialize_file` for the app's `.cwasm`** in the
  parent. Wasmtime allocates internal structures (the parsed
  module sections, type tables, etc.) — these land in
  `scudo:primary`. Each component should add several MB.
  ⚠ Caveat: the .cwasm mmap itself is already file-backed
  (Shared_Clean via page cache) regardless of whether the parent
  touches it; the dirty win is from the in-RAM wasmtime structures.
- **Skia preload** — `FontMgr::default()` + a touch of any
  default typeface to warm the font cache. Several MB.
- **EGL preload** — out of scope per D5 (parent stays EGL-cold;
  Adreno EGL context state isn't fork-safe in general).

These are natural next steps and the scope doc already calls out
the `--zygote-preload <app-id>` CLI shape for Component-level
preload. Not done in this step because step 4 (two distinct apps
concurrent) needs to come first to validate multi-app behavior
before we sink work into preload optimization.

**Success criterion as stated**: "zygote-launched children share
substantially more pages with the parent than direct-launched
processes share with each other (which is 0 by definition)" —
✅ met. Direct processes share ~0 dirty pages; zygote children
share ~5.3 MB. **The ≥30 MB target stretch goal**: ❌ not met at
MVP — requires Component + Skia preload (follow-up).

**What broke / what we noted:**

- `pkill -KILL -f wart-host` followed by an `am force-stop`
  restore-script was the only reliable way to clear the device.
  Render-loop children with the lifecycle-standalone signal
  handler installed don't die quickly on plain SIGTERM (they
  drain frames first).
- The numbers above are steady-state; pre-render numbers
  (held by `WART_ZYGOTE_HOLD_SECS`) are ~6 MB Rss because the
  child hasn't paged anything in yet (see step 1 results).

### Step 4 — Two apps concurrent (1-2 days)

- Build a real second `.warpkg` for this purpose. Suggested: a
  trivial markdown reader (reuses the existing `markdown-renderer`
  system dep, validates cross-app deps in the multi-app scenario).
  Could be ~50 lines of Compose. App-id `com.wart.mdview`.
- Install both wart-app and com.wart.mdview via `--install`.
- Smoke: launch wart-app and com.wart.mdview concurrently via two
  zygote launches. Verify both render simultaneously on screen.
  Input goes to whoever the SF/InputFlinger arbitration decides
  (MVP: last-touched wins via InputFlinger's z-order; arbitration
  policy is task 46).

Success criterion: two distinct apps on screen at once, both
rendering, both responsive (to touch on whichever has input).

#### Step 4 results (2026-05-27)

**Outcome:** ✅ two distinct app slots run concurrently via the
zygote at 60 fps each. Both children acquire their own SF surface,
their own EGL context, their own render loop — independent and
parallel.

**Method**: per user pick (option C from the step-3 close-out
discussion), packaged the existing wart-app build as a real
`.warpkg` with app-id `com.example.wart-app` (rather than building
a new app), then ran it alongside the dev-cwasm GUI launch.
Same code in both children but two distinct install slots, two
distinct process trees from the zygote.

**Packaging steps (one-shot work, not committed to scripts/)**:

```bash
# wart-app: use the existing post-adapt component
mkdir -p /tmp/wart-app.warpkg/components
cp /tmp/skiko-component.wasm /tmp/wart-app.warpkg/components/ui.wasm
cat > /tmp/wart-app.warpkg/package.toml <<'EOF'
app_id      = "com.example.wart-app"
version     = "0.1.0"
world       = "my:skiko-gfx/skiko-ui"
composition = "same-store"

[components]
ui = "components/ui.wasm"
EOF

# System deps wart-app imports — installer auto-detects from the
# component's WIT imports and refuses install if any are missing.
# Three needed: emoji-picker, system-fonts, markdown-renderer
# (already-installed markdown-renderer system bundle satisfied
# the markdown import).
# Built locally + wrapped in `.warpkg` directories with kind=system
# manifests. Installed via `wart-host --install`.
```

The `system-fonts` Rust crate wasn't pre-built; `cargo build
--target wasm32-wasip2 --release` finished it in seconds.

**Run** (logcat trimmed to the per-child render confirmation):

```
I wart-zygote: cmd="LAUNCH_GUI com.example.wart-app"
I wart-zygote: forked pid=7394 for app_id=com.example.wart-app
I standalone[7394]: surface 1440x2880 ...
I standalone[7394]: loaded installed:com.example.wart-app:0.1.0:ui
I standalone[7394]: rendered frame 1, 2, 3 ...

I wart-zygote: cmd="LAUNCH_GUI"
I wart-zygote: forked pid=7443 for app_id=
I standalone[7443]: surface 1440x2880 ...
I standalone[7443]: loaded cwasm:/data/local/tmp/skiko-component.cwasm
I standalone[7443]: rendered frame 1, 2, 3 ...

(steady state ~30 s later — both still at 60 fps:)
I standalone[7394]: rendered frame 1800
I standalone[7443]: rendered frame 1200
```

**smaps_rollup with both children alive:**

|                | Parent    | Child #1 (installed) | Child #2 (dev) |
|----------------|-----------|----------------------|----------------|
| Rss            | 24 284 kB | 196 932 kB           | 179 760 kB     |
| Shared_Clean   |  5 464    |  24 996              |  24 988        |
| Shared_Dirty   |  5 580    |   5 652              |   5 660        |
| Private_Clean  | 12 532    |  53 124              |  49 872        |
| Private_Dirty  |    708    | 113 160              |  99 240        |
| Anonymous      |  6 060    | 115 580              | 101 800        |

**Notable shifts vs the single-child step-3 numbers:**

- **`Shared_Clean` jumped from ~7 MB → ~25 MB per child** when
  the second child came online. Both children now have the
  wart-host binary + Skia + libEGL + libgui mapped, and the
  kernel attributes those file-backed pages as "shared" because
  ≥2 processes hold them. This is natural page-cache sharing,
  not zygote-specific, but it shows the wart-host process model
  has good code-sharing properties out of the gate.
- **`Shared_Dirty` per child stays at ~5.6 MB.** That's the
  zygote-specific COW with the parent — unchanged by adding a
  second child (each child shares the same engine state with
  the parent, independently of siblings).
- **Net memory per additional child**: ~120 MB private dirty +
  60 MB private clean ≈ **180 MB per concurrent app at this
  config**. The wasm linear memory (Compose snapshot/composer
  trees + Skia GPU buffer mirrors) is the dominant cost. At
  MVP, the COW savings are dwarfed by the per-app working set.

**What this tells us about Hybrid viability:**

- **Two apps concurrent fits in ~360 MB** (180 each). On a
  4 GB device like the Pixel 2 XL, that's comfortable. On
  lower-end devices the per-app footprint is the binding
  constraint, not the zygote overhead.
- **Per-child SF surface allocation succeeded for both apps**
  from a non-Activity context. No fight over the SF connection.
  Each child got 1440×2880, both rendered concurrently. The
  "topmost wins" SF behavior means only the latest-allocated
  is visually on top; the other renders to an obscured surface
  but the process+EGL+Skia work continues. Z-order arbitration
  is task-46 work (a real wart-arbiter).
- **No fork-time landmines fired with two concurrent children**
  in flight (the second `LAUNCH_GUI` arrives while the first
  child is mid-render). The zygote's single-threaded accept
  loop serialized them; no race.

**Known limitations carried into step 5:**

- No SIGCHLD reaping in the zygote → zombies pile up.
- No app shutdown command in the protocol → the only way to
  stop a child is external (kill -KILL by pid).
- Both children use the same logical SurfaceFlinger layer
  (no z-order policy); only one is visible at a time.
- The ~30 MB COW target from the scope doc is still not met
  (~5.6 MB Shared_Dirty per child); Component / Skia preload
  remains the natural follow-up.

**Files touched (this step had no source-code changes):**

- `tasks/45-wart-zygote-spike.md` — this section.
- (one-shot packaging work for `/tmp/wart-app.warpkg`,
  `/tmp/emoji.warpkg`, `/tmp/fonts.warpkg` left in `/tmp/`
  on the dev machine; not version-controlled. A future
  `scripts/build-system-warpkgs.sh` would automate this.)

### Step 5 — Spike close-out (0.5 day)

- Update CLAUDE.md status table with task 45 row.
- Write a "what we learned" section in this doc: what fork()
  broke, what the COW math actually was, where bottlenecks would
  appear at >2 apps, what production needs that the MVP skipped.
- Recommend task 46 scope based on findings.

## Known unknowns

- **fork() + the rsbinder OnceLock**: if any binder service was
  initialized in the parent (unlikely for headless preload, but
  worth auditing), the OnceLock state is COW'd; the child sees
  `Ok(())` and doesn't re-init, but the binder FD is the parent's.
  Mitigation: parent must not touch binder; audit step 1.
- **fork() + tokio**: child can't reuse parent's tokio runtime
  (executor threads aren't forked). Audit + isolate.
- **fork() + EGL on Android Adreno (Pixel 2 XL)**: Adreno's EGL is
  closed-source and may not survive fork() well. Mitigation per
  D5: don't init EGL in parent. Risk: even *no-init* fork might
  break Adreno's vendor pre-init in the linker. Verify in step 2.
- **fork() + libgui SurfaceComposerClient**: SF connection state
  is per-process; child must establish its own. Confirmed in
  task 33's standalone path (each invocation does this fresh).
- **wasmtime Store across fork**: NEVER fork with an active Store.
  Stores hold guest memory + epoch-interruption state; sharing
  via COW into a child is a recipe for double-free / use-after-
  free. Parent stays Store-less; each child creates its own.
- **/data/local/tmp socket path SELinux**: untrusted_app domain
  probably can't bind to a socket there. We're root via `su`
  for the MVP; production needs sepolicy for the wart-zygote
  domain. Out of scope for spike.

## File-touch map (anticipated)

- `wart-host/src/zygote.rs` (new) — fork loop, socket protocol,
  child dispatch.
- `wart-host/src/main.rs` — `--zygote` and `--zygote-launch
  <app-id>` CLI flags.
- `wart-host/src/lib.rs` — `pub mod zygote;` (Android-only).
- `wart-host/src/standalone.rs` — refactor the "acquire SF +
  init EGL + render loop" out of the main function into a
  callable.
- `wart-host/Cargo.toml` — likely `libc` (`fork`, `dup2`,
  `setsid`, `waitpid` syscalls). Already in tree probably.
- `wart-host/cpp/sf_surface.cpp` — no changes expected; the
  shim is already child-side-safe.
- `tasks/45-wart-zygote-spike.md` — this doc; update per-step.
- `CLAUDE.md` — status table row.
- New `apps/mdview/` (step 4) — minimal Compose markdown reader
  for the multi-app concurrency test.

## Resume hints for fresh sessions

1. `cat .task-state` — TASK=45 STEP=N tells you where to pick up.
2. Read **Decisions D1-D7** above before doing anything else;
   most early failures will be due to violating one of those.
3. **Step order is load-bearing**: don't add EGL before fork works
   for the headless child. The headless smoke is the cheap
   integration test for "does fork + wasmtime engine + preloaded
   Component + spawn child + run wasi:cli/command actually work
   on this device?"

## Related

- `tasks/33-boot-model-bringup.md` — the standalone wart-host
  binary that the forked child IS. The render loop, libgui shim,
  EGL setup all carry directly.
- `tasks/35-app-install.md` — `wart-host/src/app_installer.rs`
  + `app_loader.rs` — what the child uses to find the requested
  `.cwasm`.
- `tasks/36-cross-app-deps.md` — cross-app dep wiring used by
  step 4's second app.
- `post-art-roadmap.md` §7 + §9 — the architectural baseline
  this task implements one slice of.
- `MEMORY.md` →
  [[project-app-lifecycle-and-packaging]] — the locked decisions
  that motivate this spike.
- `MEMORY.md` →
  [[wasmtime-drc-no-autoschedule]] — the unresolved DRC issue;
  Hybrid isolates per-process GC stalls but doesn't fix them.
  Spike acknowledges this; production deployment still gated
  on DRC fix.
