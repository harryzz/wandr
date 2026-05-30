---
name: project-standalone-input
description: "Task 33 Step 3 (standalone input from InputFlinger) — implemented, builds, but touch not routing; blocked on input-window geometry"
metadata: 
  node_type: memory
  type: project
  originSessionId: 1b4553dd-d7f1-4367-9435-31b88f0c8841
---

Task 33 Step 3 — input for the standalone runtime — **mid-implementation as
of 2026-05-22.** Full resume detail is in `.task-state` and
`tasks/33-boot-model-bringup.md` (Step 3 "Implementation progress").

**Approach chosen: A — InputFlinger input channel** (not direct evdev).
Confirmed viable on device: the input channel registers cleanly
(`dumpsys input` → `channelName='wart input', status=NORMAL`) with **no
SELinux AVC denials**. The AOSP reference is
`frameworks/native/libs/gui/tests/EndToEndNativeInputTest.cpp`.

**Implemented** (all builds clean — host + in-tree shim — and deploys):
folded into the `sf_surface` shim — `register_input_window()`
(`waitForService("inputflinger")` → `createInputChannel` →
`setInputWindowInfo` + `setFocusedWindow`) and `sf_input_poll()` draining
`InputConsumer`; `src/sf_surface.rs` `poll_input()`; `standalone.rs`'s loop
dispatches via `input::dispatch_pointer_v2`.

**Blocker — touch does not route.** `dumpsys input` shows the wart input
window with `frame=[0,0][0,0], touchableRegion=<empty>` → InputDispatcher
drops injected taps as `ACTION_OUTSIDE`. Root cause: `setInputWindowInfo`
was applied to `g_control` — the *parent* `SurfaceControl` — but the buffer
and bounds live on the **BLAST child layer** that `getSurface()` creates.
The parent has no buffer ⇒ empty input geometry; SF derives input geometry
from the layer and ignores the `WindowInfo`'s explicit `frame`/
`touchableRegion`. The Step-2 `createSurface`-transpose also leaks in (the
layer's input frame is landscape `2880×1440` vs the portrait content).

**Resume:** give the input window real bounds —
`Transaction::setCrop(g_control,…)` or set input on the BLAST child — then
map the touch coordinate space (InputFlinger reports in the landscape layer
frame; guest UI is the transposed portrait buffer). Checkpoint: `dumpsys
input` shows a non-empty `touchableRegion`.

Related: [[project-standalone-orientation]], [[project-boot-model-libgui-build]].
