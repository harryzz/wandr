---
name: project-patha-inputflinger
description: "Path A (standalone wart-inputflinger as the inputflinger service, ART-off) — what works, the SF window-infos blocker, and the arbiter-feeds-dispatcher fix direction"
metadata: 
  node_type: memory
  type: project
  originSessionId: a6ba002c-9c9c-4673-9e97-6c4e1c3eba6d
---

Path A (task 80) = run AOSP's real `InputManager` (InputReader+InputDispatcher) as
the standalone `inputflinger` binder service so ONE dispatcher reads input once and
routes it (vs the task-80 per-host evdev bootstrap, which fanned global keys to every
host → power-key flicker). Code: `runtime/wart-inputflinger/` (soong cc_binary, built
on a-03 `external/wart-inputflinger`; deploy `/data/local/tmp/wart-inputflinger`, run
via `wart-launch` = uid system+gid input+CAP_BLOCK_SUSPEND). `run-hybrid-stack --no-art`
wires it. Build fast path (a-03): direct-ninja, NOT `m` — `prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja out/target/product/generic_arm64/system/bin/wart-inputflinger`.

**WORKS (device-verified, ART off):** system-key dedup. The dispatcher policy
`interceptKeyBeforeQueueing` forwards POWER(26)/VOL(24,25) to the arbiter socket ONCE
(on DOWN) and drops them from windows (no `POLICY_FLAG_PASS_TO_USER` → `DropReason::POLICY`,
InputDispatcher.cpp:1191). One physical press = one toggle, no flicker. Needed
`setInputDispatchMode(true,false)` after start() (boots disabled → "Dropped event
because input dispatch is disabled"). Hosts use their existing inputflinger client path
(`sf_surface.cpp:309` createInputChannel/InputConsumer) by NOT setting WART_EVDEV_INPUT.

**BLOCKER (touch never routes — "no touchable window"):** the dispatcher has ZERO
windows because **SurfaceFlinger never pushes WindowInfos to our process under ART-off**:
- SF binds the inputflinger service ONCE at its own init (`SurfaceFlinger.cpp:773
  waitForService("inputflinger") → mInputFlinger`); `SF::binderDied` does
  `mInputFlinger.clear()` when system_server (the WM) dies and never re-resolves.
- `SurfaceFlinger::updateInputFlinger` EARLY-RETURNS if `mInputFlinger` is null
  (SurfaceFlinger.cpp:4104) → pushes WindowInfos to NOBODY (dispatcher self-registers
  a listener in its ctor: InputDispatcher.cpp:962, but gets nothing).
- Restarting SF after registering wart-inputflinger (so init re-binds ours) was tried
  and STILL no push (a binding race as system_server's `inputflinger` servicemanager
  entry hands over to ours; mInputFlinger ends up null/stale). SF composites fine +
  holds visible touchable windows (`dumpsys SurfaceFlinger` shows `input{(0x0)
  touchableRegion=...}`), so it's purely the listener-delivery path.
- The client "pull" `addWindowInfosListener(..., &outInitialInfo)` only returns the
  reporter's CACHE of the last PUSH (`WindowInfosListenerReporter.cpp` mLastWindowInfos)
  — useless when no push happened.

**Also found:** the taimen layer transform transposes input touchableRegions to
2880×1440 (host registers portrait 1440×2880, SF reports landscape) — a SECOND coord
issue that only matters once windows actually reach the dispatcher. wart-inputflinger
viewport is env-tunable (`WART_VP_LOGICAL_W/H`, `WART_VP_DEVICE_W/H`, `WART_VP_ORIENT`)
to dial in once delivery works; forwarded by run-hybrid-stack.

**DECISIVE DIAGNOSIS (2026-06-04, wart_wininfo_probe):** a plain root process that
registers a `gui::WindowInfosListener` (NO addService) and runs UNDER NORMAL ART
RECEIVES SF pushes fine — `onWindowInfosChanged: 11 windows` (StatusBar/Taskbar/launcher),
PORTRAIT frames (e.g. StatusBar [0,0,1440,98]). So: (a) SF's push is NOT permission-gated
against our process; the ART-off failure is CONCLUSIVELY `mInputFlinger==null`; (b) the
2880×1440 transpose is OUR hosts' pre-rotated surfaces, a SEPARATE issue (ART-up frames
are portrait). Probe binary = runtime/wart-inputflinger/wart_wininfo_probe.cpp.

**BYPASS RULED OUT:** `onWindowInfosChanged` is only on the CONCRETE `InputDispatcher`
(42 internal dispatcher headers — not includable in our cc_binary); `IInputFlinger` has
no window-inject API; the client "pull" (`outInitialInfo`) is just the reporter's CACHE
of the last push. So SF's push is the ONLY way to get windows in → must make SF push →
need `mInputFlinger` bound to our service.

**FIX ATTEMPT (fragile, NOT working yet):** start wart-inputflinger FIRST (so SF init's
waitForService binds mInputFlinger=ours), THEN restart SF; + a reconnect-poke thread in
wart-inputflinger (`getPhysicalDisplayIds()` → `ComposerServiceAIDL::getComposerService()`
→ `WindowInfosListenerReporter::reconnect`) to move the dispatcher's listener to the new
SF. The poke DOES fire ("ComposerServiceAIDL reconnected" after restart) and SF-new shows
no "Failed to link" + holds the windows — yet STILL no push to our listener (SF only
pushes on a CHANGE *after* (re)registration; alignment under ART-off keeps failing). And
the inputflinger-FIRST reorder REGRESSED input reading (power stopped too — InputReader
disrupted by the concurrent SF restart). REVERTED toward consolidation.

**RECOMMENDATION:** consolidate the wins (system-key dedup + display power + service infra
+ no-hardcode socket), commit, and make path-A touch a focused task. Remaining options to
crack it: (1) properly instrument SF (the quick `m surfaceflinger` failed at a HIDL step;
needs a working full SF build) to see mInputFlinger/listener state under ART-off; (2)
host→inputflinger CUSTOM binder service carrying gui::WindowInfo (token serializes over
binder) so we feed windows without SF — but onWindowInfosChanged isn't reachable, so this
needs constructing the concrete InputDispatcher directly (re-implementing InputManager
wiring). Both are real work. SF-restart choreography is inherently fragile under ART-off.

**Scoped as a focused task: `tasks/84-pathA-touch-windowinfos.md`** (the 3 options +
build/test recipe + the ruled-out shortcuts) — start there in the new session.

See [[project_art_shutdown]], tasks/80-standalone-input-art-less.md (Path A section),
tasks/84-pathA-touch-windowinfos.md.
