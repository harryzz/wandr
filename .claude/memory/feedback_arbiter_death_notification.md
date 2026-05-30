---
name: feedback_arbiter_death_notification
description: "Task 54 — the zygote (not the arbiter) is the parent of app children, so the arbiter learns of deaths via a SUBSCRIBE_EXITS push bridge + a 5s poll backstop"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 5c0eb8cc-cdbc-4cfe-b6cf-7d5eb0c39607
---

Task 54 (device-verified 2026-05-29). In the Hybrid runtime the
**zygote** `fork()`s every app, so it is the only process the kernel
delivers `SIGCHLD` to. The **arbiter** is a *sibling*, not the parent —
it never sees app deaths directly. Before task 54 a dead app (LMK / OOM /
crash) stayed in the arbiter's running-apps map forever and its per-host
control socket (`/data/local/tmp/wart-host-<pid>.sock`) lingered, so the
IME's `ime-send-key-event` writes hit a refused connection → ghost
keyboard that eats keystrokes.

**How it works now:** the arbiter opens a long-lived `SUBSCRIBE_EXITS`
connection to the zygote socket; the zygote's SIGCHLD reaper broadcasts
`EXITED <pid> <summary>` to subscribers. The arbiter subscriber thread →
`handle_child_exit(pid, detail)` (under a coarse `arbiter_lock` shared
with the command path) → reuses `demote_from_overlay()` (overlay
teardown) + `state::remove()` + unlink the host socket + persist. A 5 s
`kill(pid,0)` poller is the backstop (dropped subscriber / crashed
zygote). The zygote also sweeps stale `wart-host-*.sock` at startup
(SIGKILL skips Drop) — cleaned 59 accumulated files on first run.

**Why:** correctness gap in steady-state lifecycle; the existing
`kill(pid,0)` liveness probe only ran at arbiter crash-recovery startup
(task 46 crash-marker), never live.

**How to apply:** when adding any per-app resource the arbiter tracks,
clean it up in `handle_child_exit` (the single death entry point), NOT
just in the `kill`/command handlers — deaths the arbiter didn't initiate
(LMK/OOM/crash) only flow through there. Don't add a SIGCHLD handler to
the arbiter expecting to catch app deaths; it won't. See related
[[project_app_lifecycle_and_packaging]], [[feedback_wart_zygote_fork_survival]].
Full detail in `docs/architecture-runtime.md` "Death notification" +
`tasks/54-arbiter-death-notification.md`.
