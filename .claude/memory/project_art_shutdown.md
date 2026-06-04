---
name: project_art_shutdown
description: How to stop/start the Android Java framework (ART) for post-ART testing — proof of concept + gotchas + recovery
metadata: 
  node_type: memory
  type: project
  originSessionId: a6ba002c-9c9c-4673-9e97-6c4e1c3eba6d
---

Toward the post-ART end goal (run the wart stack with ALL ART/Java services off).
Device-experiment findings (Pixel 2 XL, 2026-06-04):

**Mechanism: `adb shell stop` / `adb shell start`** halts / restores the Android
Java framework. Use this, NOT `kill` (killing system_server trips the watchdog →
reboot/bootloop; `stop` is the graceful, non-rebooting path).

**Recovery is safe** (the lifeline the user requires): `adbd` is an init service
in **`class core`**, started independently of system_server, so it survives a
framework stop. The device is on **USB** (serial `804KPSL1724590`, not a wifi IP),
which also survives. So adb stays up throughout → `adb shell start` always recovers.

**✅ Proven: our stack is ART/zygote-independent.** With the framework stopped,
EVERY wart process survived (`wart-arbiter` + all `wart-host` zygote children) plus
`adbd`. Gone: `system_server`, `zygote`, `zygote_secondary`/`zygote64`,
`webview_zygote`. Our procs are plain root processes (setsid), not init services
and not children of the Android zygote, so `stop` doesn't touch them.

**⚠️ GOTCHA: no-arg `adb shell stop` is TOO BROAD.** It also stopped
**surfaceflinger** + **audioserver** — native C++ services our stack NEEDS
(compositor + audio), not ART/Java. So "disable ART" must target the Java layer
only, not a blanket stop.

**🎯 Targeted ART-off (the right recipe, not yet fully verified):** stop only the
Java framework init services — **`zygote` + `zygote_secondary`** (they carry
system_server) — and KEEP the native survivors: `surfaceflinger`, `audioserver`,
`sensorservice`, `servicemanager`, `hwservicemanager`, `system_suspend`, the HALs,
`adbd`. TODO when building the `--no-art` mode: confirm `stop zygote[_secondary]`
actually brings system_server down (it's a forked child) — may need an explicit
system_server kill — while leaving surfaceflinger up.

**Recovery cycle:** `adb shell start` restarts the full framework (+ SF + zygotes;
~boot time). Our surviving wart procs lose their SurfaceControl/EGL surfaces when
SF restarts (they were bound to the old SF instance), so after `start` re-run
`tools/scripts/run-hybrid-stack.sh` to re-attach our stack to the fresh SF.

**Human-test findings (2026-06-04, `--no-art`):** two gaps surfaced. (1) **No display-
power owner with ART off → device wedges** (black screen; power button dead because
power-key→wake is a PMS function; touch dead because the panel is off; the arbiter's
screen poller reads the now-stale `debug.tracing.screen_state` → spurious doze/auto-
lock). Recover with `adb shell input keyevent 224` after restoring ART. → **task 81**:
wart owns display power (power-key→`SetDisplayPower` toggle via task-78 wart-hal-display;
arbiter drives screen state from its own `panel_on` under `WART_NO_ART`; force-on at
boot). (2) **Keys not routed** — task-80 Step-2 routed touch only, so every host's
InputReader reads hardware keys → one volume press fanned to 6 pids (volume ×6). →
**task 82** (key dedup/focus).

**Service strategy (roadmap §6.6, post-spike):** three buckets — KEEP (bind surviving
native daemons: SF/audioserver/sensorservice/HALs — done), REIMPLEMENT (Java policy →
arbiter modules: AMS/WMS/PMS/alarm/notify/audio/keyguard/sensors/pkg — done), PATH A
(run a system_server-hosted C++ service standalone + register its binder name — only
candidate = InputFlinger; spike proved it runs). The bottleneck is a shared **security
context**, not per-service work: a wart native proc needs **uid system + gid input +
CAP_BLOCK_SUSPEND + a sepolicy domain** to use the survivors with ART off (bare root
hangs on SF's ACCESS_SURFACE_FLINGER check + aborts in EventHub:894). Same context
task 81 setPowerMode needs. **The right way = our flashable image** (init.rc
user/group/capabilities/seclabel + wart sepolicy, ART services not started, enforcing)
— DEFERRED (needs lineage_taimen device build + vendor blobs). **Decision 2026-06-04:
keep the dev scaffold** (rooted + su/setenforce 0 + a setuid+caps `wart-launch`
launcher mimicking the init.rc context) → **task 83**; flashable image later.

**⚠️ HIGH-CPU under --no-art = Magisk su-log workers (CORRECTED 2026-06-05; the
earlier 3-part fix was INCOMPLETE).** Symptom: phone HOT, `top` ~260% busy (129%user
+122%sys) vs ~14% with ART up. Cause: on every `su -c`, magiskd forks a worker that
runs `am ... action log` to notify the (dead) framework; `am` can never reach
ActivityManager so the worker **loops it forever** (new PID each retry → defeats
naive PID tracking, ~100%/core), and they **accumulate** across the many `su -c` of
bringup. WHY THE OLD FIX FAILED: (a) `magisk --sqlite "UPDATE policies SET
logging=0,notification=0"` does NOTHING — the boot magiskd CACHED its policy and
never re-reads the DB; (b) the one-time `pkill -f com.topjohnwu.magisk` killed only
the `am` CHILDREN (their args contain that string), NOT the magiskd WORKER PARENTS
(named just `magiskd`) — so the parents instantly respawn `am`; (c) it ran
mid-bringup, before all the `spawn_detached`/arbiter `su -c` that each leave a fresh
worker. THE REAL FIX (`magisk_worker_sweep` in run-hybrid-stack, run at the END of
bringup, --no-art only): for each `com.android.commands.am.Am`, kill its PARENT
(the stuck worker, stops respawn) + the am child; the MAIN magiskd has no am child
so it survives and `su` keeps working. A 2nd pass after `sleep 4` (same su session,
no new grant) catches the worker the sweep's OWN `su -c` spawns. Verified: ~260% →
~14%. Manual one-shot: `pgrep -f com.android.commands.am.Am` → kill PPIDs (≠1) +
`pkill -f com.android.commands.am.Am`, twice. NOTE: any later manual `adb shell su
-c` re-creates one worker; re-run the sweep. (Steady-state the stack uses the setuid
`wart-launch`, NOT magisk su, so no new workers form on its own.)

**The blocker for true ART-less operation = INPUT.** `InputDispatcher` /
`InputManagerService` (the `input`/`inputflinger` binder services) are **hosted
inside system_server** — there's no separate `inputflinger` process. So with ART
off there is NO touch/key dispatch; our UI is render-only. Binding to an
InputDispatcher AIDL does NOT help — it dies with system_server. The fix is a
**standalone input source** → see `tasks/80-standalone-input-art-less.md`.
[[project_standalone_input]] (current input = BBQ-direct attach, still rides
system_server's InputDispatcher). PMS contention ([[project_proximity_screen_off]])
is a sub-symptom of this same coupling and dissolves once ART is gone.
