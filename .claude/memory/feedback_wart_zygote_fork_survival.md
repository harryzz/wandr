---
name: feedback-wart-zygote-fork-survival
description: "Empirical fork() survival of wart's full native stack on Pixel 2 XL — what's safe to share parent→child via COW, what must first-init in the child, and where the COW savings actually come from"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 0d7555bf-c89f-4a03-a1f7-af183b8bb90f
---

Tested 2026-05-27 in task 45 (wart-zygote spike, commits ad82c11
+ 353f690 + 1c5a6927 + 462d53a5). Native Rust zygote that
preloads `wasmtime::Engine` and `fork()`s on each LAUNCH request.
Run on Pixel 2 XL (Adreno 540, Android 15) device-verified.

**Why:** the §9 Hybrid runtime model
([[project-app-lifecycle-and-packaging]]) is the production
direction. Spiking the technical path before committing to it
answered the "what breaks across fork" question empirically.

**How to apply:** when planning future zygote-shaped work (the
real wart-arbiter spin-out / task 46), trust these empirical
findings rather than re-investigating from scratch. When stuck
on fork-related debugging, this is the baseline of "what
should work."

## What survived fork() with no special handling

- **Adreno EGL** (the keystone unknown). When the parent stays
  EGL-cold (D5 from the spike), the child first-inits EGL on
  a fresh ANativeWindow exactly like direct `--standalone`.
  No driver weirdness, no GPU context aliasing, no `libGLESv2_adreno.so`
  re-load issues.
- **wasmtime `Engine`** (post-construction). Pure functional
  state — no worker threads, no FDs, no signal handlers, no
  globals outside its own allocations. Safe to share read-only
  via COW.
- **SurfaceFlinger `SurfaceComposerClient`** — child can call
  `createSurface` fresh post-fork and get its own SurfaceControl
  independently. Multiple concurrent children, each with their
  own SF surface, all work.
- **The Rust standard library + Bionic stdlib + Tokio runtime
  (when not pre-spawned)** — no surprises. Children
  initialize what they need.
- **`OnceLock` / static initialization state** — COW'd as
  unset-or-set per its pre-fork value. Children with their
  own `OnceLock::get_or_init` paths re-initialize cleanly
  even if the OnceLock was inherited as "not yet initialized."

## What we deliberately did NOT do (D6/D7)

- **Don't init binder (rsbinder) in the parent.** Each child
  first-inits via `crate::binder::init` (which is an
  `OnceLock`-guarded `ProcessState::init_default`). If parent
  ever does init binder, the per-process binder FD and thread
  pool state would be COW'd into the child, leading to
  use-after-fork problems Bionic's `ProcessState::onFork` is
  designed to handle but rsbinder 0.8.0 doesn't expose.
- **Don't init EGL in the parent.** Even read-only EGL state
  is per-context and probably per-driver-instance; vendor
  drivers like Adreno are closed and not guaranteed
  fork-safe. Parent stays EGL-cold; child first-inits.
- **Don't spawn worker threads in the parent.** Tokio runtimes
  are spawned per-child if needed.

## Where the COW savings actually came from (surprise)

Of the 5 600 kB Shared_Dirty per child:

| Category | Size |
|---|---|
| Binary `.data/.bss` (wart-host + libs initialized globals) | 4 656 kB |
| `[anon:scudo:primary]` (wasmtime engine heap) | 208 kB |
| `[anon:linker]` (dynamic loader state) | 192 kB |
| other anon | 472 kB |
| stack + misc | 72 kB |

**`Engine::new` is structurally lightweight** — ~200 kB of heap.
The bulk of the "engine preload" win is the binary's
initialized statics, lazy-init'd globals, and Bionic linker
state that gets touched on first-load anyway.

**To meaningfully grow per-child COW savings**, in priority
order:

1. **`Component::deserialize_file`** in the parent. Each
   component adds several MB of `scudo:primary`. Largest
   single available win.
2. **Skia `FontMgr::default()` + touch a default typeface**
   in the parent. Several MB of font cache.
3. **`LoadedApp` (post-load, pre-instantiate)** — riskier;
   linker + dep wiring state. Might or might not be
   COW-safe; needs investigation.

Target with all three: ~25-40 MB COW per child. Beyond that
the parent's memory-write overhead at preload time has
diminishing returns.

## Per-app working set scaling (the real binding constraint)

Per concurrent app on this device at the current Compose
Material UI working set:

| Component | Size |
|---|---|
| wasm linear memory (Compose snapshot + composer trees) | ~100 MB |
| .cwasm mmap + read-only data | ~50 MB |
| Skia GPU buffer mirrors + canvas state | ~60 MB |
| **Total per app** | **~180 MB** |

The zygote's COW savings (~5 MB) are a rounding error vs
per-app working set. On 4 GB devices ~20 concurrent apps is
the memory ceiling; on 2 GB devices ~10 apps.

**Implication**: the Hybrid model's *memory* win over
monolithic-with-restart is small. The win is **per-process
crash isolation**, which monolithic can't provide. Same
lesson stock Android learned about Zygote.

## Performance numbers worth knowing

- Headless `wasi:cli/command` child via LAUNCH: ~25 ms from
  fork() to `call_run` returning Ok.
- GUI child via LAUNCH_GUI: ~600 ms from fork() to first
  rendered frame (50 ms for SF surface, 550 ms for first
  Compose layout + Skia warm-up). Both are slower than
  ideal; preload helps but Skia first-paint is per-context.
- Steady-state render: 60 fps per child, no inter-child
  contention at 2 concurrent. The single-threaded zygote
  accept loop serializes fork requests at ~1 Hz max which
  is fine for app launches.

## Known limitations carried into task 46+

- No SIGCHLD reaper in the zygote — zombies pile up. Trivial
  to add (~0.5 day) but not done at MVP.
- No shutdown command in the protocol — only external
  `kill -KILL`.
- Only one SF surface visible at a time (latest-allocated
  wins z-order). No arbiter policy. Touch input goes to
  whichever has InputDispatcher focus.
- The dev-cwasm fallback (`LAUNCH_GUI` with empty arg) is a
  smoke convenience; production uses installed app-ids.
- Preload registry not built — at MVP only the engine is
  preloaded; Component / Skia preload is deferred.

## Related

- [[project-app-lifecycle-and-packaging]] — the §9 Hybrid
  architectural decision this spike empirically validates.
- [[wasmtime-drc-no-autoschedule]] — one app's GC stall is
  isolated per-process in Hybrid (the main Hybrid win) but
  the underlying DRC issue is unchanged.
