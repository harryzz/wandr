---
name: reference_wslg_wayland_resize_crash
description: WSLg Wayland backend crashes the desktop host on window resize — force X11 (WINIT_UNIX_BACKEND=x11)
metadata: 
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

On WSLg, running the desktop host (`wasm-android-host --app <id>`) under the **Wayland** winit
backend **crashes on window RESIZE**: a burst of `[geom] Resized -> …` events, then
`Io error: Connection reset by peer (os error 104)` and `event_loop.run_app(...).unwrap()`
panics (`lib.rs` ~1863, `ExitFailure(1)`). The Wayland compositor connection drops mid-resize.

**Fix / workaround:** force the **X11** backend — `WINIT_UNIX_BACKEND=x11`. X11 works; Wayland is
the problem. So the desktop run command on WSLg should be:
```
WINIT_UNIX_BACKEND=x11 WANDR_APPS_ROOT=<apps> WANDR_DESKTOP_SIZE=WxH \
  wasm-android-host --app <app-id>
```
(This matches the desktop-dev-loop run command in the task-115 plan, which already set
`WINIT_UNIX_BACKEND=x11`.)

The rendering itself is fine under either backend; it's specifically resize + the Wayland
connection. Not a host logic bug — a WSLg/winit-Wayland issue. See `[[project_desktop_dev_loop]]`.
