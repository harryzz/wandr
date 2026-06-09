# Task 54 — Arbiter death notification + socket cleanup

> Status: ✅ device-verified 2026-05-29. Parts A + B + socket
> cleanup all shipped and exercised on the Pixel 2 XL.
>
> Closes the steady-state lifecycle gap discovered on device
> 2026-05-29: Android's lowmemorykiller killed wandr-app at
> `oom_score_adj=0` (foreground tier); arbiter never noticed
> because it isn't the parent of zygote-forked apps; orphaned
> control socket left the IME visually present but useless for
> character keys (taps reach the IME, internal cycling works,
> but `ime-send-key-event` writes vanish into a dead-pid socket).
>
> ## Results (2026-05-29)
>
> Implemented exactly the "A primary + B backstop + socket cleanup"
> shape below. Five touch points:
> - `runtime/wandr-host/src/zygote.rs` — `exit_subscribers()` registry +
>   `SUBSCRIBE_EXITS` command (moves the stream out of `handle_one` so
>   it stays open) + `broadcast_exit()` from the SIGCHLD reaper +
>   `sweep_stale_host_sockets()` at startup.
> - `runtime/wandr-arbiter/src/zygote_client.rs` — `subscribe_exits()`
>   (long-lived; unbuffered ack read so no `EXITED` line is buffer-stolen).
> - `runtime/wandr-arbiter/src/main.rs` — `arbiter_lock()` coarse mutex
>   (held by `handle_client` + `handle_child_exit`), `handle_child_exit()`
>   (reuses `demote_from_overlay()` for the overlay-teardown signal
>   cascade + `state::remove()` for state teardown + `remove_host_socket()`),
>   subscriber thread + 5 s polling-backstop thread.
> - `runtime/wandr-arbiter/src/state.rs` — `pid_alive()` made `pub`.
> - `runtime/wandr-host/src/standalone.rs` — graceful-exit unlink of the
>   per-host control socket on the shutdown-signal break.
>
> **Device verification** (full LMK repro): launched
> `com.example.wandr-app` (pid 15077, fg) + `wandr.ime.keyboard`
> (pid 15157, overlay) + `set-ime` + `attach-editor` to engage the
> split (list showed IME `[fg] [ime]`, app `[editor:text]`). Then
> `kill -9 15077`. Logcat (all within the same millisecond, on the
> subscriber thread — NOT the 5 s poller):
> ```
> wandr-zygote/reaper: pid 15077 reaped (signal=9, tracked=true)
> wandr-zygote: broadcast EXITED 15077 signal=9 to 1 subscriber(s)
> arbiter: on_child_exit pid=15077 app=com.example.wandr-app (signal=9)
> arbiter: overlay-clearing ime_pid=15157 behind_pid=15077
> arbiter: on_child_exit pid=15077 tore down overlay split (cleared=true)
> arbiter: removed orphaned host socket /data/local/tmp/wandr-host-15077.sock
> arbiter: on_child_exit pid=15077 app=com.example.wandr-app — cleaned up
> ```
> Post-kill `list` dropped wandr-app and the IME lost its `[fg]` marker
> (overlay torn down → ghost-keyboard scenario resolved); the app's
> `.sock` was gone. Startup sweep separately removed **59** accumulated
> stale `wandr-host-*.sock` files (the literal cruft from the buggy soak).
> Subscription established at daemon start (`new exit-subscriber (1 total)`).

## Why now

Empirical from a 36-hour Hybrid-stack soak (2026-05-29):

```
[129767.305329] lowmemorykiller: Killing 'wandr-host' (29087) (tgid 29087), adj 0,
[129767.305692] lowmemorykiller: Killing 'binder:29087_6' (29144) (tgid 29087), adj 0,
```

- wandr-app PID 29087 (`oom_score_adj=0`, the foreground tier
  set by `wandr-arbiter foreground`) was killed by the kernel
  LMK. The IME (29133) survived because its working set is
  smaller (~80 MB vs ~180 MB for wandr-app with Compose +
  skiko + cards + assets); LMK tie-breaks by RSS.
- The arbiter's `list` still showed `app=com.example.wandr-app
  pid=29087 [fg]` 36 hours after the kill. No SIGCHLD reach,
  no liveness probe.
- IME's render loop healthy (50 fps, frames 1294200 → 1301400).
  Synthetic tap on 🌐 still cycled languages correctly.
- Character keys silently disappeared — arbiter would write
  `ime-send-key-event` to
  `/data/local/tmp/wandr-host-29087.sock` (zombie file from the
  pre-death listener; unlinked? no), connection refused, log
  `"deliver-failed"`, no cleanup.

**User-visible**: ghost keyboard overlaid on the launcher; tap
keys → nothing visible happens; the only escape is the IME's ⌄
hide key.

## Root causes (two)

### RC1: arbiter can't observe zygote-forked deaths

Process tree:
```
pid 29052 wandr-host --zygote    PPID=1
pid 29071 wandr-arbiter --daemon PPID=1
pid 29133 wandr-host (IME)       PPID=29052   ← zygote is parent
pid 29087 wandr-host (wandr-app)  PPID=29052   ← zygote is parent (now dead)
```

The arbiter installs a SIGCHLD reaper per task 46 step 1, but
SIGCHLD is delivered to the **parent**, which is the zygote, not
the arbiter. The arbiter only sees deaths via `kill(pid, 0)`
liveness probe at **crash-recovery startup** (task 46 crash-
marker work), NOT during steady state.

Result: a process can disappear (LMK, OOM, SEGV, …) and the
arbiter keeps thinking it's running forever.

### RC2: control sockets not unlinked on child exit

`wandr-host` children bind `/data/local/tmp/wandr-host-<pid>.sock`
in `ime_inbound::start_listener`. When the process exits
(normal or SIGKILL), the socket inode is removed by the kernel
but the **file path** in the filesystem persists. A subsequent
`connect(2)` to it succeeds in `socket(2)` + `bind(2)` discovery
but fails the actual connection (`ECONNREFUSED`). The arbiter
treats this as a transient failure rather than a definitive
dead-pid signal.

Accumulated artifact: 50+ stale `wandr-host-<old-pid>.sock` files
in `/data/local/tmp/` from previous sessions. (Mostly cosmetic,
but noisy in `dumpsys input` and `ls`.)

## Fix shape

Two coordinated changes that close both gaps.

### Part A — Zygote → arbiter death notification

The zygote DOES receive SIGCHLD for every forked child (it
already reaps them per `wandr-host/src/zygote.rs`). Pipe that
into a notification the arbiter consumes.

Cleanest design: extend the existing zygote socket protocol
(`/data/local/tmp/wandr-zygote.sock`).

**Option A — push notifications on a long-lived connection:**

- Arbiter opens a long-lived connection to the zygote socket
  with a new `SUBSCRIBE_EXITS\n` request (or implicit on first
  connection from PID matching the arbiter's pid).
- Zygote SIGCHLD handler reaps the child, then writes
  `EXITED <pid> <signal>\n` to every active subscriber.
- Arbiter's reader thread parses each line, calls a new
  `state::on_child_exit(pid, signal)` that:
  - Removes the entry from `running_apps`.
  - If `pid == focused_pid`, clears `focused_pid`, signals the
    IME to demote (existing `cmd_overlay_clear` path).
  - If `pid == ime_pid`, clears `active_ime` + signals any app
    in `OverlayBehind` to return to `Foreground`.
  - Persists the new state to `wandr-arbiter-state.json`.

Pros: clean event-driven, no polling, fits the existing
SIGCHLD reaper, no kernel mechanism beyond what already
exists. Cons: requires a long-lived connection (small new
state); reconnect-on-fault handling.

**Option B — polling fallback:**

- Arbiter spawns a thread that wakes every 5 s and runs
  `kill(pid, 0)` against every entry in `running_apps`.
- Same `on_child_exit` handler.

Pros: zero changes to zygote; simple. Cons: 5 s window where a
dead app appears alive; wastes a syscall per app per tick.

**Recommendation**: A as primary; B as backstop (also covers
the "zygote crashed mid-session" case). Both are small —
~150 LoC combined.

### Part B — Socket unlink on child exit

Each `wandr-host` child unlinks its `.sock` file at shutdown:

```rust
// runtime/wandr-host/src/ime_inbound.rs::start_listener
let path = format!("/data/local/tmp/wandr-host-{}.sock", pid);
let listener = UnixListener::bind(&path)?;

// Register cleanup. Drop runs on graceful shutdown
// (lifecycle_standalone::should_shutdown loop break) AND on
// panic via the existing panic hook.
let _guard = scopeguard::guard(path.clone(), |p| {
    let _ = std::fs::remove_file(&p);
});
```

Plus a startup sweep: `wandr-host --zygote` deletes any
`wandr-host-*.sock` whose `<pid>` is not in the current process
list. Runs once at zygote startup so accumulated stale files
get cleaned.

Pros: defensive against SIGKILL paths (drop won't run on
SIGKILL, but startup sweep covers that on next zygote run).
Cons: still doesn't help during SIGKILL within a session, but
Part A's death notification means arbiter learns the pid is
dead before it tries to write to the socket.

## Steps

| # | Step | Estimate |
|---|---|---|
| 1 | Extend zygote socket protocol — `SUBSCRIBE_EXITS` request, broadcast `EXITED <pid> <signal>` to subscribers from SIGCHLD reaper. `runtime/wandr-host/src/zygote.rs`. | ~1 h |
| 2 | Arbiter subscriber thread — opens connection on daemon startup, reconnects on disconnect (with backoff), feeds events into existing state-mutation path. `runtime/wandr-arbiter/src/main.rs` + `zygote_client.rs`. | ~45 min |
| 3 | `state::on_child_exit(pid, signal)` — removes from running map; cascades to focused-pid / active-ime cleanup; signals IME demote if needed; persists state. `runtime/wandr-arbiter/src/state.rs`. | ~45 min |
| 4 | Polling backstop (5 s `kill(pid, 0)`) — separate thread, same `on_child_exit` callback. | ~20 min |
| 5 | Socket unlink — RAII guard in `ime_inbound::start_listener` + startup sweep in `zygote::serve`. `runtime/wandr-host/src/{ime_inbound,zygote}.rs`. | ~30 min |
| 6 | Device verification — reproduce the OOM-kill scenario (`adb shell stop` then trigger memory pressure, OR `am send-trim-memory` if available, OR just `kill -9 <wandr-app-pid>` from `su`), confirm: (a) arbiter `list` updates within 5 s, (b) IME demotes / clears editor focus, (c) `.sock` file removed on graceful exit and surviving stale files swept at next zygote start. | ~30 min |
| 7 | Memory `feedback_arbiter_death_notification.md` + close-out. | ~15 min |

## Verification recipe

Repro the LMK scenario without waiting for natural memory pressure:

```
# 1. Launch the stack
bash tools/scripts/run-hybrid-stack.sh
adb shell "su -c '/data/local/tmp/wandr-arbiter launch com.example.wandr-app'"
adb shell "su -c '/data/local/tmp/wandr-arbiter launch-overlay wandr.ime.keyboard'"
adb shell "su -c '/data/local/tmp/wandr-arbiter set-ime wandr.ime.keyboard'"

# 2. Simulate LMK
WANDR_APP_PID=$(adb shell "ps -ef | grep wandr-host | awk 'NR==3{print \$2}'")
adb shell "su -c 'kill -9 $WANDR_APP_PID'"

# 3. Within 5 s arbiter list should drop wandr-app + clear fg
adb shell "su -c '/data/local/tmp/wandr-arbiter list'"
# Expected:
#   OK count=1
#     app=wandr.ime.keyboard pid=<x> elapsed_ms=… [ime]

# 4. logcat verifies cascading actions
adb logcat -d | grep -E 'arbiter.*EXITED|on_child_exit|demote|sock removed'
```

## Out of scope

- Restart-on-crash policy (auto-relaunch wandr-app if killed) —
  separate decision; current behavior is "user re-launches".
- LMK tuning (adjust `oom_score_adj` for foreground IME vs app
  vs background) — orthogonal; current adj=0 for IME + app may
  not be the right shape if we want LMK to prefer killing IME
  over the user-facing app.
- `oom_score_adj_min` reservation for the zygote + arbiter
  (they're at adj=0 today; if LMK got really aggressive it
  could kill the runtime infrastructure itself).

## Related

- [`tasks/45-wandr-zygote-spike.md`](45-wandr-zygote-spike.md) —
  the zygote design that established zygote-as-parent.
- [`tasks/46-wandr-arbiter-mvp.md`](46-wandr-arbiter-mvp.md) —
  the SIGCHLD reaper + crash-marker work that this task
  closes the steady-state gap of.
- [`docs/architecture-runtime.md`](../docs/architecture-runtime.md)
  — three-socket protocol + signal table; gets a new section
  on death notification once this lands.
