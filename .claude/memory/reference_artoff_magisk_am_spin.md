---
name: reference_artoff_magisk_am_spin
description: "Under --no-art, Magisk su-grants spawn looping `am` workers that starve timing-sensitive HALs; avoid with `adb root` (no su)"
metadata: 
  node_type: memory
  type: reference
  originSessionId: f5db1920-cf66-44e0-89f3-8b3adade8711
---

Under `--no-art`, **every Magisk `su -c` grant** makes `magiskd` fork
`com.android.commands.am.Am` to deliver a su-access log/notification to its Manager
app. The framework is dead → `am` can't reach ActivityManager → `magiskd` **respawns
it in a tight loop** (worker reparents to `adbd`/init). Even **one** spinner can starve
a timing-sensitive native HAL past a hard timeout. Confirmed culprit for the
`--no-art` camera: it starved the qcam MCT command thread →
`mct_controller_proc_serv_msg: Timedout type=1` → SIGABRT of
`camera.provider@2.4-service` (which then respawns degraded: `gyro_module_init:
disabled`). A *single* `am` worker (40% on one core, 600%+ idle elsewhere) was enough
— likely binder-thread saturation of `wandr-activityms` (the `am` hammers its `activity`
stub, "Mixing copies of libbinder / Expecting header 0x53595354"), which the camera
also calls.

**Does NOT stop it:** per-uid `logging/notification=0` in the Magisk `policies` table
(this device runs a Magisk fork, `magisk -V`=30700, hidden manager `com.xyshj.machine`);
uninstalling the manager (magiskd auto-falls back to the `com.topjohnwu.magisk` stub
and keeps forking `am`).

**Fix:** `adb root` — this is a `userdebug` build (`ro.debuggable=1`), so adbd restarts
as uid 0 and root commands **never invoke Magisk `su`** → zero `am` spinners. Run
timing-sensitive `--no-art` probes via root adbd, not `su -c`. For the running stack
(which uses `su`), make the one-shot `magisk_worker_sweep` in `run-hybrid-stack.sh` a
**continuous background daemon** for the `--no-art` lifetime. Sweep = kill each
`com.android.commands.am.Am` + its parent (skip pid 1 and main magiskd 1060).

Extends [[project_art_shutdown]] (which documented the high-CPU `am` accumulation). The
camera consequence is tracked in task 95 ([[project_artless_camera]]).
