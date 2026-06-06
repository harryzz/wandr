---
name: reference_wart_apps_root_install
description: Manual wart-host --install must set WART_APPS_ROOT to match the running stack root
metadata: 
  node_type: memory
  type: reference
  originSessionId: 8a79d726-5989-436c-93ca-fceb8f26e051
---

When hot-installing a single warpkg with `wart-host --install` on a device whose
stack was brought up by `run-hybrid-stack.sh`, you MUST pass
`WART_APPS_ROOT=/data/local/tmp/wart-apps` — that is the `APPS_ROOT` the script
sets and every running host/zygote loads from. The installer's DEFAULT root is
`/data/wart/apps`, a DIFFERENT directory the running stack never reads. Installing
without the env writes the new component to `/data/wart/apps/...` while the live
launcher keeps loading the old one from `/data/local/tmp/wart-apps/...` — the
update silently has no effect.

Correct one-app reinstall (e.g. launcher):
```
adb shell "su -c 'WART_APPS_ROOT=/data/local/tmp/wart-apps \
  LD_LIBRARY_PATH=/data/local/tmp /data/local/tmp/wart-host --install \
  /data/local/tmp/<pkg>.warpkg'"
adb shell "su -c '/data/local/tmp/wart-arbiter kill <app-id>'"
adb shell "su -c '/data/local/tmp/wart-arbiter preload <app-id>'"   # re-preload new disk
adb shell "su -c '/data/local/tmp/wart-arbiter launch <app-id>'"
```
Diagnose a "my change didn't show" by reading `/proc/<pid>/maps` of the running
guest host — the mapped `.cwasm` path reveals BOTH the root AND the version
actually loaded (it showed `/data/local/tmp/wart-apps/...` while I'd installed to
`/data/wart/apps/...`). `preload` re-reads disk + the kill→preload→launch cycle
hot-reloads WITHOUT a full ART restart, as long as the correct root is updated
first. `adb push <dir> <existing-dir>` NESTS (push into a stale target dir creates
`x/x/...`) — `rm -rf` the remote target before pushing. See [[reference_a03_ninja_build]].
