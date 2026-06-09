---
name: project-boot-model-libgui-build
description: "How to build libgui-dependent C++ (task 33 boot-model shims) — built in-tree on the a-03 AOSP host. a-03 is AVAILABLE INFRASTRUCTURE, NOT a blocker."
metadata: 
  node_type: memory
  type: project
  originSessionId: c7e399b2-df64-40f8-a1cb-5eb2ed7e1afe
---

**a-03 IS NOT A BLOCKER — STOP TREATING IT AS ONE.** (Recurring
correction from the user, 2026-05-29.) AOSP is huge and a soong build
needs a powerful machine; a-03 (128 GB / 72 core) is exactly that
machine, it is **available and reachable** (`ssh harry@a-03 -i
~/.ssh/id_rsa.my`), and using it for libgui/soong C++ builds is the
**normal, expected workflow** — not an obstacle, not a deferral reason.
When a task needs a libgui shim rebuild (e.g. task 43's
`sf_set_rotation`, task 46 step 4 rebuilt `libsf_surface.so` on a-03
via a 43 s direct-ninja path), the plan is "ssh to a-03, edit, `m`,
copy the .so back, deploy" — proceed with it. Do NOT write "blocked /
deferred because it needs a-03." (Separately: many things framed as
"needs a-03" may not even need it — e.g. the `WANDR_ORIENT` host-side
Skia transform already does content rotation with no shim. Check the
Rust/Skia path first.)

Task 33 (boot-model) needs C++ that links Android's private platform
`libgui` (`SurfaceComposerClient` etc.) — `sf_probe`, the future
`sf_surface` shim, and the Step 3 InputFlinger `InputConsumer` shim.

**Out-of-tree compilation of `libgui` headers is infeasible** — its public
headers transitively need both AIDL *and* HIDL generated headers plus
several AOSP repos; the dependency fan-out never terminates cleanly.
`libgui`-dependent C++ must be built **in an AOSP source tree** as a soong
`cc_binary`, where soong resolves all codegen automatically.

Build host: `a-03` (`ssh harry@a-03 -i ~/.ssh/id_rsa.my`, 128 GB / 72 core)
holds a LineageOS 22.2 tree at `~/android/lineage`. adb to the Pixel 2 XL
from a-03 via `ADB_SERVER_SOCKET=tcp:harry-t490:5037` (phone is USB to
`harry-t490`). `repo` is at `~/.local/bin`; non-interactive ssh skips
`.profile`, so `export PATH=$HOME/.local/bin:$HOME/platform-tools:$PATH`.

Recipe: drop the source as a `cc_binary` in `external/<name>/` with an
`Android.bp` (`shared_libs: ["libgui","libui","libutils","libbinder",
"liblog","libEGL","libGLESv2","libnativewindow"]`), then
`lunch aosp_arm64-trunk_staging-userdebug && m <name>`. A **generic
`aosp_arm64` lunch on a LineageOS tree** needs three fixes (a real
`lineage_taimen` lunch would need proprietary vendor blobs instead):
1. neutralize the two `lineage_generator` kernel-header modules in
   `vendor/lineage/build/soong/Android.bp`;
2. `DISABLE_DEXPREOPT_CHECK=true`;
3. `BUILD_BROKEN_DUP_RULES := true` in
   `build/make/target/board/generic_arm64/BoardConfig.mk`.

**Why:** the project's Rust/cargo cross-compile (NDK) cannot touch
`libgui`; only an in-tree soong build can. This was discovered the hard way
across a long task-33 Step 1 session.

**How to apply:** for any new boot-model C++ shim, add it as a soong
`cc_binary`/`cc_library` in the a-03 tree and `m` it; copy the artifact
back and deploy with adb. Don't attempt to vendor AOSP headers into
`wandr-host/build.rs`. See `tasks/33-boot-model-bringup.md` Step 1.
Related: [[feedback-bionic-compat]].

**WORKING RECIPE (verified 2026-06-04, task 80 spike) — non-interactive ssh:**
```
ssh -i ~/.ssh/id_rsa.my harry@a-03 'bash -lc "
  cd ~/android/lineage
  source build/envsetup.sh >/dev/null 2>&1
  lunch aosp_arm64-trunk_staging-userdebug >/dev/null 2>&1
  export TARGET_RELEASE=trunk_staging TARGET_PRODUCT=aosp_arm64 TARGET_BUILD_VARIANT=userdebug
  export DISABLE_DEXPREOPT_CHECK=true
  m <module>
"'
```
GOTCHA: `lunch` sets `TARGET_RELEASE` but it does NOT survive into `m`'s child →
you MUST re-`export TARGET_RELEASE=trunk_staging` yourself, else
`release_config.mk:273: No release config set ... release is one of: .`. The full
`m` does a ~5-min kati full-tree parse EVERY time. **After the first `m` adds the
module to the soong ninja, iterate in SECONDS via direct-ninja (skips kati):**
`prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja <out-artifact-path>`.
cc_binary artifact: `out/soong/.intermediates/external/<mod>/<mod>/android_arm64_armv8-a/<mod>`
(no `_shared`); cc_library_shared: `.../android_arm64_armv8-a_shared/lib<mod>.so`.
