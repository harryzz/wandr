# Task 33 — Boot-model bring-up: run the runtime as a standalone privileged process

**Status:** 🔲 scoped — not started. This is a sub-roadmap; execute the
steps in order, each verifiable on the rooted phone.

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

### Step 2 — Decouple the host from `NativeActivity` / winit-Android

- Add a launch mode with a plain `main()` and **no `android-activity`
  / no winit-Android `EventLoop`**. The host's per-frame loop becomes
  a plain render+poll loop (the desktop path is the reference).
- Keep the `NativeActivity` mode as a build mode — the PoC/demo and
  desktop still use it. Standalone is **additive**, not a replacement.
- cwasm: the standalone process has no `AssetManager`; it must load
  the cwasm from the filesystem (the host already prefers filesystem —
  just ensure the APK-asset fallback isn't on the standalone path).

### Step 3 — Input acquisition from InputFlinger

- No Activity ⇒ no winit input. Consume from **InputFlinger's input
  channel** (roadmap §9 leaning — InputFlinger already does touch
  processing / key remapping / gesture detection; reading raw
  `/dev/input/event*` would reinvent it).
- Register an input target / obtain an `InputChannel`, consume events
  (`InputConsumer`), route to the guest's existing
  `on-pointer-event-v2` / `on-key-event-v2` WIT exports.
- Likely a C++ shim (`InputConsumer` is C++) or rsbinder.

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
- **DRC GC (`post-art-roadmap.md` §12):** not this task, but note —
  monolithic means one app's GC stall freezes all; relevant once >1
  app runs concurrently, not for this single-app bring-up.

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

- [ ] Step 1 — a frame on the physical display from a non-Activity
      `su`-run process.
- [ ] Step 2 — standalone mode runs the render loop with no
      `android-activity` dependency; NativeActivity mode still builds
      and works.
- [ ] Step 3 — touch + key events from InputFlinger reach the guest;
      tapping a Compose widget responds.
- [ ] Step 4 — documented launch + SystemUI-stop + recovery path;
      runtime owns the screen.
- [ ] Step 5 — lifecycle (resume/pause) driven without Activity
      callbacks.
- [ ] No regression — NativeActivity APK still boots and renders.

---

## First action for a fresh session

Read `post-art-roadmap.md` §5, §5.1, §6.1, §9, §11 for the decisions
this builds on, then start **Step 1** (standalone-surface spike). It is
bounded, on the existing rooted phone, and proves or kills the whole
post-ART display path at the cheapest point.
