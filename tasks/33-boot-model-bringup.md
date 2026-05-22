# Task 33 — Boot-model bring-up: run the runtime as a standalone privileged process

**Status:** 🟡 in progress — Step 1 ✅ device-verified 2026-05-21. This is a
sub-roadmap; execute the steps in order, each verifiable on the rooted phone.

**Drafted:** 2026-05-20. Spun out of `post-art-roadmap.md` §11
("Recommended next — boot-model bring-up").

---

## Why this task

The PoC runs as a normal **`NativeActivity` APK** on a rooted phone —
WindowManager hands it a window, the Activity drives its lifecycle and
input. The post-ART direction (see `post-art-roadmap.md`) needs the
runtime to run as a **privileged standalone process** that *owns* the
display and input directly, with no Activity / WindowManager /
ActivityManager above it.

This task gets the runtime to that point. It does **not** remove ART —
per the roadmap, the device keeps booting normally; the runtime runs as
a privileged peer and progressively takes over. "Stop ART" is the last
step of the broader arc, not this task.

### Decisions already made (from `post-art-roadmap.md`)

- **Rooted-incremental**, not a from-scratch AOSP image. "Wire first,
  stop ART last." Every step here is verifiable on the live rooted
  device; nothing bricks an intermediate state.
- **SurfaceFlinger client** for display — §5. Wayland was considered
  and rejected for the Android target — §5.1.
- **Monolithic-first** runtime model — §9. One process, one
  `wasmtime::Engine`. (Hybrid/zygote is the later production target;
  not this task.)
- **Keep the native daemons** (SurfaceFlinger, InputFlinger,
  AudioFlinger, servicemanager, vendor HAL daemons) — §6.1.

---

## Current host architecture (what a fresh session must know)

`wart-host/` — a Rust binary, also a C++ shim layer in `cpp/`.

- **Entry:** `android_main(app: AndroidApp)` (`src/lib.rs:639`). The
  `android-activity` crate calls it. It builds a winit
  `EventLoop::builder().with_android_app(app).build()` and
  `run_app(...)`. winit's `EventLoop` is **once-per-process and
  Activity-coupled** (this is what the `RecreationAttempt` panic on
  the Pixel 6 Pro was — see below).
- **Surface:** `NativeActivity` → winit hands an `ANativeWindow*` →
  `src/egl.rs` builds the EGL/GL context on it → skia-safe GL backend
  draws → `eglSwapBuffers`. The window is *implicitly* a SurfaceFlinger
  layer that WindowManager allocated.
- **Input:** winit events (from the Activity) → `src/input.rs` →
  the guest's WIT exports (`on-pointer-event-v2`, `on-key-event-v2`,
  …).
- **cwasm loading:** prefers a filesystem cwasm
  (`/sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm`),
  falls back to the APK asset via `AssetManager` (needs `AndroidApp`).
- The host **also builds for desktop** (winit desktop, JIT) — a
  non-Android code path already exists and is a useful reference for
  "host without an Activity."

The seam to exploit: **everything from `egl.rs` down is just an
`ANativeWindow*`.** Today the Activity provides it. The boot model
makes the runtime provide it itself.

---

## End state

A privileged process (launched by `init.rc` in production, by a
`su`-run binary in dev) that:

1. allocates its own fullscreen `SurfaceControl` from SurfaceFlinger,
2. reads input from InputFlinger,
3. runs the WASM app(s),

with SystemUI / the Java app stack `stop`-ed so the runtime owns the
screen.

---

## Steps

### Step 1 — Standalone-surface spike  *(keystone de-risk — start here)*

The single biggest unproven assumption: **can the runtime get a
display surface without an Activity?** Prove it minimally before
anything else.

- A standalone binary (or a `--standalone` launch mode of wart-host)
  with a plain `main()`, launched via `adb shell su -c …` — **not** a
  `NativeActivity`.
- Create a fullscreen `SurfaceControl` directly from SurfaceFlinger
  via `SurfaceComposerClient` — use a small C++ shim in `cpp/` linking
  **`libgui`** (`SurfaceComposerClient::createSurface` →
  `SurfaceControl` → `getSurface()` → an `ANativeWindow*`). Do **not**
  reimplement the `BufferQueue`/`IGraphicBufferProducer`/`gralloc`
  producer by hand — libgui does it.
- Set the layer fullscreen, top z-order, visible.
- Hand the `ANativeWindow*` to the existing `egl.rs` path and render
  one frame (a solid color is enough; the full skia path if cheap).

**Verify:** a frame appears on the physical display, from a process
that was never a `NativeActivity`. If this fails, the showstopper is
found at the cheapest possible point.

**✅ DONE — device-verified 2026-05-21.** `cpp/sf_probe.cpp` (in the wart
repo at `wart-host/cpp/sf_probe.cpp` + `sf_probe.bp`) runs clean on the
Pixel 2 XL: `SurfaceComposerClient initCheck=0` → `createSurface ok` →
transaction (top z-order, shown) → EGL → `eglSwapBuffers` → **solid blue
frame fills the whole panel** for 10 s, from a non-`NativeActivity`
`su`-run process. No SIGILL, no SELinux denial. The post-ART display path
is proven.

Build method (the out-of-tree libgui-header approach was abandoned — AIDL
*and* HIDL codegen fan-out makes it infeasible; `libgui` must be compiled
in an AOSP source tree):

- Build host `a-03` (128 GB / 72 core) holds a LineageOS 22.2 tree at
  `~/android/lineage`. `sf_probe` is a soong `cc_binary` at
  `external/sf_probe/` (`Android.bp` = `wart-host/cpp/sf_probe.bp`).
- Lunched **generic `aosp_arm64-trunk_staging-userdebug`** (not
  `lineage_taimen` — that needs proprietary vendor blobs). A generic lunch
  on a LineageOS tree needs three fixes:
  1. neutralize the two `lineage_generator` kernel-header modules in
     `vendor/lineage/build/soong/Android.bp` (they reference kernel
     make-vars only defined for a device build);
  2. `DISABLE_DEXPREOPT_CHECK=true` (LineageOS system-server-jar check);
  3. `BUILD_BROKEN_DUP_RULES := true` in
     `build/make/target/board/generic_arm64/BoardConfig.mk` (LineageOS vs
     generic-product duplicate install rules, e.g. `apns-conf.xml`).
- `breakfast`/`extract-files` artifacts (`device/google/{taimen,wahoo,
  gs-common}`, `kernel/google/wahoo`, `vendor/google/taimen`,
  `packages/apps/ElmyraService`) moved to `~/android/_trimmed` so the
  generic lunch stays clean.
- Output: `out/target/product/generic_arm64/system/bin/sf_probe` — 51 KB
  aarch64 PIE ELF. Built-against generic AOSP-15 `libgui` runs fine on the
  device's LineageOS-22.2 SurfaceFlinger — the libgui-ABI worry (M1/M4) is
  empirically settled.

### Step 2 — Decouple the host from `NativeActivity` / winit-Android

**🟡 partially done 2026-05-22** — the standalone scaffold landed together
with Step 1's wart-host integration: `wart-host --standalone` is a plain
`main()` path (no `android-activity`, no winit `EventLoop`) that acquires
its surface from the `libsf_surface` shim and runs a render+pace loop —
**device-verified drawing the renderer test frame on the Pixel 2 XL**.
Files: `wart-host/src/standalone.rs`, `src/sf_surface.rs` (dlopen wrapper
for `libsf_surface.so`), `SkiaRenderer::from_native_window`
(`canvas_impl.rs`), `main.rs` `--standalone` dispatch. The `libgui` shim is
`cpp/sf_surface.{cpp,bp}`, built in-tree as a soong `cc_library_shared`.
**✅ cwasm render loop done 2026-05-22** — `standalone.rs::run_cwasm_loop`
instantiates the component and drives `call_render_frame` + scheduler +
lifecycle with no winit; device-verified running the full Compose PoC at
60 fps from a non-Activity process.

**✅ orientation RESOLVED 2026-05-22 — UI renders upright, crisp, correct
aspect, no global side effects.**

*Root cause.* EGL on this device (taimen) always hands the producer a
buffer whose axes are the **transpose** of the `createSurface` dimensions
— `createSurface(1440×2880)` yields a `2880×1440` landscape buffer, which
mismatches the portrait panel. `setBuffersGeometry` cannot override it.

*Approaches tried and rejected:*
- **`setBuffersTransform(ROT_90/270)`** — SurfaceFlinger on this device
  composites `ROT_90` and `ROT_270` identically; it does not honour the
  per-buffer transform as a composition rotation. Dead end.
- **`setDisplayProjection(ROTATION_90)` + landscape layer stack** — rotates
  correctly, but `setDisplayProjection` is a **global** display change: it
  rotates the launcher / SystemUI too. Wrong mechanism for a guest layer.
- The earlier 4-way `WART_ORIENT` base-matrix render-rotate — only ever
  reaches ±90° / mirrored, never upright (confirmed again).

*The fix (shipped).* Exploit the transpose instead of fighting it:
`createSurface(2880×1440)` (landscape dims) → EGL hands back a
`1440×2880` **portrait** buffer that matches the portrait panel 1:1.
SurfaceFlinger composes it with no rotation and no scaling; the guest
renders a portrait UI with a plain **identity** base matrix. Step 1 of the
shim keeps a `setDisplayProjection(ROTATION_0)` *identity* projection
(rotates nothing — only resets stale display state, since that state
persists across process exit). Device-verified: upright, crisp, fills the
panel width.

Files: `cpp/sf_surface.{cpp,h}` (swapped `createSurface` dims, `ROTATION_0`
identity projection, `out_transform`), `src/sf_surface.rs` (`CreateFn`
3-arg + `transform` field), `src/standalone.rs` (passes transform, drives
the guest `on-resize` once after instantiate), `src/canvas_impl.rs`
(`from_native_window` gains a `transform_hint` param + quarter-turn base
matrix + `begin_frame` clears to opaque black).

**🟡 Still open in Step 2 (deferred — belongs with Steps 4–5 "own the
display"):**
- *Transparent lower region.* Where Compose doesn't paint, the `wart`
  `SurfaceControl` is transparent and the launcher composites through.
  `eLayerOpaque` is now set via `Transaction::setFlags` and `begin_frame`
  clears to black, but the launcher still bled through in testing — HWC
  shows an odd `sourceCrop=[58 116 1222 2444]` (SF sampling a sub-rect of
  the buffer). Needs a separate look.
- *Content fills only ~top half.* The app boots with a `BasicTextField`
  focused + soft keyboard, laid out top-aligned; the Compose scene IS
  correctly `1440×2880`. Cannot verify other states — no standalone input
  until Step 3 (InputFlinger).
- *App rotates with the OS.* When another app + the accelerometer rotate
  the display, the `wart` layer rotates too — because we are a guest layer
  in the OS-owned display, not the display owner. Expected at this stage;
  resolved when the runtime owns the display (Steps 4–5).

**✅ no-regression confirmed 2026-05-22** — the `NativeActivity` APK
rebuilt, installed, and ran upright on the Pixel 2 XL; `render_frame`
steady, the shared `begin_frame` clear-to-black is harmless there (Compose
overdraws it). Step 2 is fully closed.

- Add a launch mode with a plain `main()` and **no `android-activity`
  / no winit-Android `EventLoop`**. The host's per-frame loop becomes
  a plain render+poll loop (the desktop path is the reference).
- Keep the `NativeActivity` mode as a build mode — the PoC/demo and
  desktop still use it. Standalone is **additive**, not a replacement.
- cwasm: the standalone process has no `AssetManager`; it must load
  the cwasm from the filesystem (the host already prefers filesystem —
  just ensure the APK-asset fallback isn't on the standalone path).

### Step 3 — Input acquisition from InputFlinger

🟡 **scoped 2026-05-22 — plan below, not yet implemented.**

No Activity ⇒ no winit input. The host side is already done: `src/input.rs`
`dispatch_pointer_v2` / `dispatch_key_v2` feed the guest's
`on-pointer-event-v2` / `on-key-event-v2` WIT exports (the winit path uses
exactly these). Step 3 is only: get events into `standalone.rs`'s loop.

#### Approach A vs B — research outcome

**Approach A — InputFlinger input channel (CHOSEN).** AOSP ships an exact,
working reference for a *native, non-Activity* process doing this:
`frameworks/native/libs/gui/tests/EndToEndNativeInputTest.cpp` (class
`InputSurface`). That de-risks it — the recipe is known-good. It also reuses
InputFlinger's transport / batching / focus / key handling rather than
re-implementing them, fits the project's in-tree-C++-shim pattern
(`sf_surface`), and survives Step 4 (Step 4 stops SystemUI + launcher, not
`system_server` — InputFlinger stays up).

**Approach B — direct `/dev/input/event*` (evdev), rejected for now.** Pure
Rust, no shim, no binder, no `system_server` dependency — genuinely
appealing for the long-term boot model. But it means re-implementing the
multitouch protocol-B (slot) decode, raw-coordinate scaling, and evdev→
Android keycode mapping; and InputFlinger's hit-test/batching is lost.
Keep B as the **fallback** if SELinux blocks the `su`-domain process from
the `inputflinger` binder or the input-channel socket.

#### Approach A — concrete recipe (from `EndToEndNativeInputTest.cpp`)

New in-tree C++ shim `cpp/sf_input.{cpp,bp}` (soong `cc_library_shared`
`libsf_input`, alongside `libsf_surface` in `external/sf_input/` on a-03;
`shared_libs: libgui libinput libutils libbinder liblog`). `wart-host`
`dlopen`s it like `sf_surface`. The shim, given the `SurfaceControl` the
`sf_surface` shim already created:

1. `sp<IInputFlinger> if = interface_cast<IInputFlinger>(
   defaultServiceManager()->waitForService(String16("inputflinger")))`.
2. `if->createInputChannel("wart channel", &channelCore)` →
   `InputChannel::create(std::move(channelCore))` → the client channel.
3. Build `sp<gui::WindowInfoHandle>`: `token =
   channel->getConnectionToken()`, `name`, `dispatchingTimeout = 5s`,
   `globalScaleFactor = 1.0`, `touchableRegion.orSelf(Rect(0,0,PW,PH))`,
   an `InputApplicationInfo` with a fresh `BBinder` token.
4. `SurfaceComposerClient::Transaction()
   .setInputWindowInfo(sfSurfaceControl, windowInfoHandle)
   .setFocusedWindow(FocusRequest{token,name,...}).apply(true)` — so
   InputDispatcher routes touches in our region to the channel and key
   events to us as the focused window.
5. `InputConsumer consumer(channel)`. Expose a C ABI
   `sf_input_poll(out_events*, max) -> count` that does a non-blocking
   `consume(&factory, /*consumeBatches=*/true, frameTime, &seq, &ev)` loop
   (poll the channel fd with timeout 0), `sendFinishedSignal(seq,true)` per
   event, and flattens each `MotionEvent`/`KeyEvent` into a small POD
   struct (`kind`, `pointer_id`, `x`, `y`, `pressure`, `key_code`).

Rust side:
- `src/sf_input.rs` — `dlopen` wrapper (mirror of `sf_surface.rs`); the
  shim needs the `ANativeWindow*`/`SurfaceControl` handle, so `sf_surface`
  must also hand back the `SurfaceControl` (add an out-param or a getter).
- `src/standalone.rs` — once per loop iteration, call `sf_input_poll`,
  translate each POD event through `input::dispatch_pointer_v2` /
  `dispatch_key_v2` before `call_render_frame`.

#### Risks / unknowns to verify during implementation

- **SELinux:** the `su`-domain process calling the `inputflinger` binder
  and reading the input-channel socket may hit AVC denials — pull `logcat`
  + `dmesg` (use `rsbinder-triage`). If hard-blocked, fall back to B.
- The `sf_surface` shim currently returns only the `ANativeWindow*`; it
  must also expose the `SurfaceControl` (`g_control`) for
  `setInputWindowInfo`. Either a new getter export or merge the input
  setup into `sf_surface` itself.
- Coordinates: InputFlinger reports in display space; our surface is the
  full panel at `1:1`, so no scaling — but confirm against the
  identity-orientation path.
- Key events need the window focused (`setFocusedWindow`) and an
  `InputApplicationInfo`, else InputDispatcher drops them / ANRs.

#### Implementation progress — Step 3 input ✅, display clip ✅, orientation ⏳

##### ✅ Step 3 — input routing (device-verified 2026-05-22)

Approach A, folded into the `sf_surface` shim: `register_input_window()`
does `waitForService("inputflinger")` → `createInputChannel` →
`InputChannel::create` → `WindowInfoHandle` → `setInputWindowInfo(g_control)`
+ `setFocusedWindow`; `sf_input_poll()` drains `InputConsumer::consume` into
a `SfInputEvent[]` POD; `src/sf_surface.rs` exposes `poll_input()`;
`standalone.rs`'s loop dispatches via `input::dispatch_pointer_v2`.

Input geometry was initially empty (`frame=[0,0][0,0]` → taps dropped as
`ACTION_OUTSIDE`) because `g_control` carried no buffer. That is resolved
as a side-effect of the display fix below: `g_control` now carries the
buffer directly, so SurfaceFlinger derives the input window geometry from
the buffer (`frame=[0,0][1440,2880]`). `dumpsys input` shows
`channelName='wart input', status=NORMAL`, the window `[TOUCHED]`, no AVC
denials. Touch routes to the guest; coordinates pass straight through
(portrait `1440×2880`, identity) — verified by the user (scrolling works).

##### ✅ Display geometry — the `1440×1440` clip is fixed

Symptom: the guest rendered a full portrait UI but only a `1440×1440`
square (top-left) reached the panel; the rest showed the launcher.

Root cause (two layers): `g_control->getSurface()` spins up a
`BLASTBufferQueue` whose buffer lands on an internal **child**
`SurfaceControl`, clipped to `g_control`'s bounds. With `g_control`
landscape and the buffer portrait, the clip collapsed to their `1440×1440`
overlap.

Fix shipped in `cpp/sf_surface.cpp` (device-verified): attach the
`BLASTBufferQueue` **directly to `g_control`** instead of calling
`getSurface()` — `sp<BLASTBufferQueue>::make("wart", g_control, PW, PH, fmt)`
then `g_bbq->getSurface(true)`. One layer, no parent/child clip. Plus
`setFixedTransformHint(g_control, 0)` so SurfaceFlinger composites the layer
full-portrait. Result: `dumpsys SurfaceFlinger` shows one `wart` layer, HWC
composites `0 0 1440 2880` — **full screen, no clip**.

##### ⏳ Remaining — the guest UI is rotated 90° (host-side task)

**The EGL buffer transpose on taimen is unconditional.** Verified
exhaustively: `eglQuerySurface` always returns the *transpose* of the
requested buffer size, regardless of `createSurface` dims, BBQ size, or the
transform hint (`setFixedTransformHint` **and** the client-cache
`g_control->setTransformHint(0)` — which the BBQ provably reads — both had
zero effect). The Adreno/EGL driver prerotates off the physical display
install orientation, not the layer hint. **No shim-side lever disables it.**

This couples orientation and crop:
- BBQ landscape → EGL buffer portrait → host renders identity, **upright**,
  but the BBQ crops to `1440×1440` (BBQ size ≠ buffer).
- BBQ portrait → EGL buffer landscape → BBQ size == buffer, **full screen,
  no crop**, but the host must render into a landscape buffer and the UI
  ends up **90° rotated**.

The shim currently ships the **portrait** config (full screen, no clip,
rotated 90°) — the better trade. Closing the rotation is now a **host-side
renderer task**, not a shim one:

- The host (`wart-host/src/canvas_impl.rs`, `SkiaRenderer::from_native_window`,
  ~lines 350–415) gets a **landscape `2880×1440`** EGL buffer and must render
  the **portrait `1440×2880`** guest UI into it so the final on-panel image
  is upright.
- The `WART_ORIENT` env knob (0–3 quarter-turns; `from_native_window` reads
  it, defaults to 1 when buffer dims look swapped) was iterated on device:
  `1` and `3` both land **90° off** (opposite-handed); `0` and `2` force a
  **landscape guest layout** (logical = physical). None give upright
  portrait — the `orient` base-matrix logic conflates "swap logical dims"
  with "rotation amount" and can't express "portrait logical `1440×2880` +
  the rotation a prerotated landscape buffer needs."
- Next session: rework the `orient` match block (the `base_matrix`,
  `logical_width/height` tuple) so a portrait logical canvas can pair with
  a 0°/180°-class mapping into the landscape buffer, then iterate
  `WART_ORIENT` on device until the screenshot is upright. Pair the result
  with the shim's pinned `ROT_0` hint. Consider whether the guest's input
  coordinate space needs a matching transform (currently identity
  passthrough — correct only while the guest is portrait `1440×2880`).

**Lead — Android pre-rotation (`developer.android.com/games/optimize/`
`vulkan-prerotation`).** This validates the current shim config and names
the fix. The doc's model: a high-performance app renders into a buffer
fixed at the **panel-native (identity) orientation** — landscape `2880×1440`
on taimen — and applies a **rotation matrix** to its content matching the
device's reported transform; the compositor then does *no* rotation. That
is exactly the current shim's BBQ-portrait config (landscape buffer, full
screen, no crop) — **so do not revert the shim; it is the correct setup.**
The fix is to *match* the rotation, not guess it:
  - **Stop guessing `WART_ORIENT`.** The doc reads the surface's actual
    transform — `VkSurfaceCapabilitiesKHR.currentTransform` in Vulkan; the
    EGL/GLES analogue is the `ANativeWindow` transform hint
    (`NATIVE_WINDOW_TRANSFORM_HINT`, an `ANativeWindowQuery` from
    `system/window.h`). The renderer should read the real value and rotate
    by it. Query it **after EGL connects** (the hint is not populated until
    the producer connects) — `egl.rs`/`canvas_impl.rs` already holds the
    `ANativeWindow*`; do `nativeWindow->query(NATIVE_WINDOW_TRANSFORM_HINT,
    &v)`. (Optionally also have the shim plumb it through `out_transform`,
    which is currently hard-coded 0.)
  - **Rotation direction:** the doc rotates the content MVP by the *same*
    angle as `currentTransform` (`ROTATE_90` → `glm::rotate(…, 90°)`), with
    the framebuffer kept at identity (landscape) extent and the full
    viewport. That `1` and `3` both landed 90° off suggests the host's
    `base_matrix` is the wrong sign/pivot for this case — re-derive it from
    the doc's "rotate content by the hint, framebuffer stays identity"
    rule rather than the existing guess-matrices.

**Current shim state** (`cpp/sf_surface.cpp`, all device-verified to build
+ run): `createSurface(name, PW, PH, …)` portrait; transaction does
`setLayer(0x7fffffff)` + `setFixedTransformHint(0)` + `setFlags(eLayerOpaque)`
+ `show`, `apply(true)`; then `g_control->setTransformHint(0)`; then
`BLASTBufferQueue::make("wart", g_control, PW, PH, fmt)` +
`getSurface(true)`; `register_input_window(PW, PH)`. Globals add
`sp<BLASTBufferQueue> g_bbq` (released in `sf_destroy_surface`).
`#include <gui/BLASTBufferQueue.h>` added.

**Build / deploy / test** (build host `a-03`):
```
# SSH master (fast repeated calls):
ssh -i ~/.ssh/id_rsa.my -o ControlMaster=auto -o ControlPath=/tmp/cm-a03-%r \
    -o ControlPersist=30m harry@a-03 'echo up'
# Build the shim — `m libsf_surface` FAILS in legacy-make dexpreopt_check
# (unrelated LineageOS odex). Build the soong module directly:
scp .../cpp/sf_surface.cpp harry@a-03:~/android/lineage/external/sf_surface/sf_surface.cpp
ssh harry@a-03 'cd ~/android/lineage && \
  SO=out/soong/.intermediates/external/sf_surface/libsf_surface/android_arm64_armv8-a_shared/libsf_surface.so && \
  prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja "$SO"'
#   (must be the COMBINED ninja — build.aosp_arm64.ninja alone fails: unknown pool highmem_pool)
# Deploy:
scp harry@a-03:.../libsf_surface.so /tmp/ && adb shell "su -c 'pkill -f wart-host'"
adb push /tmp/libsf_surface.so /data/local/tmp/libsf_surface.so
adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp /data/local/tmp/wart-host --standalone'"
# Inspect: adb shell dumpsys SurfaceFlinger | grep wart   (HWC line = composite rect)
#          adb shell screencap -p /sdcard/s.png && adb pull /sdcard/s.png
# WART_ORIENT iteration: prefix the run with  WART_ORIENT=<0-3>
```

**Checkpoint for the orientation fix:** a screencap showing the guest UI
upright and full-screen, the counter (`+`/`−`) visible at the top.
NativeActivity-APK no-regression check also still pending.

### Step 4 — Launch mechanism + SystemUI coexistence

- **Dev:** `su`-run binary from `/data/local/tmp` — reversible, easy
  to iterate.
- **Production:** an `init.rc` service entry (later — needs a sepolicy
  domain).
- `stop` SystemUI (and the launcher) so the runtime owns the screen.
  **Document the exact `stop`/`start` commands and the recovery
  path** (`start` them back, or reboot) so a wedged device is always
  recoverable.

### Step 5 — Lifecycle / minimal arbiter

- One app, fullscreen — the ActivityManager-equivalent is mostly the
  existing `LocalLifecycleOwner` bridge (`feedback_lifecycle_owner_bridge`).
- Drive created/resumed/paused from the runtime itself (no Activity
  callbacks). The multi-app arbiter is **out of scope** for this task
  (monolithic, single app).

---

## Known issues / constraints

- **winit `EventLoop`** is once-per-process and Activity-coupled — the
  `RecreationAttempt` panic (`lib.rs` `android_main` `.build().unwrap()`).
  The standalone path sidesteps winit-Android entirely; do not try to
  reuse the winit Android backend without an Activity.
- **`SurfaceComposerClient::createSurface`** needs `ACCESS_SURFACE_FLINGER`
  — root provides it in dev; the `init.rc` service needs a privileged
  SELinux domain (Step 4 / later).
- **Use `libgui`** for surface creation — the BufferQueue/gralloc
  plumbing is non-trivial; don't hand-roll it via raw AIDL.
- **SELinux:** a `su`-run binary inherits a workable context for dev;
  a proper sepolicy domain for the `init.rc` service is later work.
- **Do not regress the `NativeActivity` build mode** — the PoC/demo
  and desktop builds depend on it. Standalone mode is additive.
- **DRC GC — the long-lived standalone runtime inherits an
  upstream-unsolved problem.** wasmtime's DRC collector never
  auto-triggers a sweep, so `standalone.rs::run_cwasm_loop` (a forever
  loop, no `Store::gc`) grows unbounded → OOM on a heavy Compose guest
  (~9 MB/s). The upstream auto-GC fix
  ([wasmtime#13403](https://github.com/bytecodealliance/wasmtime/issues/13403)
  / PR#13422) is **NOT a drop-in fix**: device-tested 2026-05-21 it
  bounds the heap but reintroduces screen lag / a fresh ANR — the render
  thread stalls in `force_gc` → `trace_vmctx_roots` root scans (the
  guest has many GC globals). It trades unbounded memory for unbounded
  GC-frequency overhead. So the standalone runtime is demo-usable for
  short sessions (like the `NativeActivity` PoC) but **not yet a 24/7
  host**; this is an upstream dependency, tracked in
  `post-art-roadmap.md` §12 — see memory `wasmtime-drc-no-autoschedule`.
  Do not treat #13422 as a quick win. (Also: monolithic means one app's
  GC stall freezes all — relevant once >1 app runs concurrently.)

---

## Estimates

| Step | Wall time |
|------|-----------|
| 1. Standalone-surface spike | 3–5 days |
| 2. Decouple from NativeActivity/winit | ~1 week |
| 3. InputFlinger input | ~1 week |
| 4. Launch mechanism + SystemUI stop | 2–3 days |
| 5. Lifecycle / minimal arbiter | 2–3 days |
| **Total** | **~3–4 weeks** to a standalone, interactive runtime |

---

## Verification checklist

- [x] Step 1 — a frame on the physical display from a non-Activity
      `su`-run process. ✅ device-verified 2026-05-21 (solid blue frame).
- [ ] Step 2 — standalone mode runs the render loop with no
      `android-activity` dependency; NativeActivity mode still builds
      and works.
- [x] Step 3 — touch from InputFlinger reaches the guest (device-verified
      2026-05-22; scrolling works). Display `1440×1440` clip fixed
      (BBQ-direct, full-screen composite). ⏳ guest UI still rotated 90° —
      host-side `canvas_impl.rs` orientation rework outstanding (see
      Step 3 "Implementation progress" above). Key events not yet wired.
- [ ] Step 4 — documented launch + SystemUI-stop + recovery path;
      runtime owns the screen.
- [ ] Step 5 — lifecycle (resume/pause) driven without Activity
      callbacks.
- [ ] No regression — NativeActivity APK still boots and renders.

---

## First action for a fresh session

**Steps 1–2 done; Step 3 input + display-clip done; in progress: the
guest UI is rotated 90°.** Resume at the **Step 3 "Implementation
progress → ⏳ Remaining"** subsection above — it is self-contained: the
diagnosis (unconditional EGL transpose on taimen), the current shim
state, the host-side task in `wart-host/src/canvas_impl.rs`
(`from_native_window` orient base-matrix rework), the `WART_ORIENT`
iteration findings, and the full build/deploy/test commands. The shim
(`cpp/sf_surface.cpp`) needs no further change for the rotation fix —
the work is in the Rust renderer.

For the original background, `post-art-roadmap.md` §5, §5.1, §6.1,
§9, §11.
