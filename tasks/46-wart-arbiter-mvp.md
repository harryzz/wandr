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
