---
name: feedback-device-perf-measurement
description: "How to measure CPU/perf on the wandr device without producing FALSE results — the role-demotion false-positive, instrument-with-1Hz-counters, fg−bg=render, and don't trust prior attributions (task 115 fg-CPU investigation, 2026-07-09)"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

Measuring guest CPU/perf on the wandr device is trap-laden; this session produced
TWO wrong answers before the right one (a "0.5% = fixed!" false-positive, then a
misdiagnosis inherited from a task note). Rules that would have saved the time:

**Why:** during task-115 fg-pump-CPU tuning, (1) I measured Signal at 0.5% and
almost declared victory — but it had been auto-demoted to Background (keyguard
locked), where CPU was *always* low; the FOREGROUND number was unchanged. (2) The
task note asserted the 11.5% fg-idle came from `run_concurrent` pump sweeps; I
built a pump throttle, cut pumps 21→3/s, and CPU didn't budge — the pump was never
the cost (it's render). Both errors came from not verifying the measurement
conditions and trusting a prior attribution instead of measuring.

**How to apply:**

- **Verify the ROLE before trusting a CPU number.** Apps auto-demote to Background
  when the keyguard locks (and launch *behind* it). Background role ≈ always low
  CPU — reading it as "foreground fixed" is a false-positive. Check
  `arbiter list` for `[fg]` and the log `role transition … → Foreground`;
  `arbiter unlock` + re-`launch` to get the app foreground, and confirm it *stays*
  foreground across the sample window.
- **Don't trust prior CPU attributions — MEASURE.** Add temporary 1 Hz loop
  counters (iters / renders / pumps / bg-ticks per second) and read them; existing
  throttled logs (`bg-tick #%50`, `frame #%600`) are too sparse to give a rate.
  The counters instantly showed pumps were already ~2–3/s, not the 21/s the note
  implied. Remove the diagnostics after (don't commit them).
- **fg − bg = render.** Foreground CPU minus Background CPU isolates the render
  cost (background does zero rendering, everything else runs in both). A structural
  way to attribute the delta without a profiler.
- **A change that doesn't move the number is a RESULT, not a failure** — it
  disproves the hypothesis. Here it proved the pump isn't the cost; keep/revert the
  change on its own merits, and correct the record (the task note *and* the memory).

Related device-testing traps (all bit this session): `ps|grep <app-id>` never
matches a guest (all are `wandr-host`) — use `/proc/<pid>` + screenshot for
liveness (`[[reference_host_aot_codegen_corruption]]`); reinstalling a *system-app*
while the zygote holds its preload SIGSEGVs the child — bounce
`run-hybrid-stack.sh --wandr-only` (`[[reference_missing_instance_error_stale_zygote]]`,
user apps aren't preloaded so exempt); `wasm-tools validate` is more lenient than
wasmtime's on-device precompile (async-lift of a sync-typed func passes validate,
fails precompile — see `[[project_task115_wasip3_async]]`); background cargo builds
must `source env-android.sh` or `ring` fails; the `cargo build 2>&1|grep … ; cargo
build 2>&1|tail` pattern compiles on the first call and shows cache on the second
(don't misread "Finished 0.4s" as "didn't build").
