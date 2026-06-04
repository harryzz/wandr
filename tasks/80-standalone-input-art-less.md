# Task 80 — standalone input source (ART-less input)

> Status: 🔲 scoped, not started. The blocker for running the wart stack with the
> Android Java framework (ART) fully off.

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
