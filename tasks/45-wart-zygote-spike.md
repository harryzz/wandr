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
