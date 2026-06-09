---
name: adb-push-dir-nesting-gotcha
description: "`adb push <src-dir> <dest-dir>` when <dest-dir> already exists creates <dest-dir>/<src-dir-basename>/... — files appear at a NESTED path while same-named files at the top level go stale"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d9451151-9116-4c95-a45d-8758673104ce
---

When pushing a directory to a path that already exists as a directory:

```bash
# Source: /tmp/wandr-app.wandrpkg/components/ui.wasm
adb push /tmp/wandr-app.wandrpkg /data/local/tmp/wandr-app.wandrpkg
```

If `/data/local/tmp/wandr-app.wandrpkg` ALREADY EXISTS as a directory, `adb push` does NOT replace its contents. Instead it copies the source dir INSIDE the dest dir:

```
/data/local/tmp/wandr-app.wandrpkg/package.toml                          <- STALE (from a prior push)
/data/local/tmp/wandr-app.wandrpkg/components/ui.wasm                    <- STALE (from a prior push)
/data/local/tmp/wandr-app.wandrpkg/wandr-app.wandrpkg/package.toml          <- new content, wrong path!
/data/local/tmp/wandr-app.wandrpkg/wandr-app.wandrpkg/components/ui.wasm    <- new content, wrong path!
```

Any consumer (e.g. `wandr-host --install /data/local/tmp/wandr-app.wandrpkg`) reads from the top-level path → keeps using the stale content silently. `adb push` reports `2 files pushed, 0 skipped` and the sizes look right — it's just the wrong location.

**Why:** task 36 visuals v2 hit this 2026-05-26. The MarkdownCard rebuild's new v2 wasm was correctly built, embedded, adapted, and pushed — but `wandr-host --install` kept AOT-compiling the v1 wasm from the stale top-level path. Caught by `md5sum` mismatch between local `/tmp/wandr-app.wandrpkg/components/ui.wasm` and on-device `/data/.../wandr-app.wandrpkg/components/ui.wasm`.

**How to apply:**

- Before any `adb push <dir> <dir>` in a dev loop, **rm the dest first**:
  ```bash
  adb shell "su -c 'rm -rf /data/local/tmp/wandr-app.wandrpkg'"
  adb push /tmp/wandr-app.wandrpkg /data/local/tmp/wandr-app.wandrpkg
  ```
- Or push files individually with explicit destination paths — single-file `adb push` overwrites correctly.
- Whenever an iterate-and-verify loop shows OLD behavior despite a fresh build, **md5sum the on-device file** vs the local source. If they differ, suspect the nesting trap.
- This affects `scripts/smoke-markdown.sh`, `scripts/standalone-launch.sh`, and any other workflow that pushes wandrpkg dirs repeatedly. Add the `rm -rf` step to those scripts when iterating.

Related: [[task-36-step-7-pending]] (where this bit during visuals v2).
