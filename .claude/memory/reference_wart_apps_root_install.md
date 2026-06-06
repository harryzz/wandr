---
name: reference_wart_apps_root_install
description: Manual wart-host --install must set WART_APPS_ROOT to match the running stack root
metadata: 
  node_type: memory
  type: reference
  originSessionId: 8a79d726-5989-436c-93ca-fceb8f26e051
---

NORMALIZED 2026-06-06 (commit ec90f10f): `wart-host`'s in-code default
app-registry root is now `/data/local/tmp/wart-apps` (one resolver
`app_loader::apps_root()` + `DEFAULT_APPS_ROOT`), matching what every launcher
uses — so a bare `wart-host --install` (no env) now lands where the running stack
reads. **With a wart-host older than ec90f10f the default was `/data/wart/apps`**,
a DIFFERENT directory the stack never reads → installs silently had no effect
(cost a long detour on the launcher font change). On an old binary you MUST pass
`WART_APPS_ROOT=/data/local/tmp/wart-apps`. `WART_APPS_ROOT` still overrides for a
sepolicy'd production root or a test sandbox.

GOTCHA: `adb push` of the wart-host binary resets execute permission → new
invocations fail `can't execute: Permission denied` (rc 126). Always
`chmod 755 /data/local/tmp/wart-host` after pushing (run-hybrid-stack does this;
a manual push must too). Pushing over the live binary is safe — running processes
keep the old (now-unlinked) inode; only new spawns use the new file.

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
