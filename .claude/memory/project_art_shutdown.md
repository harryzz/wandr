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

**The blocker for true ART-less operation = INPUT.** `InputDispatcher` /
`InputManagerService` (the `input`/`inputflinger` binder services) are **hosted
inside system_server** — there's no separate `inputflinger` process. So with ART
off there is NO touch/key dispatch; our UI is render-only. Binding to an
InputDispatcher AIDL does NOT help — it dies with system_server. The fix is a
**standalone input source** → see `tasks/80-standalone-input-art-less.md`.
[[project_standalone_input]] (current input = BBQ-direct attach, still rides
system_server's InputDispatcher). PMS contention ([[project_proximity_screen_off]])
is a sub-symptom of this same coupling and dissolves once ART is gone.
