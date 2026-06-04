---
name: reference-a03-ninja-build
description: "Fast a-03 AOSP rebuild: when only source (.cpp/.c/.rs) changed and the build graph is unchanged, skip m's soong+kati analysis and invoke ninja directly"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 023c2492-85e0-4052-bd04-4dc23f02fd88
---

**a-03 fast rebuild — use ninja directly when NO regeneration is needed.** `m
<module>` re-runs the whole analysis pipeline (soong → kati → ninja) every time,
which is slow even when only a source file changed. When the **build graph is
unchanged** — i.e. you edited only source (`.cpp`/`.c`/`.rs`), NOT any `Android.bp`
/ `.mk` / `.bp`-affecting file — the soong+kati analysis from a prior `m` is still
valid, so invoke ninja straight on the combined graph and it just recompiles +
relinks the one module:

```bash
cd ~/android/lineage
prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja wart-inputflinger
```

- Combined ninja file = `out/combined-<TARGET_PRODUCT>.ninja` (here
  `out/combined-aosp_arm64.ninja`, from `lunch aosp_arm64-trunk_staging-userdebug`).
- Ninja binary = `prebuilts/build-tools/linux-x86/bin/ninja` (don't rely on a
  system ninja). No `source build/envsetup.sh` / `lunch` needed for the direct call.
- The ninja target is the module name (a phony), e.g. `wart-inputflinger`,
  `libwart_sensors_hal`, `libsf_surface`.
- Output lands in the usual `out/.../<module>` path; `adb push` it like after `m`.

**Use `m` (full pipeline) when the graph DID change:** added/removed a module, edited
`Android.bp`/`Android.mk`, new source files, changed deps/flags. Then the combined
ninja must be regenerated first.

Distinct from the NEW-module gotcha (`m` dies in LineageOS kati `$(error)` dexpreopt):
there, run the soong shards then direct-ninja the soong intermediate — see
[[project-artless-sensors]]. This note is the everyday source-only fast path.
Infra context: [[project_boot_model_libgui_build]] (a-03 is available, not a blocker).
