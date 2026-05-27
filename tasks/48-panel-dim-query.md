# Task 48 — Query panel dimensions instead of hardcoding

> **Status:** 🔲 scoped 2026-05-27, not started. Spun out of task 47
> step 3c as a follow-up — the step-3c work added
> `constexpr PANEL_W=1440 / PANEL_H=2880` to `cpp/sf_surface.cpp` so
> the new overlay path could compute `Y = PANEL_H - height_px` without
> reaching into `sf_create_fullscreen_surface`'s locals. Same
> taimen-specific hardcoding that's been in the shim since task 33;
> the surface area just grew from one function to two.

## Why this task exists

`cpp/sf_surface.cpp:81-82` (post-step-3c):

```cpp
constexpr uint32_t PANEL_W = 1440;
constexpr uint32_t PANEL_H = 2880;
```

Pinned to Pixel 2 XL (taimen) portrait dimensions. The original TODO
sits in the same file (look for the `// TODO(task33): query the mode
instead of hardcoding.` comment inside `sf_create_fullscreen_surface`).
Until this is fixed, the wart standalone path will compose at
1440×2880 on any device — wrong for everything except taimen.

Not currently blocking anything (the whole boot-model stack is
taimen-only MVP), but a load-bearing assumption to lift when adding a
second target device.

## Approach

`SurfaceComposerClient` exposes display-mode metadata once the
display token has been resolved. From the existing flow:

```cpp
g_display = SurfaceComposerClient::getPhysicalDisplayToken(ids[0]);
```

After that succeeds, query the active mode:

```cpp
ui::DisplayMode mode;
status_t st = SurfaceComposerClient::getActiveDisplayMode(g_display, &mode);
if (st != OK) { /* fall back to PANEL_W/PANEL_H or bail */ }
PANEL_W_runtime = mode.resolution.width;
PANEL_H_runtime = mode.resolution.height;
```

(The exact AOSP API name + signature is android-15-specific; pattern
to check is `frameworks/native/libs/gui/SurfaceComposerClient.cpp`
on the a-03 tree.)

Variables can no longer be `constexpr` — promote to `static` namespace
locals or pass through as helper output. Both `sf_create_fullscreen_surface`
and `sf_create_overlay_surface` need to call an `init_panel_dims()`
helper before referencing `PANEL_W`/`PANEL_H`.

## Considerations

- **Orientation.** taimen panel is landscape-native (2880×1440 physical)
  but reported as portrait 1440×2880 via the display mode after the
  `setDisplayProjection(ROTATION_0, Rect(PW,PH), Rect(PW,PH))` call in
  the create path. Verify which orientation `getActiveDisplayMode`
  returns and whether it changes pre/post projection setup.
- **Sanity bounds.** Reject implausible values (0×0, >8K) and fall
  back to the taimen constants with a logged warning. A degraded
  fullscreen surface beats a hard crash.
- **Multi-display.** Current code picks `ids[0]` for the physical
  display. Multi-display is out of scope; this task doesn't need to
  address it.
- **Cache the query.** Both create paths in `sf_surface.cpp` repeat
  the display-token resolution. Once dimensions are queried, store
  them so the resize path (`sf_resize_overlay`) doesn't re-query.
- **Rust side.** `wart-host/src/sf_surface.rs` receives the
  dimensions back via the existing `out_w` / `out_h` out-params. No
  Rust changes needed.

## Steps

1. **Identify the AOSP API** (~30 min). On a-03,
   `grep getActiveDisplayMode\|getActiveMode ~/android/lineage/frameworks/native/libs/gui/include/gui/SurfaceComposerClient.h`
   to confirm the exact signature in this AOSP version. Document the
   `DisplayMode` field shape (probably `.resolution.{width,height}` of
   type `ui::Size`).

2. **Add the helper** (~30 min). New static helper
   `bool init_panel_dims(const sp<IBinder>& display)` in the
   anonymous namespace. Queries the active mode, sanity-checks the
   resolution, stores into `static uint32_t g_panel_w / g_panel_h`,
   returns `true` on success. Falls back to 1440/2880 with `LOGE`
   on failure.

3. **Replace constants** (~15 min). `constexpr PANEL_W/H` → `static
   uint32_t g_panel_w = 1440 / g_panel_h = 2880` (initial values
   are the taimen fallback). Update all references — the create
   paths and the overlay-Y arithmetic.

4. **Wire into create paths** (~15 min). Both
   `sf_create_fullscreen_surface` and `sf_create_overlay_surface`
   call `init_panel_dims(g_display)` after the display token is
   resolved, before any size-dependent transactions.

5. **Build + smoke** (~30 min). Build on a-03, push, run the task-47
   step-3c smoke (`scripts/run-hybrid-stack.sh` + `wart-arbiter
   launch com.example.wart-app` + `launch-overlay war.ime.keyboard`
   + `overlay war.ime.keyboard`). Confirm the IME still lands at
   the bottom of the screen at the right size.

6. **Update memory + remove the original TODO** (~10 min). Mark the
   "TODO(task33)" comment in `sf_create_fullscreen_surface` resolved
   and update `project-boot-model-libgui-build` if relevant.

Total: ~2 hours focused work, all on the a-03 host.

## File-touch map

- `wart-host/cpp/sf_surface.cpp` — replace `PANEL_W`/`PANEL_H`
  constants, add `init_panel_dims`, wire into create paths.
- `tasks/48-panel-dim-query.md` — this doc; add a results section
  on completion.

(No Rust, no Kotlin, no WIT changes — purely the shim.)

## Related

- `tasks/33-boot-model-bringup.md` — original site of the
  hardcoded PW/PH locals.
- `tasks/47-ime-via-guest-app.md` step 3c — the work that promoted
  the local PW/PH into namespace `PANEL_W`/`PANEL_H` constants.
- `MEMORY.md` → `project-boot-model-libgui-build` — describes how
  `libsf_surface.so` builds on the a-03 host.

## Resume hints for fresh sessions

1. The whole change lives in ONE file (`cpp/sf_surface.cpp`).
2. The shim builds on the a-03 host — see
   `scripts/standalone-launch.sh` lines 60-66 for the exact ninja
   command pattern.
3. Smoke is `scripts/run-hybrid-stack.sh` + the arbiter-driven IME
   overlay sequence (in task 47 step 3c results section).
4. Don't touch the Rust side — `out_w` / `out_h` already carry
   dimensions back to `sf_surface.rs`.
