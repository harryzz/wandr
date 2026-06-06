# Task 84 — Path-A app touch/key routing under ART-off (SF WindowInfos → our dispatcher)

> Follow-up fix (2026-06-06, commit 735a0b9d): **first-launch dead input**. A
> freshly launched app was unresponsive until backgrounded+foregrounded from the
> taskbar. Race: the arbiter authors+pushes the input-window list at `launch`, but
> the forked host registers its window token with `wart.windowreg` ~70 ms LATER
> (after EGL surface + input channel). `feed_window_block` skips a pid with no
> token, and nothing re-pushed until the next arbiter command (the foreground
> round-trip). Fix in wart-inputflinger: cache the last block + have `TX_REGISTER`
> call `refeed_last_block()` so the window is delivered the instant its token
> registers. Device-verified: fresh apps responsive on first tap. (`g_feed_mtx`
> serializes listener vs. binder re-feed; re-feed passes cache=false.)

> Status: ✅ DONE + device-verified, PORTRAIT **and LANDSCAPE** (Pixel 2 XL,
> 2026-06-04). Solved via **Option 3, arbiter-driven**: the wart-arbiter (the WMS)
> authors the ordered input-window list and pushes it to wart-inputflinger, which
> feeds the standalone InputDispatcher via `onWindowInfosChanged` — sidestepping the
> dead SF push entirely. Under `--no-art`: swipe-up unlocks the keyguard, launcher
> taps launch apps, app-switching works, IME typing works, taskbar works with the
> keyboard up, system keys still dedup, no "no touchable window". In landscape the
> chrome/IME input strips track the rotated render positions (bars on the sides) and
> app elements + the keyboard route correctly. Spun out of task 80 path A.
>
> **Landscape fix:** the fullscreen app/keyguard already worked in any orientation
> (the host inverse-maps touch via the renderer `base_matrix`, standalone.rs:1035) —
> only the chrome/IME strips were wrong, authored as portrait top/bottom strips while
> the host renders them on the physical sides. Fix: `wart-arbiter-wm::strip_rect`
> faithfully mirrors the host's `overlay_rect` (handedness `0→S/3→N/4→W/7→E`, strip
> `th` thick `off` inward from the user's edge), so the input region follows the
> bars. Pure arbiter change — no wart-inputflinger / reader-viewport change (the
> reader stays portrait; the panel buffer is physically portrait and the host owns
> content rotation). Portrait rects are byte-identical (no regression).
>
> ## What shipped (the implementation)
> - **Arbiter (WMS authors windows):** `wart-arbiter-wm::input_window_block` derives the
>   ordered per-display window rects from the surface/role model + insets + orientation
>   + keyboard occlusion (no hardcoded geometry); chrome strips placed from a new
>   `Surface.anchor` (`ChromeAnchor::{Top,Bottom}`, set at `register-chrome`). The binary
>   diffs + pushes the block (`win-begin`/`win`/`win-focus`/`win-commit`) after every
>   command + child-exit, gated on `--no-art`, to the `@wart-inputflinger` socket.
> - **Transport:** ABSTRACT-namespace UNIX socket (`@wart-inputflinger`) — wart-inputflinger
>   runs as uid system and can't bind a file under `/data/local/tmp` (0771 shell:shell);
>   the abstract namespace sidesteps filesystem perms. Arbiter (root) connects per push.
> - **wart-inputflinger (feeds the dispatcher):** a socket listener builds `gui::WindowInfo`s
>   (token per pid, unique `id=pid`) and calls the concrete dispatcher's
>   `onWindowInfosChanged` + `setFocusedWindow`. The concrete method is reached WITHOUT
>   the heavy private `InputDispatcher.h` via a one-method local decl in
>   `namespace android::inputdispatcher` — InputDispatcher is single-inheritance from
>   InputDispatcherInterface, so `getDispatcher()` IS the object at offset 0; the call
>   resolves against `libinputflinger.so`'s exported symbol (verified `llvm-nm -D`).
> - **Host (carries the token):** `register_window_token_artless()` registers
>   `(pid, channel-token)` with the `wart.windowreg` binder service after `createInputChannel`
>   (token can't ride the socket — kernel object; this one hop is binder, host↔inputflinger,
>   never through the Rust arbiter). No-op under normal ART (`checkService` null).
>
> ## Gotchas hit + fixed during bring-up
> - `sf_surface.cpp` compiles into **`libsf_surface.so`** (built on a-03), NOT the Rust
>   `wart-host` — must rebuild + push the shim, not just the host.
> - Dispatcher FATAL-asserts on duplicate window `id` (default -1) → set `id=pid` + dedup.
> - **`WindowInfo.transform` MUST be set** (the bug that made the IME/taskbar dead while
>   fullscreen worked): the dispatcher delivers `transform.transform(rawX,rawY)` as the
>   window-local coords (InputDispatcher.cpp:2135) and the host passes them to the guest
>   verbatim. SF normally authors this; bypassing SF we set `transform.set(-left,-top)`
>   per window — identity for fullscreen (offset 0, so keyguard/launcher worked), a real
>   translate for offset strips (IME bottom strip, taskbar) so their guests get
>   surface-local coords instead of raw display coords. `touchableRegion` stays display-
>   space (hit-test at :599). IME strip anchored ABOVE the taskbar:
>   `[h - inset_bottom - keyboard_px, h - inset_bottom]` (anchoring at `h` overlapped +
>   stole the taskbar's taps and mis-aligned the keys).
> - **Backlight = 0 under ART-off** (no DisplayManager): the panel renders (`screencap`
>   proves it) but is invisible — looked like "touch broken" for a whole debugging round.
>   Fixed properly: the arbiter (display-power authority under `--no-art`) drives
>   `/sys/class/leds/lcd-backlight/brightness` in `apply_display_power` (on→level, off→0;
>   `WART_BACKLIGHT_{PATH,LEVEL}` overridable). Boot force-on lights it automatically.
> - **Backlight gap (SEPARATE follow-on, not this task):** under ART-off the backlight
>   sits at brightness 0 (no DisplayManager) — the screen renders (screencap proves it)
>   but is invisible. Set `/sys/class/leds/lcd-backlight/brightness`. Relates to task 81
>   display power; should be folded into the ART-off display-power ownership.
>
> ---
> Original scoping (kept for reference) follows. Full diagnosis: `[[project_pathA_inputflinger]]`.

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
