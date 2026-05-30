---
name: rebuild-compose-after-skiko
description: "If you rebuild/republish skiko-wasm-wasi, you MUST also rebuild every compose-*-wasi module that consumes it — symptoms of skipping are subtle behavioral drift (e.g. tap-latency accumulation), not link errors."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ade59596-71ca-44d3-bc3e-26f4f4ba5671
---

When the skiko-wasm-wasi klib is republished (via
`~/wart/skiko/skiko/gradlew publishWasmWasiPublicationToMavenLocal`),
every compose-*-wasi fat klib that depended on it must also be
republished against the new skiko. Otherwise the compose klibs still
contain inlined skiko code, stale function-reference targets, and any
other ABI-bound material from the previous skiko build.

**Why:** confirmed by `~/wart/compose-multiplatform-core/BUILD-wasmWasi.md`
Step 5: "When you rebuild skiko (`~/skiko/`) you MUST republish every
compose module that consumed it, otherwise the older klibs reference
symbols that have shifted in the skiko klib's ABI."

Earlier in session 2026-05-18 I told the user "not necessary" — that
was wrong. Symptoms of skipping the rebuild can be subtle (not link
errors): unexplained behavioral drift, tap-latency that accumulates
over a session, animations that look right at first then degrade. We
spent meaningful time investigating worker-thread + gc-trigger tuning
before noticing the staleness via mtime comparison.

**How to apply:** run
`~/wart/scripts/rebuild-compose-wasi-skiko-depend.sh` after every
skiko republish. The script runs against
`~/wart/compose-multiplatform-core/` and republishes the 9 dep-ordered
modules (`:compose:ui:ui-graphics` → `:compose:material3:material3`)
via `publishWasmWasiPublicationToMavenLocal`. End-to-end ~15-30 min;
fails fast on the first error.

After the script completes, also rebuild the wart-app to .wasm +
repackage as cwasm (CLAUDE.md "Build pipeline" steps 2-3).

**Quick freshness check** before chasing subtle bugs:

```bash
# latest skiko klib mtime:
stat -c '%Y %n' ~/.m2/repository/org/jetbrains/skiko/skiko-wasm-wasi/0.0.0-SNAPSHOT/*.klib

# any compose-*-wasi klib mtime:
stat -c '%Y %n' ~/wart/compose-runtime-wasi/build/libs/*.klib

# if compose < skiko, REBUILD compose first.
```
