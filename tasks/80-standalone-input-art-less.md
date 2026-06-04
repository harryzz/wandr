# Task 80 — standalone input source (ART-less input)

> Status: 🟢 core PROVEN + device-verified (2026-06-04). Chose **Option B** (reuse
> Android's C++ `InputReader` standalone) after proving it has no blocking cases.
> **Step 0** (spike): `createInputReader` runs with no system_server; injected
> touch decodes 1:1. **Step 1** (shim): `sf_surface.cpp` runs the InputReader +
> feeds `sf_input_poll` (evdev mode, gated by `WART_EVDEV_INPUT`); built on a-03.
> **Step 3** (harness): `run-hybrid-stack.sh --no-art`/`--restore-art`/`--evdev`.
> **Capstone verified:** with `system_server` + both zygotes stopped (SF/audioserver/
> sensorservice/adbd survive, adb alive), a `sendevent` swipe-up on `/dev/input/event1`
> → keyguard guest → `keyguard UNLOCKED`. Fully interactive with ART off.
> **Step 2** (routing) DONE + device-verified ART-off: per-host input-region
> filtering — each host drops touches outside its surface's visible region
> (`sf_surface` `g_input_filter`/`input_accepts`); overlays self-set their strip,
> the fullscreen app sets its content rect (panel minus chrome insets) via
> `sf_set_input_rect` on the arbiter `geometry` push, and the keyguard is modal via
> task-79 `input-suppress` of the covered app. Verified: app-area tap doesn't leak
> to the taskbar; taskbar-strip tap fires `go-home`; boot-lock suppresses the
> launcher; swipe-unlock resumes it. **All of Steps 0/1/2/3 done.** Follow-ons:
> overlap/popup z-order, side-strip x-offset, `--no-art` reboot-persistence.
> See `[[project_art_shutdown]]`.

## Why

The post-ART end goal is to shut off `system_server` and all ART/Java services and
run the wart stack on the surviving native layer (SurfaceFlinger, audioserver,
sensorservice, HALs, servicemanager — all proven to survive `adb shell stop`; see
`[[project_art_shutdown]]`). The **one hard blocker** is input: `InputDispatcher` /
`InputManagerService` (the `input` / `inputflinger` binder services) are **hosted
inside system_server** — no separate process — so they die with ART, and our UI
becomes render-only. Today's input path (task 33, BBQ-direct attach to the
SurfaceControl) still relies on system_server's InputDispatcher dispatching to our
input channel, so it does not survive ART-off.

**Important:** an InputDispatcher AIDL/binder interface does NOT solve this — the
service lives in system_server and dies with it. We need our OWN input source.

## Goal

A standalone input source that delivers touch + keys to the wart host with NO
dependency on system_server, so the stack is fully interactive with ART off.

## Approach to evaluate (read source first)

- **evdev-direct**: read `/dev/input/event*` directly (the kernel input devices),
  decode `EV_ABS`/`EV_KEY`/multitouch (MT slots) ourselves, and feed the existing
  `dispatch_pointer_v2` / `dispatch_android_key` choke points (task 79) — bypassing
  InputDispatcher entirely. Needs: device enumeration, MT-B protocol decode,
  coordinate scaling (touch resolution → panel px), and per-surface hit-testing /
  focus (which the arbiter already models). Note the prior **`EVIOCGRAB` rejection**
  ([[project_standalone_input]]) — revisit whether grabbing is needed when ART is
  off (no competing InputDispatcher to fight) vs. while ART is still up (must not
  steal input from the OS).
- **standalone InputFlinger**: run InputReader+InputDispatcher as a native helper
  in/under our stack (heavier; pulls libinput). Likely overkill vs. evdev-direct.

Lean evdev-direct: it's the minimal native path, reuses our existing dispatch +
arbiter focus model, and is exactly the kind of HAL/native-survivor dependency the
post-ART design wants (no ART-layer dependency — `[[feedback_no_art_layer_dependencies]]`).

## Dependencies / interactions

- Pairs with the **ART-off deploy mode** (`run-hybrid-stack --no-art`, the targeted
  `stop zygote`+`zygote_secondary` recipe in `[[project_art_shutdown]]`): only once
  standalone input lands can `--no-art` give a usable (interactive) device.
- Dissolves the **PMS contention** follow-on (`[[project_proximity_screen_off]]`)
  for free — with ART off there is no PowerManagerService to fight the proximity
  blank.
- Reuses the task-79 touch choke point (`dispatch_pointer_v2`) and the arbiter's
  per-display surface/role + resource-focus model for routing/hit-testing.

## Out of scope (for the first cut)
- IME / soft-keyboard plumbing (separate; `[[project_ime_options]]`).
- Stylus/hover, multi-display input routing.
- Making ART-off survive reboot (persisted init changes) — keep it a runtime
  `stop`/`start` test mode until input is proven.

## Verification (eventual)
- With `run-hybrid-stack --no-art` (ART stopped, native survivors + our stack up),
  touch + keys drive the wart UI end-to-end — fully interactive with system_server
  dead. adb stays alive throughout (recovery via `start`).

---

## Path A — single `inputflinger` service (supersedes per-host evdev)

> Decision 2026-06-04 (user-directed): the per-host evdev InputReader (Steps 0/1/3)
> was the bootstrap that PROVED ART-less input. But it has N readers for one device,
> so **global keys fan out to every host** — task-81's ART-off device test showed one
> POWER press → **68 `power-key` forwards across 9 hosts** → screen flicker (the
> "volume ×6" problem on the power key). The fix the user steered to is the proven
> Android architecture: **ONE input source, host = applier.**

**Architecture.** Run Android's real `InputManager` (`InputReader` + `InputDispatcher`)
standalone as the `inputflinger` binder service (`runtime/wart-inputflinger/`,
soong cc_binary on a-03). One dispatcher reads `/dev/input` once and routes:
- **app keys/touches → the FOCUSED window only** (focus-based dispatch). The hosts
  connect via their EXISTING inputflinger client path (`sf_surface.cpp:309-352`
  `waitForService("inputflinger") → createInputChannel → InputConsumer →
  setInputWindowInfo`) — i.e. host = applier by simply **not** setting
  `WART_EVDEV_INPUT`. No fan-out, no per-host region filter needed.
- **system keys (POWER 26 / VOLUME 24,25) → the arbiter, ONCE.** Intercepted in our
  dispatcher policy `interceptKeyBeforeQueueing`: forward to the arbiter socket
  (`power-key` / `volume up|down`) and DON'T set `POLICY_FLAG_PASS_TO_USER`, so the
  dispatcher drops them from window dispatch (`InputDispatcher.cpp:1191` →
  `DropReason::POLICY`). This is the wart PhoneWindowManager role.

**Key source finding (why this is small).** `InputDispatcher`'s constructor
**self-registers as a SurfaceFlinger `WindowInfosListener`**
(`InputDispatcher.cpp:962 SurfaceComposerClient::getDefault()->addWindowInfosListener`).
So window geometry/focus flows in automatically from the hosts'
`setInputWindowInfo` calls — the spike's "stage-2 bridge" needs **no** code.

**Integration plan (`run-hybrid-stack --no-art`, reordered to avoid host reconnect).**
The host connects input at surface-creation, so `inputflinger` must be OURS before
the hosts start. SurfaceFlinger survives a framework stop, so:
1. resolve HOME_PKG (`cmd package …`, needs ART) — *while ART up*;
2. force-stop SystemUI + launcher; **stop the Java framework** (zygote +
   zygote_secondary → system_server); SF/audioserver survive;
3. **start `wart-inputflinger` via `wart-launch`** (uid system + gid input +
   CAP_BLOCK_SUSPEND) — registers `inputflinger`;
4. start zygote + arbiter + hosts + chrome **without** `WART_EVDEV_INPUT` → each
   host's client path connects to `wart-inputflinger`.
This avoids any "reconnect-input" mechanism: hosts only ever see our service.

**Status:** service written + API-verified (`runtime/wart-inputflinger/`,
`wart_inputflinger.cpp` + `Android.bp`); building on a-03. Remaining: copy binary
back + deploy; reorder `run-hybrid-stack --no-art` per above; device-verify under
ART-off (focused window gets touch/keys; one POWER press = one toggle, no flicker;
volume once). The task-81 display-power code (power module `power-key`/`panel` +
`wart-screen`) stays — the arbiter is still the power owner, now fed by the
dispatcher policy instead of N hosts.
