---
name: feedback_dont_delete_app_cache_dir
description: "Never `rm -rf` an installed app's cache/ dir — host can't write cache/ui.cwasm → silent test-frame fallback that mimics a guest crash"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b4642c38-ac22-459b-92dd-7b4430418889
---

Do **not** `rm -rf` an installed warpkg's `cache/` directory
(`$APPS_ROOT/.../<app>/<ver>/cache/`) to "force a fresh AOT recompile". The host
precompiles the component (cranelift JIT visible in logcat) and then tries to
**write** `cache/ui.cwasm`; if the parent `cache/` dir is gone, the write fails
with `load failed (write .../cache/ui.cwasm: No such file or directory (os error
2)) — falling back to test-frame loop` (standalone.rs ~1033). The guest never
runs, the host draws "standalone: test frame N" forever, and **no per-host
control socket** (`wart-host-<pid>.sock`) is created → editor-attach delivery
fails → the IME shows nothing.

**Why:** this looks exactly like a guest instantiation crash, a poisoned store
(lib.rs:483 "cannot enter component instance"), or a skiko/compose ABI skew — so
you chase the wrong cause. The real cause is the missing directory; the cwasm
write is the only thing failing. (Burned ~an hour on task 71 mistaking it for
ABI skew.)

**How to apply:** to refresh a cwasm, **reinstall** the warpkg
(`pack-ime-keyboard.sh` / `--install`) — install recreates `cache/` *and*
precompiles `ui.cwasm`. If you must clear just the cwasm, delete the **file**
(`cache/ui.cwasm`), not the dir, or `mkdir` it back. First launch after a real
clear is slow (full on-device JIT of ~18k Compose functions); a "preload hit for
.../cache/ui.cwasm" + "loaded installed:<app>" log means the precompiled path is
working. Distinct from the genuine stale-AOT-cwasm SIGBUS (BUS_ADRALN at
0x100000002) which a reinstall *does* fix. Related: [[feedback_rebuild_compose_after_skiko]].
