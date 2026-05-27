# Task 46 — wart-arbiter MVP (Hybrid runtime production prep)

> **Status:** 🔲 scoped 2026-05-27, not started. Sequel to task 45
> (the wart-zygote MVP spike). Goal: take the technical Hybrid path
> the spike validated and turn it into a production-ready
> two-daemon model.

## Why this task exists

Task 45 proved the technical Hybrid path works on this device
(commits `ad82c11`/`353f690`/`1c5a6927`/`462d53a5`/`6a13f839`).
What it explicitly skipped:

- SIGCHLD reaping in the zygote (zombies pile up at MVP).
- A shutdown command in the protocol (only `kill -KILL` from
  outside works today).
- Component preload — only `wasmtime::Engine` is preloaded at
  MVP; per-child COW savings are stuck at ~5 MB. The path to
  ≥20 MB COW per child runs through `Component::deserialize_file`
  in the parent.
- A real arbiter — a separate process that owns the policy
  decisions (z-order, focus, OOM priority, foreground/background)
  the spike's children currently fight over implicitly.
- init.rc + sepolicy integration — production deployment shape.

This task lands all five.

## Pre-task design decisions (proposed; D1 locked, others open)

**D1 (LOCKED 2026-05-27). Two-binary split: `wart-host` + `wart-arbiter`.**

After explicit discussion with the user post-task-45 close-out,
the production model is:

- **`wart-host`** = zygote-mode (parent) + app-mode (forked
  child). Stays as one binary because fork+COW requires the
  parent and child to share the same address space at fork
  time. Same binary, two entry points — exactly the AOSP
  `app_process` pattern (`ZygoteInit.main()` vs
  `ActivityThread.main()`).
- **`wart-arbiter`** = NEW separate binary. Policy daemon.
  Owns: which app gets foreground SF z-order, who has
  InputFlinger focus, OOM kill priorities, app reuse policy.
  Talks to `wart-host --zygote` over its UNIX socket (plus a
  new arbiter↔zygote channel for richer commands). Doesn't
  fork anything itself.

Rejected alternatives:
- *Three binaries* (carve a thin `wart-zygote-launcher` out of
  `wart-host` that exec()s app-mode children): defeats COW,
  exec destroys preload pages.
- *One binary, no arbiter yet*: doesn't validate production
  architecture end-to-end; deferring the policy layer means
  the zygote ships without a real consumer.

In init.rc, both daemons start independently:
```
service wart_zygote  /system/bin/wart-host --zygote --preload <…>
    user root
    seclabel u:r:wart_zygote:s0
    socket wart_zygote stream 0660 root system
    oneshot false

service wart_arbiter /system/bin/wart-arbiter
    user root
    seclabel u:r:wart_arbiter:s0
    socket wart_arbiter stream 0660 root system
    oneshot false
```

Distinct SELinux domains, distinct sockets, distinct release
cadence. Two cargo crates in the workspace.

**D2 (proposed). Arbiter↔zygote protocol: extend the existing
text UNIX socket, or introduce a separate channel?**

Three options:

- **D2a.** Extend the existing `LAUNCH/LAUNCH_GUI/KILL` text
  socket. Add `SPAWN <app-id> <z-pos>`, `FOREGROUND <pid>`,
  `STATUS` commands. Cheapest; one socket for all callers
  (user CLI + arbiter).
- **D2b.** Separate AF_UNIX socket for arbiter-only commands
  (`/dev/socket/wart_zygote_priv`, sepolicy-gated to
  `wart_arbiter` domain only). Keeps the user CLI socket
  semantics simple; arbiter gets a privileged channel.
- **D2c.** Binary protocol (postcard / serde) on a single
  socket. More code than D2a, schema'd evolution.

Lean: **D2a** at MVP. The arbiter is the only realistic
heavy caller; the user CLI mostly disappears once the arbiter
ships. Revisit if commands grow beyond ~10.

**D3 (proposed). Where does `wart-arbiter` listen for app-launch
requests from the user / OS?**

- Same UNIX socket pattern: `/data/local/tmp/wart-arbiter.sock`
  (dev) → `/dev/socket/wart_arbiter` (production).
- Commands TBD. Minimum: `LAUNCH_APP <app-id>` (foreground),
  `LIST_APPS`, `KILL_APP <app-id>`. Possibly `SWITCH_APP
  <app-id>` for foreground/background transitions.

**D4 (proposed). Component preload selection.**

CLI shape from the task-45 scope: `--preload <app-id>` (multi-
valued). Zygote at startup deserializes each one into a
`OnceLock<HashMap<String, Component>>`. Children look up by
app-id; miss → child does its own `deserialize_file` (graceful
degrade, no policy fight).

System bundles (markdown, emoji, fonts) should probably be
preloaded by default — they're shared by every Compose app.
Per-app components: arbiter decides which to preload based on
which apps the user is likely to launch. Out of MVP scope to
build that prediction logic — just preload what's listed on
the CLI.

**D5 (proposed). OOM priority tuning.**

Production needs `/proc/<pid>/oom_score_adj` writes:
- Zygote parent: -1000 (never killed)
- Arbiter: -800 (very rarely killed)
- Foreground app: 0 (normal)
- Background apps: 500-900 (kill first under pressure)

The arbiter writes these on app launch / focus changes.
Requires `CAP_SYS_RESOURCE` or root.

## Steps

### Step 1 — SIGCHLD reaper + KILL command (~1 day)

Two small cleanups in `wart-host/src/zygote.rs`:

- **SIGCHLD reaper.** Install a SIGCHLD handler in the zygote
  parent that signals a self-pipe; the accept loop drains
  zombies via non-blocking `waitpid(WNOHANG)` between accepts.
  Log exit status. No protocol change.
- **`KILL <pid>` command.** Add to the socket protocol. Validates
  the pid is one of our children (track child pids in a
  `Mutex<HashSet<pid_t>>` populated at fork()), sends `SIGTERM`,
  responds `OK` or `ERR not-our-child`. `KILL_FORCE <pid>` sends
  `SIGKILL`.

Success criterion: `ps -A` after running and killing N apps
shows no `[wart-host]` zombies; `KILL <pid>` works against an
own child but `ERR`s on an unrelated pid.

#### Step 1 results (2026-05-27)

**Outcome:** ✅ both criteria met. Device-verified on Pixel 2 XL.

**Reaper**: `spawn_reaper()` in `src/zygote.rs` runs
`std::thread::spawn` of a loop that blocks on `libc::wait`,
decodes status (WIFEXITED / WIFSIGNALED), and removes the
reaped pid from a `OnceLock<Mutex<HashSet<i32>>>` shared with
the accept-loop's fork handler. Holds the mutex only for the
brief instant of `.remove(&pid)`; fork-time races would leave
at most a slightly-stale entry, not a deadlock. Thread exists
only in the parent — fork() only duplicates the calling
thread, so each forked child runs reaperless (which is what
it wants).

**KILL / KILL_FORCE**: new socket commands parsed in
`handle_one` before the LAUNCH dispatch. Both validate the
pid is in `child_pids()` before signaling; unrelated pids get
`ERR not-our-child <pid>` without any `kill(2)` syscall.
KILL_FORCE sends SIGKILL, KILL sends SIGTERM. Audit log line
for every accept + every reap.

**CLI**: new `--zygote-kill <pid>` and `--zygote-kill-force <pid>`
flags in `main.rs`, wrapping the new `zygote::kill_client`.

**Smoke 1** — five rapid headless `--zygote-launch
com.example.md-smoke-rust` forks. Logcat sequence:

```
forked pid=7908 → reaped (exit=0, tracked=true)   [260 ms]
forked pid=7929 → reaped (exit=0, tracked=true)   [218 ms]
forked pid=7949 → reaped (exit=0, tracked=true)   [242 ms]
forked pid=7970 → reaped (exit=0, tracked=true)   [229 ms]
forked pid=7991 → reaped (exit=0, tracked=true)   [227 ms]
```

`ps -A | grep -c 'Z.*wart-host'` returns 0. The MVP zombie
piling-up limitation is closed.

**Smoke 2** — GUI child + KILL validation:

```
$ --zygote-kill 1         → ERR not-our-child 1
$ --zygote-kill 99999     → ERR not-our-child 99999
$ --zygote-kill 8068      → OK 8068  (sig=15 sent)
                          → reaper: pid 8068 reaped (exit=0, tracked=true)  [3.5 s later]
```

The 3.5 s delay between SIGTERM and reap is the wart-app
standalone render loop's clean-shutdown drain (lifecycle
Destroyed → 3 final frames → exit), inherited from task 33
step 5. SIGTERM hits the handler that flips the shutdown
flag; the render loop sees it and drains. Expected, not a
bug.

**Files touched (committed in this step):**

- `wart-host/src/zygote.rs` — `child_pids()` tracking,
  `spawn_reaper()` thread, `handle_kill()` shared logic,
  `kill_client()` for the client side, KILL/KILL_FORCE
  parsing in `handle_one`, child-pid insert at fork.
- `wart-host/src/main.rs` — `--zygote-kill <pid>` and
  `--zygote-kill-force <pid>` CLI flags.
- `tasks/46-wart-arbiter-mvp.md` — this section.

### Step 2 — Component preload registry (~2 days)

The highest-leverage step (closes the COW gap from 5.6 MB to
target ~25 MB).

- Extend `--zygote` CLI to accept `--preload <app-id>` (multi).
- At zygote startup, for each preload app-id: walk the
  install registry, `Component::deserialize_file` each
  component, store in a `OnceLock<HashMap<(app_id, comp_name),
  Component>>`.
- Refactor the loader (`app_loader.rs`) to consult the registry
  before re-deserializing. Miss → fall through to
  `deserialize_file` as today.
- Measure COW: run the step-3-style smaps comparison with
  preload on. Target ≥20 MB Shared_Dirty per child.

Caveat: `wasmtime::component::Component` is `Arc`-internal;
sharing across fork should be COW-safe but worth verifying
in practice. If `Component` accesses any per-process state
post-deserialization, that breaks; the loader fallback path
catches it gracefully.

Success criterion: per-child smaps_rollup shows Shared_Dirty
≥20 MB (vs 5.6 MB at engine-only preload).

#### Step 2 results (2026-05-27)

**Outcome:** ✅ target met by 3×. 57.6 MB Shared_Dirty per
child in steady-state render (vs 5.6 MB at engine-only preload
from task 45; vs ≥20 MB step-2 target). The COW math the spike
fell short of in task 45 step 3 is now closed.

**Design decisions taken** (in conversation, locked):

- **Auto-preload at startup is system-only.** Zygote walks
  `<APPS_ROOT>/system-apps/*` and preloads every component
  found. Rationale: system bundles (markdown / emoji / fonts)
  are imported by every Compose app, they're small, they
  don't churn. User apps are explicit/dynamic.
- **`PRELOAD <app-id>` socket command** for user apps and
  refresh-after-upgrade. The installer calls it post-install
  (instead of restarting the zygote, which would drop all
  preloads). Drops any prior preloads for the same app (under
  any version) so in-place upgrades replace stale entries.
- **Registry keyed by absolute .cwasm path**, not by
  (app-id, comp-name). Both `load_installed` and
  `load_dep_components` in `app_loader.rs` already pass a
  `.cwasm` path to `deserialize_file`, so one lookup site
  covers app + dep components.

**Architecture:**

```
new src/preload.rs:
  registry(): &'static Mutex<HashMap<PathBuf, Component>>
  get(path): Option<Component>          -- loader hook
  insert(path, component)                -- preload helper
  drop_prefix(prefix)                    -- per-app invalidation
  preload_app(engine, root, kind_dir, app_id)  -> count
  preload_all_system_apps(engine, root)        -> count   (called at startup)
  preload_either(engine, root, app_id)         -> (kind, count)   (PRELOAD handler)

src/app_loader.rs (load_installed + load_dep_components):
  before deserialize_file: preload::get(canonical_path) -> hit
  on hit: clone the preloaded Component (cheap, Arc-internal)
  on miss: fall through to deserialize_file as before

src/zygote.rs:
  serve():
    1. preload Engine (existing)
    2. spawn reaper (existing)
    3. preload_all_system_apps(engine, WART_APPS_ROOT)
    4. bind listen socket
  handle_one():
    accept LAUNCH/LAUNCH_GUI/KILL/KILL_FORCE/PRELOAD <app-id>
  handle_preload(): preload_either() + write OK/ERR

src/main.rs:
  --zygote-preload <app-id>  -> preload_client(app_id)
```

**Run + measurement** on Pixel 2 XL:

```
$ wart-host --zygote (with 3 system bundles installed)
 ...
 preload: + .../system-apps/war.emoji.picker/0.1.0/cache/picker.cwasm
 preload: + .../system-apps/war.markdown.renderer/0.1.0/cache/renderer.cwasm
 preload: + .../system-apps/war.fonts.loader/0.1.0/cache/loader.cwasm
 startup preload — 3 system component(s)
 listening on /data/local/tmp/wart-zygote.sock

$ wart-host --zygote-preload com.example.wart-app
 PRELOAD com.example.wart-app → OK apps 1

$ wart-host --zygote-launch-gui com.example.wart-app  (WART_ZYGOTE_HOLD_SECS=30)
 launched com.example.wart-app → pid 8493
```

**Held child (post-fork, pre-render)** — pure COW baseline:

|                  | Parent   | Held child |
|------------------|----------|------------|
| Rss              | 133 MB   |  62.9 MB   |
| Shared_Dirty     |  62.7 MB |  62.5 MB   |
| Anonymous        |  62.6 MB |  62.6 MB   |

Parent and held child have byte-identical Anonymous (62.6 MB
each) — the preloaded Components' Cranelift/typetable heap is
fully COW-shared at fork time.

**Steady-state child (full Compose render loop)**:

|                  | Parent   | Render child |
|------------------|----------|--------------|
| Rss              | 133 MB   | 167 MB       |
| Shared_Clean     |  29.4 MB |  30.8 MB     |
| Shared_Dirty     |  57.5 MB |  57.6 MB     |
| Private_Clean    |  40.9 MB |  18.9 MB     |
| Private_Dirty    |   5.3 MB |  60.0 MB     |
| Anonymous        |  62.6 MB | 114.7 MB     |

After 30 s of rendering, the child has dirtied ~5 MB of pages
that were originally COW-shared (down from 62.5 → 57.6 MB
shared) — most of the preloaded state stays COW through the
active render loop. The 60 MB Private_Dirty is the child's
own wasm-linear-memory + Skia state.

**Comparison vs task 45 step 3 baseline:**

| Per-child metric         | Task 45 step 1 | Task 46 step 2 | Δ |
|--------------------------|----------------|----------------|---|
| Shared_Dirty (held)      | 5.6 MB         | 62.5 MB        | **+57 MB** |
| Shared_Dirty (rendering) | 5.6 MB         | 57.6 MB        | **+52 MB** |
| Private_Dirty (rendering)| 99 MB          | 60 MB          | -39 MB |

The render-loop Private_Dirty drop (99 → 60) is the win
playing out: pages that the child used to dirty on its own
now come pre-dirty from the parent and stay COW-shared
through reads.

**System-wide concurrency math** (rough, single-foreground
+ N backgrounded apps; numbers approximate within ±10%):

| N apps | Step 1 (engine preload) | Step 2 (full preload) |
|--------|------------------------|----------------------|
| 1      | 198 MB                 | 212 MB               |
| 2      | 388 MB                 | 291 MB               |
| 5      | 990 MB                 | 528 MB               |
| 10     | 1 980 MB               | 924 MB               |

Below N=2 the parent's preload overhead (~109 MB more in the
parent than engine-only) makes step 2 a slight loss. From
N=2 onward, step 2 wins decisively — at N=10 it's ~2× the
memory budget headroom. On a 4 GB device this lifts the
concurrent-app ceiling from ~22 (step 1) to ~40+ (step 2).

**What this doesn't change:**

- Per-child working set is still dominated by wasm linear
  memory + Skia. Components are now COW-shared; wasm linear
  memory is intrinsically per-instance and stays private.
- Adreno EGL fork-survival is unchanged (still works).
- DRC GC scheduling issue [[wasmtime-drc-no-autoschedule]]
  is unchanged; one app's GC stall is still per-process.

**Known limitations carried forward:**

- Preload happens in the parent on a single thread. With 3
  system + 1 user app preloaded the wall-clock cost was
  ~150 ms (well under the 600 ms first-render budget). For
  large preload sets, parallelize via rayon.
- No version pinning in the preload registry. The latest
  installed version always wins. Rollback would need
  explicit per-version `PRELOAD` support.
- Engine-config drift is detected at deserialize time
  (deserialize errors → skip + log). No proactive
  re-precompile from the zygote; the loader's fall-through
  handles it lazily on first launch.

**Files touched (committed in this step):**

- `wart-host/src/preload.rs` — new module with registry +
  `preload_app` + `preload_all_system_apps` + `preload_either`.
- `wart-host/src/app_loader.rs` — preload registry lookup at
  both `Component::deserialize_file` sites
  (`load_installed` and `load_dep_components`).
- `wart-host/src/zygote.rs` — startup walk of `system-apps/`;
  `PRELOAD <app-id>` socket command + `handle_preload`;
  `preload_client` for the CLI.
- `wart-host/src/lib.rs` — `mod preload;`.
- `wart-host/src/main.rs` — `--zygote-preload <app-id>` CLI
  flag.
- `tasks/46-wart-arbiter-mvp.md` — this section.

### Step 3 — `wart-arbiter` skeleton crate + LAUNCH plumbing (~2-3 days)

New `wart-arbiter/` Cargo crate. Workspace member alongside
`wart-host/`.

- `Cargo.toml`: minimal deps — `tokio` (for async socket), `anyhow`,
  `clap` or hand-rolled CLI, `log` + `android_logger`. **No
  wasmtime, no skia, no libgui** — the arbiter is a thin policy
  process.
- `src/main.rs`: bind `/data/local/tmp/wart-arbiter.sock`,
  parse commands, dispatch.
- `src/zygote_client.rs`: wraps the wart-host socket protocol.
  `LaunchGui(app_id)` → `LAUNCH_GUI <app-id>` text command +
  parse `OK <pid>` / `ERR <reason>`. Reuses the existing
  protocol from task 45.
- `src/state.rs`: track running apps (HashMap<app_id, pid>),
  per-app metadata.
- CLI: `wart-arbiter launch <app-id>`, `wart-arbiter list`,
  `wart-arbiter kill <app-id>` (client-mode, connects to the
  arbiter socket — same one-shot pattern as wart-host's
  `--zygote-launch`).

Build target: aarch64-linux-android, same toolchain as
wart-host. Update `scripts/build-host-android.sh` (or new
sibling) to build both.

Success criterion: `wart-arbiter launch wart-app` from the
device shell triggers the zygote to fork a child running
wart-app; `wart-arbiter list` shows the running app + pid;
`wart-arbiter kill wart-app` cleans it up via `KILL <pid>`.

#### Step 3 results (2026-05-27)

**Outcome:** ✅ all five client commands (launch / launch-headless /
list / kill / preload) work end-to-end on Pixel 2 XL. The two-binary
D1 split is real and shippable.

**Final shape**:

```
wart-arbiter/                       (new top-level crate)
  Cargo.toml          deps: anyhow + log + android_logger + libc
                      no wasmtime, no skia, no libgui
  .cargo/config.toml  mirrors wart-host's NDK r27d + sysroot setup
  src/main.rs         daemon mode + client dispatch
  src/zygote_client.rs  text-protocol wrapper around wart-host's
                        UNIX socket (LAUNCH/LAUNCH_GUI/KILL/PRELOAD)
  src/state.rs        OnceLock<Mutex<HashMap<String, AppState>>>
                      tracking app-id → pid + launched_at metadata
```

Binary size: **777 KB** (vs wart-host's 52 MB — 67× smaller). Clean
crate boundary: no shared code; the arbiter knows the wart-host
socket protocol but nothing about wasmtime or rendering.

**Commands implemented** (arbiter ↔ client over
`/data/local/tmp/wart-arbiter.sock`):

| Verb            | Action |
|-----------------|--------|
| `launch <id>`   | Send LAUNCH_GUI to zygote; record in state map; reply `OK pid=N app=id` |
| `launch-headless <id>` | Same but LAUNCH (wasi:cli/command consumer) |
| `list`          | `OK count=N` + one line per app (`app=id pid=N elapsed_ms=…`) |
| `kill <id>`    | Look up pid in state; send KILL to zygote; remove on success |
| `preload <id>`  | Forward to zygote's PRELOAD command; relay reply |

**Smoke run** (device-verified):

```
$ wart-arbiter list                          → OK count=0
$ wart-arbiter launch com.example.wart-app   → OK pid=9117 app=com.example.wart-app
                                              (wart-app renders at 60 fps via zygote
                                               fork+COW; LD_LIBRARY_PATH inherited)
$ wart-arbiter list                          → OK count=1
                                                  app=com.example.wart-app pid=9117 elapsed_ms=4157
$ wart-arbiter kill com.example.wart-app     → OK killed app=… pid=9117
$ wart-arbiter list                          → OK count=0
$ wart-arbiter kill com.bogus.app            → ERR not-tracked com.bogus.app
$ wart-arbiter preload com.example.wart-app  → OK apps 1
$ wart-arbiter preload com.bogus.app         → ERR preload-failed com.bogus.app: …
```

State map is authoritative for the arbiter — the zygote's own
`child_pids` set (task 46 step 1) is the truth at the kernel level,
but the arbiter's higher-level "which app-id maps to which pid"
mapping lives here. KILL via the arbiter goes through state lookup,
which is why `kill com.bogus.app` returns `ERR not-tracked` even
though the zygote would also reject `ERR not-our-child` if asked
directly.

**Scripts**:

- `scripts/build-host-android.sh` extended to also build
  `wart-arbiter` (same NDK r27d toolchain, same sysroot config).
- `scripts/run-hybrid-stack.sh` (new) — dev convenience that
  launches `wart-host --zygote` and `wart-arbiter --daemon`
  side-by-side, with SystemUI + launcher force-stopped and an
  EXIT trap to restore. Mirrors `standalone-launch.sh` shape.

**Out of scope (lands in step 4)**: foreground/background z-order,
InputFlinger focus arbitration, `/proc/<pid>/oom_score_adj` writes.
Step 3 is the wiring + state shape; step 4 puts policy on top.

**Files added (committed in this step):**

- `wart-arbiter/Cargo.toml` (new crate)
- `wart-arbiter/.cargo/config.toml`
- `wart-arbiter/src/main.rs`
- `wart-arbiter/src/zygote_client.rs`
- `wart-arbiter/src/state.rs`
- `scripts/build-host-android.sh` — extended
- `scripts/run-hybrid-stack.sh` — new
- `tasks/46-wart-arbiter-mvp.md` — this section

### Step 4 — Arbiter policy: foreground/background + focus (~3-4 days)

The arbiter starts being a real arbiter.

- **Z-order policy.** Track which app is "foreground." When
  arbiter launches a new foreground app, send a hypothetical
  `BACKGROUND <pid>` command to the zygote, which translates
  to a lifecycle `Paused` event for the demoted app (uses the
  existing lifecycle infrastructure from task 33 step 5).
  Foreground app's SF surface is what the user sees; backgrounded
  apps continue running but don't get z-top.
- **InputFlinger focus.** Today each child requests focus
  every ~1 second (the task-33 hack). With an arbiter, only
  the foreground app should request focus. Quieter logs, no
  flapping.
- **OOM priority.** Arbiter writes `/proc/<pid>/oom_score_adj`
  on transitions. Foreground: 0. Background: 500-900 based
  on recency.

This step needs new zygote↔arbiter protocol commands. Add
to existing socket: `FOREGROUND <pid>`, `BACKGROUND <pid>`,
`STATUS` (returns list of child pids + foreground status).

Success criterion: launching two apps via the arbiter, switching
foreground between them via `wart-arbiter switch <app-id>`,
shows visual swap on screen + lifecycle Paused/Resumed events
in logcat, with InputFlinger focus following the foreground.

#### Step 4 — DEFERRED behind step 5 (decision 2026-05-27)

Step 4's real-z-order goal requires shim entry points
(`SurfaceControl::setLayer` at runtime) that the current
`libsf_surface.so` doesn't expose. Building the new shim
needs the AOSP a-03 build host
([[project-boot-model-libgui-build]]) which isn't available
here. Rather than ship "step 4 minus visual z-order" (lifecycle
+ OOM only — half the user-visible value), this step pauses
behind step 5's source-side prep work and resumes once the new
`.so` is on-device.

### Step 5 — Production deployment polish (~1 week)

The "make it real on the device" step. Some of this is
deferred per its own complexity.

- **init.rc service definitions.** `wart_zygote` and
  `wart_arbiter` services on production builds. Requires
  AOSP-build access (per [[project-boot-model-libgui-build]]
  this needs the a-03 build host).
- **SELinux policy.** New domains `wart_zygote`, `wart_arbiter`.
  Policy for: read of `/data/wart/apps/...`, write of
  `/proc/<pid>/oom_score_adj`, socket connect/accept,
  fork/exec. Same caveat re: build host.
- **`scripts/build-system-warpkgs.sh`** — automate the one-shot
  packaging task-45 step 4 did by hand. Builds markdown,
  emoji, fonts system bundles + wart-app warpkg, pushes,
  installs.
- **`scripts/run-hybrid-stack.sh`** — dev convenience that
  starts wart-zygote + wart-arbiter, like task-33's
  `standalone-launch.sh` but for the Hybrid model.
- **Crash-marker / lifecycle plumbing.** Carry over from task
  33 step 5 — arbiter logs which apps crashed, the next
  arbiter restart reports them.

The init.rc + sepolicy parts are gated on the a-03 build host
and will land separately. Steps 1-4 are buildable on the
regular dev machine.

Success criterion: full launch+switch flow works from
`scripts/run-hybrid-stack.sh` on the regular dev machine; the
init.rc / sepolicy work blocks-on-host stays as a known
deferred task.

#### Step 5 partial — source-side ready (2026-05-27)

Per the decision to land step 5 first (since step 4 needs shim
entry points), this commit ships everything in step 5 that's
buildable from a regular dev machine. The AOSP-a-03-host parts
(init.rc, sepolicy, shim `.so` rebuild) remain as TODOs.

**What landed (source-side, ready for rebuild + deploy):**

- **`cpp/sf_surface.{cpp,h}` — two new entry points**:
  ```c
  int32_t sf_set_layer(int32_t z);     // Transaction::setLayer
  int32_t sf_set_visible(int32_t visible); // Transaction::show/hide
  ```
  Both wrap `SurfaceComposerClient::Transaction` calls on
  `g_control`, apply async, return 0 on success / -1 if the
  surface is down. Hidden+shown layers keep their last BBQ
  frame — cheaper than re-creating the surface for
  background → foreground.

- **`src/sf_surface.rs` — Rust bindings**:
  - Two new `dlsym`-loaded function pointers stored in
    `SfSurface` as `Option<SetLayerFn>` / `Option<SetVisibleFn>`.
    `Option` because the field is `None` until the .so is
    rebuilt on the a-03 host (graceful degrade: arbiter then
    falls back to lifecycle + OOM with no visual z-order).
  - Public methods `SfSurface::set_layer(z: i32) -> bool` and
    `SfSurface::set_visible(visible: bool) -> bool` that return
    `false` when the shim is too old.

  Build verified clean on aarch64-android. Dead-code warnings
  on `set_layer`/`set_visible` are expected — step 4 will
  consume them.

- **`scripts/build-system-warpkgs.sh` (new)** — automates the
  task-45-step-4 manual packaging:
  ```
  $ scripts/build-system-warpkgs.sh
  ```
  Builds `markdown-renderer`, `emoji-picker`, `system-fonts`
  for wasm32-wasip2; packages each as a `.warpkg`; packages
  wart-app from `/tmp/skiko-component.wasm` (Kotlin pipeline
  expected to have produced this); pushes all four; installs
  via `wart-host --install` under `$APPS_ROOT`
  (default `/data/local/tmp/wart-apps`).
  Override `APPS_ROOT=/data/wart` for production layout.

- **`scripts/run-hybrid-stack.sh`** — shipped in step 3, so
  this step 5 item is already done.

**What's still TODO (blocks-on-a-03-host):**

- Rebuild `libsf_surface.so` against the AOSP a-03 tree so
  the new `sf_set_layer` / `sf_set_visible` symbols are
  actually present at dlsym time. Push to
  `/data/local/tmp/libsf_surface.so`. Verify with
  `nm -D libsf_surface.so | grep sf_set_`.
- `init.rc` service definitions for `wart_zygote` +
  `wart_arbiter` under `/system/etc/init/`.
- SELinux policy: domains `wart_zygote` + `wart_arbiter`,
  rules for `/data/wart/apps/...` read,
  `/proc/<pid>/oom_score_adj` write, socket bind/accept,
  fork/exec.
- Crash-marker plumbing for the arbiter (carry over from
  task 33 step 5 — currently arbiter is rebuilt-from-scratch
  every restart with empty state; production wants
  arbiter-side persistence of last-known-running apps for
  crash-detection logging).

When you next have the AOSP a-03 build host:

1. Build `libsf_surface.so` from `wart-host/cpp/sf_surface.{cpp,bp}`
   (the `.bp` is unchanged; just rebuild against the new `.cpp`).
2. `adb push libsf_surface.so /data/local/tmp/libsf_surface.so`.
3. Verify symbols: `adb shell 'nm -D /data/local/tmp/libsf_surface.so | grep sf_set_'`.
4. Step 4 unblocks. Land the arbiter policy: signal-driven
   role flips, `set_layer`/`set_visible` from the child render
   loop, OOM priority writes from the arbiter, focus throttle.

#### Shim rebuild done (2026-05-27)

Rebuilt `libsf_surface.so` on a-03 against the new `.cpp` using
the direct ninja path (much faster than `m libsf_surface`):

```
$ ssh harry@a-03
$ cd ~/android/lineage
$ source build/envsetup.sh && lunch aosp_arm64-trunk_staging-userdebug
$ prebuilts/build-tools/linux-x86/bin/ninja \
    -f out/combined-aosp_arm64.ninja \
    out/soong/.intermediates/external/sf_surface/libsf_surface/android_arm64_armv8-a_shared/libsf_surface.so
real    0m43.123s
```

(`m libsf_surface` would have re-run soong + ninja from scratch
— minutes. The direct-ninja path skips soong regeneration when
nothing in the bp/manifest layer changed; only the .cpp diff
matters.)

llvm-readelf confirmed both new symbols exported:

```
sf_set_layer    (FUNC GLOBAL DEFAULT)
sf_set_visible  (FUNC GLOBAL DEFAULT)
```

Pushed `.so` to device + rebuilt wart-host with a new dlsym
diagnostic line (commit `8f9e8e9` in wart-host). LAUNCH_GUI a
fresh wart-app, logcat now shows:

```
sf_surface: dlsym summary — input_poll=true query_hint=true
            request_focus=true set_layer=true set_visible=true
```

All five optional symbols resolved. Existing render path
unaffected — wart-app renders identically. The new entry points
aren't called yet (step 4 consumes them) but they're live and
ready.

`.so` stashed at `wart-host/cpp/build/libsf_surface.so` (the
path `scripts/standalone-launch.sh` / `scripts/run-hybrid-stack.sh`
push from).

**Step 4 now unblocked.** Remaining blockers for full step-5
production-polish: init.rc service definitions + SELinux
domains (both still need the a-03 host but are separable from
shim work).

## Known unknowns

- **Does `Component::deserialize_file` produce COW-safe state?**
  `Arc<...>` internals should fork-share cleanly (Rust's Arc is
  just refcounted, no thread state). But wasmtime might have
  internal caches or interior mutability we don't see. Worst
  case: the parent's deserialize gets thrown away in the child
  and each child redeserializes. Step 2 measures this.
- **SF z-order with multiple non-Activity surfaces.** Task-33
  empirically observed "latest allocated wins." We need to
  CHANGE z-order at runtime to swap foreground. Possible APIs:
  `SurfaceControl::setLayer(int z)` via the libgui shim
  (probably needs a new shim entry point). If not directly
  possible from the libgui shim, fallback: hide the demoted
  surface (visibility=false) and show the new one. Visually
  identical to z-order from the user's perspective.
- **InputFlinger arbitration**. The existing
  `setInputWindowInfo` call from `cpp/sf_surface.cpp` registers
  a window; multiple registrations from different processes
  should each get their own focus slot. Verify in step 4.
- **wasm guest fork-safety.** Children's wasm linear memory is
  per-Store; fork() is before Store creation. But if any
  zygote-time preload work creates a Store (it shouldn't),
  that's UAF territory. Audit in step 2.

## File-touch map (anticipated)

- `wart-host/src/zygote.rs` — SIGCHLD reaper, KILL command,
  preload registry hookup.
- `wart-host/src/app_loader.rs` — preload registry lookup
  before deserialize.
- `wart-host/src/main.rs` — `--preload <app-id>` CLI.
- `wart-arbiter/` (new crate) — full skeleton + policy.
- `Cargo.toml` (workspace root, if it exists; else
  `wart-host/Cargo.toml` extended with workspace).
- `scripts/build-host-android.sh` — extend to build both
  binaries.
- `scripts/build-system-warpkgs.sh` (new).
- `scripts/run-hybrid-stack.sh` (new).
- `tasks/46-wart-arbiter-mvp.md` — this doc; update per-step.
- `CLAUDE.md` — status table row when started; close-out row
  on completion.

## Resume hints for fresh sessions

1. `cat .task-state` — TASK=46 STEP=N tells you where to pick
   up.
2. Read `tasks/45-wart-zygote-spike.md` "What we learned" and
   "Recommended task 46 scope" sections first. The whole point
   of this task is operationalizing that recommendation.
3. The big architectural decision (two binaries, D1) is locked
   from a user discussion; don't revisit unless something
   forces it.
4. Step order is load-bearing for measurement: step 2 (preload
   registry) should finish before step 4 (real arbiter) so the
   end-to-end smoke shows the production-target COW numbers,
   not the MVP numbers.

## Related

- `tasks/45-wart-zygote-spike.md` — the spike this task
  productionizes.
- `MEMORY.md` → [[project-app-lifecycle-and-packaging]] —
  the locked §9 architecture this task implements one slice
  of.
- `MEMORY.md` → [[wart-zygote-fork-survival]] — empirical
  baseline of what's COW-safe / what's not.
- `tasks/33-boot-model-bringup.md` — the standalone path the
  zygote's forked children re-use; lifecycle / signal handling
  carries directly.
- `tasks/35-app-install.md` + `tasks/36-cross-app-deps.md` —
  the installer + dep wiring the arbiter consumes.
- `post-art-roadmap.md` §11 — the boot-model migration this
  feeds into.
