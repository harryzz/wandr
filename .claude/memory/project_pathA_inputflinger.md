---
name: project-patha-inputflinger
description: "Path A (standalone wandr-inputflinger as the inputflinger service, ART-off) — what works, the SF window-infos blocker, and the arbiter-feeds-dispatcher fix direction"
metadata: 
  node_type: memory
  type: project
  originSessionId: a6ba002c-9c9c-4673-9e97-6c4e1c3eba6d
---

**✅ RESOLVED (task 84, device-verified 2026-06-04):** app touch/keys now route under
ART-off. The SF-push blocker (below) was sidestepped, NOT fixed: the **wandr-arbiter
(the WMS) authors the ordered window list and pushes it to wandr-inputflinger**, which
feeds the standalone dispatcher via `InputDispatcher::onWindowInfosChanged` (the
unit-test entry). Key pieces: (1) `wandr-arbiter-wm::input_window_block` derives rects
from the surface/role model + insets/orient/keyboard (chrome strips from a new
`Surface.anchor`); binary diff-pushes `win-begin/win/win-focus/win-commit` to the
**abstract** socket `@wandr-inputflinger` (uid-system can't bind a file in /data/local/tmp)
after every command/child-exit, gated `--no-art`. (2) wandr-inputflinger reaches the
concrete `onWindowInfosChanged` WITHOUT the heavy private InputDispatcher.h: a one-method
decl in `namespace android::inputdispatcher` (single-inherit from InputDispatcherInterface
→ getDispatcher() is the object at offset 0; resolves vs libinputflinger.so's exported
symbol). (3) host registers `(pid,channel-token)` with the `wandr.windowreg` binder svc
after createInputChannel (token is a kernel object — can't ride the socket; this one hop
is binder, never through the Rust arbiter); arbiter refers to windows by pid. Gotchas (all fixed):
`sf_surface.cpp`→`libsf_surface.so` (a-03 build, NOT Rust host); dispatcher FATAL-asserts
on duplicate WindowInfo.id (default -1) → set `id=pid`+dedup. **THE key fix: set
`WindowInfo.transform.set(-left,-top)`** — the dispatcher delivers `transform.transform(
rawX,rawY)` as window-local coords (InputDispatcher.cpp:2135) and the host passes them to
the guest verbatim; SF normally authors this, so bypassing SF an offset window (IME
strip, taskbar) handed the guest RAW DISPLAY coords → keys dead, while fullscreen
(offset 0=identity) worked → that's why keyguard/launcher worked but IME/taskbar didn't.
`touchableRegion` stays display-space (hit-test InputDispatcher.cpp:599). IME strip
anchored ABOVE the taskbar: `[h-inset_bottom-keyboard_px, h-inset_bottom]`. Backlight=0
under ART-off (no DisplayManager) — panel renders but invisible (cost a full debug round)
→ arbiter drives `/sys/class/leds/lcd-backlight/brightness` in apply_display_power
(WANDR_BACKLIGHT_{PATH,LEVEL} overridable; boot force-on lights it). VERIFIED PORTRAIT + LANDSCAPE:
swipe-unlock, launcher, app-switch, IME typing, taskbar-with-keyboard, system-key dedup,
landscape chrome/IME + app + keyboard. LANDSCAPE fix: the fullscreen app/keyguard already
worked any orientation (host inverse-maps touch via renderer base_matrix, standalone.rs
:1035); only chrome/IME strips were wrong (authored portrait top/bottom while host renders
them on the physical SIDES). Fix = `wandr-arbiter-wm::strip_rect` mirrors the host's
`overlay_rect` (handedness 0→S/3→N/4→W/7→E; strip th-thick off-inward from the user's
edge) so the input region follows the bars. PURE arbiter change — reader stays portrait
(panel buffer is physically portrait; host owns content rotation), NO wandr-inputflinger/
viewport change. Portrait rects byte-identical (no regression). The blocker write-up below
is the diagnosis record.

---

Path A (task 80) = run AOSP's real `InputManager` (InputReader+InputDispatcher) as
the standalone `inputflinger` binder service so ONE dispatcher reads input once and
routes it (vs the task-80 per-host evdev bootstrap, which fanned global keys to every
host → power-key flicker). Code: `runtime/wandr-inputflinger/` (soong cc_binary, built
on a-03 `external/wandr-inputflinger`; deploy `/data/local/tmp/wandr-inputflinger`, run
via `wandr-launch` = uid system+gid input+CAP_BLOCK_SUSPEND). `run-hybrid-stack --no-art`
wires it. Build fast path (a-03): direct-ninja, NOT `m` — `prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja out/target/product/generic_arm64/system/bin/wandr-inputflinger`.

**WORKS (device-verified, ART off):** system-key dedup. The dispatcher policy
`interceptKeyBeforeQueueing` forwards POWER(26)/VOL(24,25) to the arbiter socket ONCE
(on DOWN) and drops them from windows (no `POLICY_FLAG_PASS_TO_USER` → `DropReason::POLICY`,
InputDispatcher.cpp:1191). One physical press = one toggle, no flicker. Needed
`setInputDispatchMode(true,false)` after start() (boots disabled → "Dropped event
because input dispatch is disabled"). Hosts use their existing inputflinger client path
(`sf_surface.cpp:309` createInputChannel/InputConsumer) by NOT setting WANDR_EVDEV_INPUT.

**BLOCKER (touch never routes — "no touchable window"):** the dispatcher has ZERO
windows because **SurfaceFlinger never pushes WindowInfos to our process under ART-off**:
- SF binds the inputflinger service ONCE at its own init (`SurfaceFlinger.cpp:773
  waitForService("inputflinger") → mInputFlinger`); `SF::binderDied` does
  `mInputFlinger.clear()` when system_server (the WM) dies and never re-resolves.
- `SurfaceFlinger::updateInputFlinger` EARLY-RETURNS if `mInputFlinger` is null
  (SurfaceFlinger.cpp:4104) → pushes WindowInfos to NOBODY (dispatcher self-registers
  a listener in its ctor: InputDispatcher.cpp:962, but gets nothing).
- Restarting SF after registering wandr-inputflinger (so init re-binds ours) was tried
  and STILL no push (a binding race as system_server's `inputflinger` servicemanager
  entry hands over to ours; mInputFlinger ends up null/stale). SF composites fine +
  holds visible touchable windows (`dumpsys SurfaceFlinger` shows `input{(0x0)
  touchableRegion=...}`), so it's purely the listener-delivery path.
- The client "pull" `addWindowInfosListener(..., &outInitialInfo)` only returns the
  reporter's CACHE of the last PUSH (`WindowInfosListenerReporter.cpp` mLastWindowInfos)
  — useless when no push happened.

**Also found:** the taimen layer transform transposes input touchableRegions to
2880×1440 (host registers portrait 1440×2880, SF reports landscape) — a SECOND coord
issue that only matters once windows actually reach the dispatcher. wandr-inputflinger
viewport is env-tunable (`WANDR_VP_LOGICAL_W/H`, `WANDR_VP_DEVICE_W/H`, `WANDR_VP_ORIENT`)
to dial in once delivery works; forwarded by run-hybrid-stack.

**DECISIVE DIAGNOSIS (2026-06-04, wandr_wininfo_probe):** a plain root process that
registers a `gui::WindowInfosListener` (NO addService) and runs UNDER NORMAL ART
RECEIVES SF pushes fine — `onWindowInfosChanged: 11 windows` (StatusBar/Taskbar/launcher),
PORTRAIT frames (e.g. StatusBar [0,0,1440,98]). So: (a) SF's push is NOT permission-gated
against our process; the ART-off failure is CONCLUSIVELY `mInputFlinger==null`; (b) the
2880×1440 transpose is OUR hosts' pre-rotated surfaces, a SEPARATE issue (ART-up frames
are portrait). Probe binary = runtime/wandr-inputflinger/wandr_wininfo_probe.cpp.

**BYPASS RULED OUT:** `onWindowInfosChanged` is only on the CONCRETE `InputDispatcher`
(42 internal dispatcher headers — not includable in our cc_binary); `IInputFlinger` has
no window-inject API; the client "pull" (`outInitialInfo`) is just the reporter's CACHE
of the last push. So SF's push is the ONLY way to get windows in → must make SF push →
need `mInputFlinger` bound to our service.

**FIX ATTEMPT (fragile, NOT working yet):** start wandr-inputflinger FIRST (so SF init's
waitForService binds mInputFlinger=ours), THEN restart SF; + a reconnect-poke thread in
wandr-inputflinger (`getPhysicalDisplayIds()` → `ComposerServiceAIDL::getComposerService()`
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

**✅ RESOLVED (task 84) — SOLVED+device-verified PORTRAIT+LANDSCAPE.** (Moved from
the index line, which carried this resolution while the file ended at the task-84
scoping above.) App touch/keys route ART-off: swipe-unlock, launcher, app-switch,
IME typing, taskbar-with-keyboard, landscape chrome+app+keyboard, system-key dedup.
Architecture: arbiter (WMS) authors the window list → abstract socket
`@wandr-inputflinger` → `onWindowInfosChanged` (1-method decl in
`android::inputdispatcher`, offset-0 single-inherit cast vs the libinputflinger.so
export — NO heavy InputDispatcher.h); host registers pid→token via the
`wandr.windowreg` binder service. KEY FIX: `WindowInfo.transform.set(-left,-top)` —
the dispatcher delivers `transform.transform(raw)` = window-local coords
(InputDispatcher.cpp:2135); offset windows (IME/taskbar) got raw display coords →
dead, while fullscreen (identity) worked. LANDSCAPE: arbiter `strip_rect` mirrors
the host `overlay_rect` handedness (0→S/3→N/4→W/7→E) so chrome/IME input follows
the rotated bars; app/keyguard already worked via the host base_matrix inverse-map.
Also: id=pid dedup, IME strip above taskbar, arbiter drives backlight (was 0
ART-off = invisible panel). `sf_surface` → `libsf_surface.so` (a-03 C++, not Rust).

**FIRST-LAUNCH-INPUT FIX (735a0b9d):** a freshly-launched app was input-dead until
a bg+fg round-trip — the arbiter pushes the window block at launch BEFORE the
forked host registers its `wandr.windowreg` token (~70 ms later);
`feed_window_block` skips a token-less pid and nothing re-pushed. Fix = cache the
last block + on `TX_REGISTER` call `refeed_last_block()` to re-apply it once the
token lands.
