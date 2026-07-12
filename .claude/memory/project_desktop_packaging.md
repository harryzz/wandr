---
name: project_desktop_packaging
description: "FUTURE idea (recorded, NOT started): make desktop install easy — ship apps as a zip package + bundle the host binary with its libs (ffmpeg) and an install script."
metadata:
  node_type: memory
  type: project
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

FUTURE direction, user-raised 2026-07-12 — **recorded only, explicitly NOT to be
started** ("no for now just ask for record"). Goal: easy desktop installation.

Today the flow is manual: build the host, `<host> --install <wandrpkg-dir>` into
`WANDR_APPS_ROOT`, and keep the host binary next to its runtime libs (Windows:
ffmpeg `avcodec-*.dll` on PATH; macOS: ffmpeg dylibs; Linux: system ffmpeg). The
`run-app-{macos.sh,windows.bat}` scripts already wrap the launch env
([[reference_host_build_scripts]]).

Idea, two parts:
1. **Apps as a zip package** — a distributable app bundle (the wandrpkg + assets)
   that installs with one step instead of the manual `--install`.
2. **Binary + libs + install script** — a self-contained bundle carrying the host
   binary, its bundled runtime libs (ffmpeg), and an install script that lays
   everything out, sets `WANDR_APPS_ROOT`, and creates a launcher — so a user
   unzips and runs one script, no build/PATH dance.

Per-platform packagers to consider when picked up: macOS `.app`/`.dmg`, Windows
MSI/zip, Linux AppImage/tar. Keep it resolution/host-independent — no per-app
hardcoding in the host (BINDING rule, [[feedback_no_hardcoding]]).
