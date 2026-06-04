# Task 84 — Path-A app touch/key routing under ART-off (SF WindowInfos → our dispatcher)

> Status: 🔲 open (scoped 2026-06-04). Spun out of task 80 path A. The standalone
> `wart-inputflinger` service works for **system keys** (POWER/VOLUME deduped, no
> flicker — committed `5e99896e`), but **app touch/keys don't route** under ART-off
> because SurfaceFlinger never delivers WindowInfos to our InputDispatcher. This
> task is to make app input route to the focused window with the Java framework off.
> Full hard-won diagnosis lives in `[[project_pathA_inputflinger]]` — read it first.

## The blocker (decisively diagnosed — don't re-derive)

The InputDispatcher hit-tests touches against the windows SF pushes it via a
`WindowInfosListener`. Under ART-off our dispatcher gets **zero windows**, so every
touch logs `"no touchable window"` and is dropped. Root cause, confirmed in AOSP
source + on device:

- SF binds the `inputflinger` service **once, at its own init**
  (`SurfaceFlinger.cpp` `waitForService("inputflinger") → mInputFlinger`).
- `SF::binderDied` does `mInputFlinger.clear()` when system_server (the WM) dies and
  **never re-resolves**.
- `SurfaceFlinger::updateInputFlinger` **early-returns when `mInputFlinger` is null**
  (`SurfaceFlinger.cpp:4135` in the a-03 tree) → it pushes WindowInfos to **nobody**,
  including our dispatcher's self-registered listener (`InputDispatcher.cpp:962`).

**Decisive proof it IS `mInputFlinger` (not permission / not our listener):** the
`wart_wininfo_probe` (a plain root process that just registers a
`gui::WindowInfosListener`, no `addService`) run **under normal ART** receives pushes
fine — `onWindowInfosChanged: 11 windows`, **portrait** frames. So SF's push works
for an ordinary process when `mInputFlinger` is valid; the only ART-off difference is
`mInputFlinger == null`.

## Constraints (ruled-out shortcuts — don't retry blindly)

- **No clean bypass to feed the dispatcher.** `onWindowInfosChanged` is only on the
  *concrete* `InputDispatcher` (42 internal dispatcher headers — not includable in an
  external cc_binary); `IInputFlinger` has no window-inject API; the client "pull"
  (`addWindowInfosListener` `outInitialInfo`) is just the reporter's CACHE of the last
  push. So **SF must push** → `mInputFlinger` must be bound to our service.
- **SF-restart dance is fragile.** Restarting SF so its init re-binds our inputflinger
  was tried both orders: it either races on `waitForService` (mInputFlinger stays
  null) or, with inputflinger-first, the concurrent restart **regressed input
  reading** (power died too). The reconnect-poke (`getPhysicalDisplayIds()` →
  `ComposerServiceAIDL::getComposerService()` → `WindowInfosListenerReporter::reconnect`)
  DOES move our listener to the new SF, but SF still didn't push (SF only pushes on a
  window CHANGE *after* re-registration). Not solved.

## Options (pick one in the new session)

1. **Instrument SurfaceFlinger properly.** Add logs at `updateInputFlinger`
   (mInputFlinger / mUpdateInputInfo) + `WindowInfosListenerInvoker` (listener count /
   send) and on device confirm exactly why no push after an SF restart. BLOCKER: the
   quick `m surfaceflinger` failed at a HIDL/composer step (build stops ~225/2952) —
   needs the full SF build fixed first. DEPLOY RISK: pushing an AOSP `libsurfaceflinger.so`
   onto LineageOS (bind-mount over /system/lib64 + init `start surfaceflinger`) — ABI
   must match (device + a-03 both API 35, so plausible, but SF is a big lib). Most
   decisive once it builds.
2. **Reliable SF re-bind choreography.** Get `mInputFlinger` bound deterministically:
   register inputflinger BEFORE any SF (re)start so `waitForService` returns instantly,
   AND don't disrupt the InputReader. Possibly: keep wart-inputflinger's EventHub
   isolated from the SF restart (separate process already), and force a window CHANGE
   after the listener reconnects (so SF actually pushes). Cheapest if the timing can be
   made robust; risk = the fragility seen so far.
3. **Host→inputflinger custom window feed (bypass SF push).** A custom binder service
   from the hosts carrying `gui::WindowInfo` (the token serializes over binder). To feed
   the dispatcher we'd need the *concrete* `InputDispatcher` (`onWindowInfosChanged`) —
   i.e. construct `InputDispatcher` + `InputReader` directly instead of via `InputManager`
   (re-implement the reader→dispatcher wiring the unit tests/InputManager use). Most code
   but fully sidesteps SF + uses our own authoritative geometry (the arbiter is the WMS).

## Secondary issue (only bites once windows arrive)

The taimen pre-rotation transposes our hosts' input touchableRegions to 2880×1440 in
display space (ART-up windows are portrait 1440×2880). Once the dispatcher gets our
windows, align the `wart-inputflinger` viewport (env-tunable: `WART_VP_LOGICAL_W/H`,
`WART_VP_DEVICE_W/H`, `WART_VP_ORIENT`, forwarded by run-hybrid-stack) to SF's window
space, or fix the host so the input region isn't transposed.

## How to work it (env + build + test)

- **Build (fast):** edit `runtime/wart-inputflinger/*.cpp` → `scp` to
  `a-03:~/android/lineage/external/wart-inputflinger/` → direct-ninja (NOT `m`):
  `prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja \
  out/target/product/generic_arm64/system/bin/wart-inputflinger` → `scp` back.
  (a-03 = `ssh harry@a-03 -i ~/.ssh/id_rsa.my`; recipe in `[[project-boot-model-libgui-build]]`.)
- **Diagnostic:** `wart_wininfo_probe` (already built target) — register a listener +
  log pushes; safe under ART; the canonical "does SF push to us" check.
- **Run ART-off:** `tools/scripts/run-hybrid-stack.sh --no-art` (currently stable:
  stop framework → wart-inputflinger → hosts, NO SF restart). Recover:
  `--restore-art` (kills wart-inputflinger + `start`). Bootanim covers the UI under
  ART-off: `setprop service.bootanim.exit 1; stop bootanim` to reveal it.
- **Read result:** `logcat | grep -E "no touchable window|InputDispatcher"` (touch),
  arbiter log for `power-key`/`set_display_power` (system keys).

## Done when
- Under `--no-art`, a tap on the keyguard/launcher routes to the focused window
  (swipe-up unlocks; app icons respond); system-key dedup still works; no input
  regressions; device recoverable throughout.

## Related
`[[project_pathA_inputflinger]]` (the diagnosis + all dead-ends), task 80 (path A),
task 82 (key dedup — system keys solved here), task 81 (display power), task 83
(security context / wart-launch), `[[project_art_shutdown]]`.
