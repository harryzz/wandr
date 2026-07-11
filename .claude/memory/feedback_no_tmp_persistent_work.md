---
name: feedback_no_tmp_persistent_work
description: NEVER put probes/tests/repros/working source in /tmp or any non-persistent folder — they get wiped on crash/restart and the work is lost.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 50ea929c-30b1-4a02-a37f-3cad3c8adbb2
---

NEVER create tests, probes, repros, or any working source files in `/tmp`, the
session scratchpad, or any folder that does not survive a crash / restart /
session boundary. Put them in the **persistent repo** (`repros/`, `tests/`,
`apps/`, etc.) and reference them by their persistent path.

**Why:** ~2 days of OpenSwiftUI-on-wasm work was lost because probes and the
clean-shim stack lived under `/tmp/{OpenSwiftUI,oag-fork,Compute,oag-shims}` and
got wiped. This was caused by me. It is not acceptable to lose work this way.

**How to apply:**
- Probe/repro/test SOURCE → always under `~/wandr/repros/...` or `~/wandr/tests/...`.
- If you find a dependency or build script pointing at `/tmp/...`, re-point it to
  a persistent path before building (this already bit the OpenSwiftUI demo — its
  `Package.swift`/build script were re-pointed from `/tmp/OpenSwiftUI` to
  `swift/OpenSwiftUIProject/OpenSwiftUI`).
- Throwaway build *logs* / scratch may use the scratchpad, but never the thing
  whose loss costs work (source, repro, accumulated findings).
- Capture ongoing diagnostic findings into a persistent file (e.g. `RESUME.md`)
  as you go, not only at the end — so a crash mid-investigation loses nothing.

See [[feedback_capture_build_output]] (don't lose build output) and the
OpenSwiftUI live state in `repros/openswiftui-wasm/RESUME.md`.
