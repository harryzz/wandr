---
name: project_idle_cpu_chrome
description: idle CPU of the wandr stack profiled (simpleperf) and cut 14-15% → 9% — date-fork in clock_text + idle-adaptive input poll; launcher idle wasm work remains
metadata: 
  node_type: memory
  type: project
  originSessionId: a2edab94-9d77-4289-807e-6fabf67af25c
---

2026-06-10: user noticed idle CPU creep (8-9% → 11-12%). Profiled with on-device
`simpleperf` (exists at /system/bin/simpleperf) instead of guessing. **Three distinct
causes** (the "it must be the 60 Hz poll" single-cause guess was wrong for the
biggest one):

1. **statusbar ~4.7% = `clock_text()` fork+exec'ing `/system/bin/date` ~1 Hz**
   (status_impl.rs). Profile = copy_mm/copy_page_range/unmap_single_vma + [linker]
   relocations: each spawn forks the 57 MB wasmtime host + relinks toybox. FIXED:
   in-process `libc::localtime_r` (bionic tzset reads persist.sys.timezone — still
   ART-free). Verified correct local time on screen.
2. **60 Hz idle event loop ~2-4%/surface** (taskbar profile = InputConsumer::consume +
   sf_input_poll + scheduler drain + kernel wake cost; NO render symbols — task-64
   on-demand render gating works fine). FIXED: idle-adaptive poll in standalone.rs —
   after 1 s with no `dirty` events, poll cap 16 ms (POLL_MS) → 48 ms (IDLE_POLL_MS);
   first event snaps back. Background stays 200 ms. Verified wakes 63-67/s → 21-26/s.
3. **launcher ~1% idle wasm execution** (~27% of its samples in cwasm JIT frames —
   dioxus timer callbacks / re-renders while static). NOT yet fixed — open follow-up.

RESULT (12 s sustained, idle+unlocked): total wandr+SF **14-15% → 9.0%** of one core
(statusbar 4.7→1.5, SF 3→1.9, keyguard-bg 0.2, signal-bg 0.3). UNCOMMITTED (rode with
the task-93 crypto working tree).

Measurement recipe that worked: per-pid jiffies delta (`/proc/<pid>/stat` f14+f15) over
10-12 s; wake rate = voluntary_ctxt_switches delta (main thread); `simpleperf record
-p <pid> --duration 10` + `report --sort symbol` for attribution. GOTCHA: the
standalone "rendered frame N" counter increments per LOOP ITERATION, not per render —
N/uptime gives loop Hz, not fps (misread it as 55 fps rendering at first). GOTCHA:
auto-lock can flip roles mid-measurement (launcher→Background = 5 wakes/s is correct
BG_POLL, not a bug); unlock right before measuring.
