# Architecture: the wandr runtime (zygote / arbiter / host)

This doc explains the three-process Hybrid runtime that boots wandr apps
on a wandrified Android: **wandr-host --zygote** (component preloader +
fork server), **wandr-arbiter --daemon** (policy + lifecycle), and the
per-app **wandr-host** child processes. It catalogues every transport
(three UNIX sockets) and every signal in the protocol, and traces what
happens from `wandr-arbiter launch <app>` through to a Compose UI on
SurfaceFlinger.

Companion to:
- [`architecture-host-guest-boundary.md`](architecture-host-guest-boundary.md)
  — what crosses a host↔guest WIT call.
- [`architecture-ime.md`](architecture-ime.md) — the IME, which is the
  most intricate user of this infrastructure.

Background: [task 45](../tasks/45-wandr-zygote-spike.md) (zygote spike) +
[task 46](../tasks/46-wandr-arbiter-mvp.md) (arbiter MVP).

## TL;DR

- **wandr-host --zygote** is a long-lived parent that preloads
  `wasmtime::Engine` + a registry of precompiled `.cwasm` components.
  Forks on each `LAUNCH` request; children inherit the engine via COW.
- **wandr-arbiter --daemon** is a sibling policy daemon. The user's
  CLI (`wandr-arbiter launch …`, `set-ime`, `foreground`, `kill`) all
  bottom out as text commands on its socket. It asks the zygote to
  fork, then signals + sockets the child to push role / IME / focus
  state.
- **wandr-host children** are the actual app processes. Each owns
  one `wasmtime::Store`, one `SurfaceFlinger` surface, one EGL
  context, one Compose render loop. The arbiter pushes inbound
  events to each child over a per-host control socket.
- **Three sockets**:
  - `/data/local/tmp/wandr-zygote.sock` — arbiter → zygote (fork
    requests + component preload registry).
  - `/data/local/tmp/wandr-arbiter.sock` — user CLI + host children
    → arbiter (policy commands + outbound IME routing).
  - `/data/local/tmp/wandr-host-<pid>.sock` — arbiter → host child
    (inbound events: editor focus, key events).
- **Three signals**:
  - `SIGUSR2` → child becomes **Foreground** (z=MAX, visible,
    Resumed).
  - `SIGUSR1` → child becomes **Background** (z=0, hidden, Paused).
  - `SIGRTMIN+1` → child becomes **OverlayBehind** (z=0, visible,
    stays Resumed — used for the focused app while the IME is
    overlaid on top).

## Process layout

```
   ┌──────────────────────────────────────────────────────────────┐
   │                       Linux kernel                           │
   │                                                              │
   │   wandr-host --zygote                  wandr-arbiter --daemon  │
   │   (one process, long-lived)           (one process, sibling) │
   │   listens: /tmp/wandr-zygote.sock      listens:               │
   │   preloads: Engine + system-apps/*    /tmp/wandr-arbiter.sock │
   │       │                                  │                   │
   │       │  fork()                          │                   │
   │   ┌───┼───────┬─────────┬──────────┐     │                   │
   │   │   │       │         │          │     │                   │
   │   ▼   ▼       ▼         ▼          ▼     │                   │
   │  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐                 │
   │  │wandr-host│ │wandr-host│ │wandr-host│ │wandr-host│             │
   │  │ pid=A   │ │ pid=B   │ │ pid=C   │ │ pid=D   │             │
   │  │ app:    │ │ app:    │ │ app:    │ │ app:    │             │
   │  │ wandr-app│ │ ime.kbd │ │ md-cli  │ │ wandr-app│             │
   │  │   GUI   │ │ OVERLAY │ │ HEADLESS│ │   GUI   │             │
   │  │         │ │         │ │         │ │         │             │
   │  │ each binds /data/local/tmp/wandr-host-<pid>.sock            │
   │  └────┬───┘ └────┬────┘ └────┬───┘ └────┬───┘                │
   │       └────────────arbiter writes→──────┘                    │
   │                                                              │
   │  SurfaceFlinger ◄─── GUI children attach BBQ surfaces        │
   │  InputFlinger   ◄─── GUI children register input windows     │
   └──────────────────────────────────────────────────────────────┘
```

Children come in three kinds depending on which `LAUNCH*` command
the arbiter sent the zygote:

| kind            | zygote cmd               | host CLI flags                    | purpose                                            |
|-----------------|--------------------------|-----------------------------------|----------------------------------------------------|
| GUI (fullscreen)| `LAUNCH_GUI <app>`       | `--standalone --app <app>`        | Compose app owning a fullscreen SF surface         |
| GUI (overlay)   | `LAUNCH_GUI_OVERLAY <app>`| `--standalone-overlay --app <app>`| IME / future overlays — bottom-strip SF surface    |
| Headless        | `LAUNCH <app>`           | `--run-once <app>`                | One-shot `wasi:cli/command` consumer; no surface   |

Each child binds its per-host control socket, sets up its
`wasmtime::Store` (inheriting the zygote-preloaded engine), and
either drops into a render loop (`standalone::run`) or runs
to completion (`run_once::run`).

## Socket #1 — zygote socket

**Path:** `/data/local/tmp/wandr-zygote.sock`
**Speaker:** `wandr-arbiter` (and a debug CLI form via
`wandr-host --zygote-client`)
**Listener:** `wandr-host --zygote`
**Format:** text, one command per line, reply one line.

| Request                              | Reply on success           | Purpose                                                                 |
|--------------------------------------|----------------------------|-------------------------------------------------------------------------|
| `LAUNCH <app-id>`                    | `OK <child-pid>`           | Fork a headless `wasi:cli/command` child                                |
| `LAUNCH_GUI [<app-id>]`              | `OK <child-pid>`           | Fork a fullscreen Compose child (`--app` optional, defaults to wandr-app)|
| `LAUNCH_GUI_OVERLAY <app-id>`        | `OK <child-pid>`           | Task 47 step 3c — fork an IME-shaped overlay-surface child              |
| `PRELOAD <app-id>`                   | `OK preloaded` / `OK cached`| Add `.cwasm` to the zygote's deserialized registry (task 46 step 2)     |
| `KILL <pid>`                         | `OK killed` / `ERR …`      | Reap a known child (refuses non-children)                               |
| `SUBSCRIBE_EXITS`                    | `OK subscribed` (then push)| Task 54 — long-lived connection; zygote pushes `EXITED <pid> <summary>` from the reaper for every child death |

On `LAUNCH*`:
1. Zygote parent `fork()`s.
2. Parent writes `OK <child-pid>\n` back to the arbiter.
3. Child resets signal handlers, re-execs `wandr-host` with the
   right CLI flags (or in-place re-invokes the standalone /
   run_once entry point — see `wandr-host/src/zygote.rs` for the
   exec-vs-in-place choice and why).

On `PRELOAD <app-id>`:
1. Zygote parent `Engine::deserialize_file(...)` the app's
   precompiled `.cwasm`.
2. The resulting `Component` is held in a global registry keyed
   by app-id.
3. Forked children inherit the registry via COW; the next
   `LAUNCH <same-app>` skips the deserialize.

The zygote auto-preloads everything under
`<APPS_ROOT>/system-apps/` at startup. The arbiter explicitly
preloads frequently-used user apps on demand.

## Socket #2 — arbiter socket

**Path:** `/data/local/tmp/wandr-arbiter.sock`
**Speaker:** user CLI (`wandr-arbiter launch …`), host children
(for the IME-routing path)
**Listener:** `wandr-arbiter --daemon`
**Format:** text, one command per line, reply one line.

| Command (line)                                  | Reply                              | Purpose                                                                                       |
|-------------------------------------------------|------------------------------------|-----------------------------------------------------------------------------------------------|
| `launch <app-id>`                               | `OK pid=<pid> app=<id>`            | Forward `LAUNCH_GUI` to zygote, record pid, auto-promote to Foreground                        |
| `launch-overlay <app-id>`                       | `OK pid=<pid> app=<id>`            | Forward `LAUNCH_GUI_OVERLAY` to zygote (used for the IME)                                     |
| `launch-headless <app-id>`                      | `OK pid=<pid> app=<id>`            | Forward `LAUNCH` to zygote                                                                    |
| `kill <app-id>`                                 | `OK killed app=<id> pid=<pid>`     | Send SIGTERM, reap, drop from running-apps map                                                |
| `list`                                          | `OK count=N …`                     | Dump running-apps map + roles + IME / editor-focus state                                      |
| `preload <app-id>`                              | `OK preloaded` / `OK cached`       | Forward `PRELOAD` to zygote                                                                   |
| `foreground <app-id>`                           | `OK fg=<id>`                       | Signal that app to Foreground (SIGUSR2), prior fg to Background (SIGUSR1)                     |
| `overlay <app-id>`                              | `OK overlay=<id> behind=<id>`      | Signal app to Foreground, prior fg to OverlayBehind (SIGRTMIN+1)                              |
| `set-ime <app-id>`                              | `OK ime=<id>`                      | Mark `<app-id>` as the active IME (one slot — future polish: pick-list)                       |
| `set-ime -`                                     | `OK ime=(cleared)`                 | Clear the active-IME slot                                                                     |
| `attach-editor <focused-pid> <input-type>`      | `OK delivered`                     | Sent by host children when a TextField focuses; auto-promotes IME to overlay over caller pid  |
| `detach-editor <focused-pid>`                   | `OK delivered`                     | Paired with attach; auto-clears the overlay                                                   |
| `ime-send-key-event <code-point> <key-id> <act>`| `OK delivered`                     | Sent by IME host child; arbiter routes to the focused-app's control socket as `key-event …`   |

The arbiter persists its running-apps map + foreground +
active-IME to `/data/local/tmp/wandr-arbiter-state.json` after
every command. On restart it re-attaches surviving children
via `kill(pid, 0)` liveness probes (task 46 crash-marker work).

## Socket #3 — per-host control socket

**Path:** `/data/local/tmp/wandr-host-<pid>.sock` (one per host
child)
**Speaker:** `wandr-arbiter`
**Listener:** the host child's `ime_inbound` module
**Format:** text, one command per line, no reply (fire-and-forget;
the arbiter has already replied to its own caller).

| Command (line)                                                | Becomes `InboundEvent`                | When                                                                |
|---------------------------------------------------------------|---------------------------------------|---------------------------------------------------------------------|
| `key-event <code-point> <key-id> <action>`                    | `KeyEvent { code_point, key_id, action }` | IME tapped a key → arbiter routed to the focused app                |
| `editor-attached <input-type> <hint> <initial-text>`          | `EditorAttached { info }`             | Some app focused a TextField; this is the IME child receiving notice|
| `editor-detached`                                             | `EditorDetached`                      | Editor lost focus; IME child auto-demoted                           |

Each host child's render loop drains its `ime_inbound` queue
once per frame (see `wandr-host/src/standalone.rs`) and dispatches:

- `KeyEvent` → `dispatch_key_v2(skiko, store, action, cp, kid)` —
  becomes a Compose `KeyEvent` in the focused app.
- `EditorAttached` / `Detached` → the IME guest's exported
  `wandr:ime/ime.on-editor-attached(input-type)` /
  `on-editor-detached()` — the IME app picks the matching layout
  (task 49 step 1b).

## Signal protocol

Children install three signal handlers in `wandr-host/src/app_role.rs`,
backed by a single `AtomicI32` `ROLE` they observe once per frame
in the render loop.

| Signal       | New role         | Render-loop reaction                                                          |
|--------------|------------------|-------------------------------------------------------------------------------|
| `SIGUSR2`    | Foreground       | `sf_set_layer(MAX)` + `sf_set_visible(true)` + `sf_request_focus()` + lifecycle Resumed |
| `SIGUSR1`    | Background       | `sf_set_layer(0)` + `sf_set_visible(false)` + lifecycle Paused                |
| `SIGRTMIN+1` | OverlayBehind    | `sf_set_layer(0)` + `sf_set_visible(true)` + lifecycle stays Resumed          |

`SIGRTMIN+1` exists for the editor-focused-app case: when the
arbiter promotes the IME to Foreground (SIGUSR2), it doesn't want
to *Pause* the focused app — the cursor needs to keep blinking,
text needs to keep mutating. OverlayBehind keeps the app rendering
+ visible, but at z=0 so the IME's overlay surface composites on
top.

Children also install three lifecycle signals (`SIGTERM`,
`SIGINT`, `SIGHUP`) that flip an atomic shutdown flag — the
render loop breaks, fires `Destroyed`, drains 3 frames, exits
cleanly. See `wandr-host/src/lifecycle_standalone.rs`.

The **zygote** installs `SIGCHLD` for child reaping (task 46 step 1)
— it is the `fork()` parent of every app, so it is the only process
the kernel notifies of an app death. The arbiter is a *sibling*, not
the parent, so it never receives those `SIGCHLD`s directly — see
**Death notification** below for how it learns of deaths.

## Death notification (task 54)

Because the zygote — not the arbiter — is the parent of every app
child, an app dying (LMK, OOM, SIGSEGV, clean exit) only wakes the
zygote's reaper. Without a bridge, the arbiter's running-apps map
goes stale forever (observed: a 36-hour soak still listed an
LMK-killed app as `[fg]`), and the dead app's per-host control
socket lingers so the IME's `ime-send-key-event` writes vanish into
a refused connection (ghost keyboard).

Two coordinated mechanisms close this:

1. **Event-driven push (primary).** The arbiter opens a long-lived
   `SUBSCRIBE_EXITS` connection to the zygote socket on daemon
   startup; the zygote moves that stream into an `exit_subscribers`
   list. Each time the reaper reaps a child it broadcasts
   `EXITED <pid> <exit-summary>` to every subscriber. The arbiter's
   subscriber thread parses each line and calls
   `handle_child_exit(pid, detail)`. Disconnected subscribers are
   dropped on the next failed write; the arbiter reconnects with a
   1 s backoff (so it survives a zygote restart).
2. **Polling backstop.** A second arbiter thread `kill(pid, 0)`-probes
   every tracked pid every 5 s and calls the same `handle_child_exit`
   for any that died. Covers a dropped subscriber link and the
   zygote-crashed-mid-session case.

`handle_child_exit` (under a coarse `arbiter_lock` shared with the
command path) reuses the existing teardown: if the dead pid was
either side of an IME overlay split it runs `demote_from_overlay`
(hides the IME, repromotes the survivor), then `state::remove`
(clears foreground / active-IME / editor-focus / overlay pointers),
then unlinks `/data/local/tmp/wandr-host-<pid>.sock`, then persists.

Per-host control sockets are also unlinked on the graceful-shutdown
path (`standalone.rs`), and the zygote sweeps any stale
`wandr-host-*.sock` whose pid is no longer alive at startup (the
SIGKILL/LMK case, where Drop never runs).

## Component preload registry

`tasks/46-wandr-arbiter-mvp.md` step 2. The zygote keeps a
process-global `HashMap<app-id, wasmtime::component::Component>`
of deserialized `.cwasm`s. The map is populated by:

1. **Startup auto-preload.** On zygote startup, scan
   `<APPS_ROOT>/system-apps/*/<version>/cache/*.cwasm` and
   `Engine::deserialize_file` each. ~30 ms per .cwasm × N system
   apps.
2. **On-demand `PRELOAD <app-id>`.** Arbiter can promote a user
   app into the preload set (e.g. `wandr-arbiter preload
   com.example.wandr-app` so the next launch skips deserialize).

Children inherit the registry via COW. When they need a
component, they first check the registry (O(1) hashmap lookup,
no I/O, no deserialize) — preloaded apps fork-to-rendering in
~25 ms vs ~120 ms cold.

Closes the COW gap to ~57 MB Shared_Dirty per render child
(measured task 46 step 2) — 10× the engine-only baseline.

## End-to-end: launching an app

What happens when the user types `wandr-arbiter launch
com.example.wandr-app`:

```
   wandr-arbiter CLI (one-shot client process)
     │
     │  connect /data/local/tmp/wandr-arbiter.sock
     │  send "launch com.example.wandr-app\n"
     ▼
   wandr-arbiter --daemon
     │  ── cmd_launch (main.rs:793) ──
     │  connect /data/local/tmp/wandr-zygote.sock
     │  send "LAUNCH_GUI com.example.wandr-app\n"
     ▼
   wandr-host --zygote
     │  ── handle_command (zygote.rs) ──
     │  fork(); parent writes "OK <child-pid>\n", child execs
     │  wandr-host --standalone --app com.example.wandr-app
     ▼
   wandr-host child (the new app)
     │  ── standalone::run ──
     │  1. binder::init
     │  2. SfSurface::create (libsf_surface.so dlopen)
     │     ↳ SurfaceComposerClient::createSurface
     │     ↳ BLASTBufferQueue + ANativeWindow ready
     │     ↳ register input window (IInputFlinger::createInputChannel)
     │  3. EGL context bound to ANativeWindow
     │  4. wasmtime::Store::new(preloaded-Engine)
     │  5. Linker + add_to_linker_sync(wasi-p2)
     │  6. load_dep_components — instantiate + wire deps
     │  7. linker.instantiate(component) — run guest's _start
     │  8. ime_inbound::start_listener — binds
     │     /data/local/tmp/wandr-host-<pid>.sock
     │  9. render loop (60 fps):
     │     a. observe role atomic; handle transitions
     │     b. take_pending_overlay_resize (IME only)
     │     c. screen_state watcher poll
     │     d. drain sf.poll_input → dispatch_pointer_v2
     │        / dispatch_android_key
     │     e. drain ime_inbound queue → dispatch_key_v2 /
     │        call_on_editor_attached
     │     f. call guest's renderFrame(nanos)
     │     g. EGL swap buffers
     │
   (back at arbiter, immediately after step 2's "OK <pid>"):
     │
     │  record app=com.example.wandr-app pid=<child-pid> in state
     │  if no current fg: send SIGUSR2 to child (auto-promote)
     │  write running-apps + foreground to wandr-arbiter-state.json
     │  reply "OK pid=<pid> app=com.example.wandr-app\n" to CLI
```

Total wall time on a Pixel 2 XL: ~120 ms cold, ~25 ms with the
component preloaded.

## Lifecycle + cleanup

| Trigger                          | What happens                                                                                                       |
|----------------------------------|--------------------------------------------------------------------------------------------------------------------|
| Child exits normally / crashes / LMK-killed | Zygote reaper reaps it and broadcasts `EXITED <pid>` to the arbiter (task 54); arbiter `handle_child_exit` removes it from the map, tears down any IME overlay split, unlinks the orphaned control socket, persists. A 5 s `kill(pid,0)` poller is the backstop. Crashes additionally drop `/data/local/tmp/wandr-host-crash.json`, logged on next launch. |
| User: `wandr-arbiter kill <id>`   | Arbiter sends SIGTERM, waits 1 s, escalates to SIGKILL; entry removed from map.                                    |
| Arbiter exits                    | Children survive (each is its own process); on next arbiter start, `kill(pid, 0)` liveness probe re-attaches them. |
| Zygote exits                     | Same — children survive but new launches fail until zygote restarts.                                               |
| Reboot                           | Magisk module re-spawns zygote + arbiter at `late_start_service` (task 46 step 5, `wandr-stack-magisk/`).           |
| User installs new app            | `wandr-host --install <wandrpkg>` (offline tool) → drops `.cwasm` + `package.toml` into `<APPS_ROOT>/apps/<id>/<v>/`; no daemon notify needed (next `launch <id>` picks it up). |

## Where things live in code

| File                                  | Role                                                                          |
|---------------------------------------|-------------------------------------------------------------------------------|
| `wandr-host/src/main.rs`               | CLI entry: `--zygote` / `--install` / `--standalone` / `--run-once` / etc.    |
| `wandr-host/src/zygote.rs`             | Zygote parent + fork path + LAUNCH/PRELOAD dispatcher                         |
| `wandr-host/src/standalone.rs`         | GUI child render loop                                                         |
| `wandr-host/src/run_once.rs`           | Headless child `Command::instantiate` + `wasi_cli_run.call_run` (task 36)     |
| `wandr-host/src/app_loader.rs`         | `LoadedApp` + `WandrLoader::load` + `wire_dep_into_linker` (task 35, 36, 39)   |
| `wandr-host/src/app_installer.rs`      | `wandr-host --install` — manifest parse + AOT-precompile + cache-key writer    |
| `wandr-host/src/app_role.rs`           | Foreground / Background / OverlayBehind + signal handlers                     |
| `wandr-host/src/lifecycle_standalone.rs`| SIGTERM/INT/HUP + screen-state watcher + crash-marker drain                  |
| `wandr-host/src/ime_inbound.rs`        | Per-host control socket listener + InboundEvent queue (task 47, 49)           |
| `wandr-host/src/keyboard_host_impl.rs` | `Keyboard.Import.sendKeyEvent` → arbiter routing                              |
| `wandr-host/src/ime_host_impl.rs`      | `Ime.Import.notifyEditorAttached/Detached` → arbiter routing                  |
| `wandr-host/src/sf_surface.rs`         | dlsym wrapper over libsf_surface.so                                           |
| `wandr-host/cpp/sf_surface.cpp`        | C++ shim — SurfaceComposerClient + InputFlinger + BLASTBufferQueue            |
| `wandr-arbiter/src/main.rs`            | Arbiter daemon — sockets + command dispatch + signal sends                    |
| `wandr-arbiter/src/state.rs`           | Persisted state — running apps, foreground, active IME, editor focus, overlay |
| `wandr-arbiter/src/zygote_client.rs`   | LAUNCH / LAUNCH_GUI / LAUNCH_GUI_OVERLAY / PRELOAD / KILL request helpers     |
| `wandr-stack-magisk/`                  | Magisk module that auto-starts zygote + arbiter (task 46 step 5)              |

## Why this shape

Three deliberate choices that shaped the design:

- **Hybrid runtime model (`post-art-roadmap.md` §9).** Fork-from-
  zygote, not a single multi-app process. Each app is its own
  PID + Store + EGL context → kernel-enforced isolation, no
  hostile-app-tanks-everyone failure mode, OOM-killer can pick a
  victim cleanly. The per-app working set (~180 MB) dominates the
  zygote's COW savings (~5 MB/child), so the win here is
  *isolation*, not memory. Task 45 spike confirmed empirically.
- **Arbiter as separate policy daemon, not folded into zygote.**
  Keeps the zygote's invariants minimal (just fork on request,
  preload components, reap on SIGCHLD). The arbiter is the only
  thing that knows about roles, focus, IME slots — i.e. anything
  that could break when policy changes. Task 46 step 3.
- **Signals + text sockets, not binder.** The whole arbiter↔host
  IPC fits in a few hundred bytes/sec of text. Binder would
  require AIDL codegen, sepolicy, and a service-manager
  registration none of which we get for free on a non-AOSP-
  blessed daemon. Three UNIX sockets + three signals do
  everything binder would, with `cat` + `strace` debuggability.

## Related

- [`tasks/45-wandr-zygote-spike.md`](../tasks/45-wandr-zygote-spike.md) — zygote design + fork-survival empirics.
- [`tasks/46-wandr-arbiter-mvp.md`](../tasks/46-wandr-arbiter-mvp.md) — arbiter MVP, role signalling, Magisk auto-start.
- [`tasks/47-ime-via-guest-app.md`](../tasks/47-ime-via-guest-app.md) — overlay surface, per-host IME socket, step 3c input-window fix.
- [`tasks/49-ime-content-control.md`](../tasks/49-ime-content-control.md) — editor-attach + key events flowing through the runtime.
- [[project-app-lifecycle-and-packaging]] — broader Hybrid §9 framing.
