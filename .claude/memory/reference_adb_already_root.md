---
name: reference_adb_already_root
description: "On the Pixel 2 XL dev device adb runs as root — no `su -c` wrapper needed"
metadata: 
  node_type: memory
  type: reference
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-04T18:26:39.955Z
---

The Pixel 2 XL (`804KPSL1724590`) dev device has `adb` already running as root, so
shell commands do NOT need a `su -c '...'` wrapper — run them directly:
`adb shell '/data/local/tmp/wandr-arbiter list'`, not
`adb shell "su -c '/data/local/tmp/wandr-arbiter list'"`. The `su -c` form still
works but is redundant, adds quoting hazards, and the user has asked to drop it.
The committed scripts (`run-hybrid-stack.sh` etc.) still use `su -c` internally —
that's fine, leave them; this applies to ad-hoc `adb shell` I run by hand.
Related: [[reference_wandr_apps_root_install]], [[reference_a03_ninja_build]].
