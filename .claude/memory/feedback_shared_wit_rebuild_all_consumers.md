---
name: feedback_shared_wit_rebuild_all_consumers
description: "Changing a SHARED WIT type breaks the ABI for EVERY guest importing it — rebuild all consumers, not just the host + the app you're on. Stale guest = instantiate trap 'type mismatch, expected record of N fields, found M'."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7d7dad2e-750c-4658-9e32-fc4c95e9f48e
---

**RULE: when you change a shared WIT interface/type, REBUILD EVERY GUEST whose
world imports it — not just the host and the one app you're working on.** A guest
component bakes the WIT ABI in at build time; if the host's interface changes
(e.g. a record gains a field) and a guest isn't rebuilt, it fails to instantiate
with: `linker.instantiate failed: component imports instance ...,
<fn> has the wrong type: type mismatch with parameters: expected record of N
fields, found M`.

**Why (2026-06-04):** the audio `stream-class` refactor (commit a965bf0e) added a
`class` field to `audio.track-config` in `wit/skiko-gfx.wit` (3→4 fields). It
correctly mirrored the WIT *source* to every consumer's `wit/deps/` AND rebuilt
the host + Signal — but did NOT rebuild the **IME** (`wandr.ime.keyboard`) or
**wandr-app**. Those crashed at launch with "expected record of 4 fields, found 3"
because `create-track` is in `interface audio`, which the `skiko-ui` world
`import`s, and BOTH apps' worlds `include my:skiko-gfx/skiko-ui`. WIT-source-sync
≠ done; the embedded ABI is per-built-component.

**How to apply:**
- After editing `wit/skiko-gfx.wit` (or any shared WIT), find every guest whose
  world imports the changed interface: `grep -rl skiko-ui apps/*/*/wit/*.wit`
  (skiko-ui imports canvas+audio+…); each must be rebuilt.
- The skiko-ui consumers as of 2026-06-04: `apps/user/wandr-app`,
  `apps/system/wandr.ime.keyboard`, `apps/user/wandr.signal/ui`.
- Rebuild a Kotlin/Compose guest: `cd <app> && ./gradlew
  compileProductionExecutableKotlinWasmWasi` then its per-app pack script
  (IME → `tools/scripts/pack-ime-keyboard.sh`, which does
  embed→component-new→`wandr-host --install`, NO apps-root wipe).
- **Do NOT** use `build-system-wandrpkgs.sh` to fix one stale app — it `rm -rf`s the
  apps root and destroys Signal state. See [[feedback_build_system_wandrpkgs_wipes_apps_root]].
- Diagnose the trap by launching the guest via the arbiter and reading the
  `wandr-zygote/child: standalone failed: linker.instantiate failed: …` line in
  logcat — it names the interface + the field-count mismatch exactly.
Related: the WIT-sync rule in CLAUDE.md, [[feedback_read_source_first]].

**TWO extra traps this hunt exposed (2026-06-04, fixing the IME):**
- **skiko's Kotlin WIT binding is HAND-MAINTAINED, not WIT-generated.**
  `external/skiko/skiko/src/wasmWasiMain/kotlin/generated/{SkikoUi,InternalSkikoUi}.kt`
  are "generated" in name only — no generator in the repo. The audio refactor
  updated the WIT + Rust host + Signal (dioxus auto-regen) but the skiko
  `create-track`/`TrackConfig` Kotlin binding stayed 3-field, so Compose/skiko
  guests (IME, wandr-app) couldn't `component embed` (core module imported 3
  params vs 4-field WIT). FIX = edit those files by hand to match the WIT
  (added `StreamClass` enum + defaulted `streamClass` field + 4th import param),
  then republish: `cd external/skiko && ./gradlew
  :skiko:publishWasmWasiPublicationToMavenLocal`, then rebuild the consumer.
  Signal's UI was immune because it's dioxus-canvas (Rust bindgen), not skiko.
- **The wandr-zygote PRELOADS guest components in memory — a stack restart is
  required after rebuilding a guest.** After installing the new 4-field IME, the
  loader still logged `preload hit` and forked the STALE 3-field component the
  zygote had cached at its first (failed) launch — on-disk component + cwasm were
  4-field but the running zygote wasn't. FIX = `tools/scripts/run-hybrid-stack.sh`
  (safe: pkills + restarts wandr-host/arbiter, does NOT wipe APPS_ROOT) so the
  zygote re-preloads. NOTE: `pack-ime-keyboard.sh` also `rm -rf`s the wrong path
  (`$APPS_ROOT/apps/...` but the IME installs to `$APPS_ROOT/system-apps/...`).
