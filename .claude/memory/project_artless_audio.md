---
name: project-artless-audio
description: "ART-off audio: audioserver is native+standalone but WEDGES on waitForService('activity') (ActivityManager) → no audio under --no-art; fix = tiny IActivityManager stub, NOT an audioflinger refactor"
metadata: 
  node_type: memory
  type: project
  originSessionId: 023c2492-85e0-4052-bd04-4dc23f02fd88
---

**✅ SOLVED + device-verified (2026-06-05): C++ stub `wart-activityms`.** Registers a
precise `activity` (IActivityManager) + a generic `sensor_privacy` binder; audioserver
un-wedges through both `waitForService`s and re-registers media.audio_flinger/policy/
aaudio; an output stream opens (loopback probe: `media.audio_policy ready` +
`media.aaudio ready` + AAudioService `openWithConfig … MMAP stream`). Built on a-03
(`runtime/wart-activityms/cpp/`), launched by run-hybrid-stack `--no-art` (via
wart-launch → uid system) which then restarts audioserver so it re-inits with the
stubs present. KEY: C++/libbinder `addService` WORKS where rsbinder's FAILED (below); a
plain BBinder defaults to Local stability (no VINTF requirement). The stub hardcodes
the descriptor `"android.app.IActivityManager"` (a-03 libbinder doesn't export
`IActivityManager::descriptor` / compile IActivityManager.cpp) + uses the header's
transaction-code enum (header-only, no link). audioserver needs a CHAIN of
system_server stubs (activity → sensor_privacy → …); add more names to the
`generics[]` array in wart_activityms.cpp as logcat `Waited one second for <svc>`
reveals them. a-03 build gotchas: new module → `m` dies in kati but soong regenerates
→ ninja the soong intermediate
`out/soong/.intermediates/external/wart-activityms/.../wart-activityms`
([[reference-a03-ninja-build]]); class BBinder is in `<binder/Binder.h>` (no
BBinder.h); add `include_dirs:["frameworks/native/libs/binder/include_activitymanager"]`
for IActivityManager.h. The dormant Rust crate
(runtime/wart-activityms/{src,build.rs,aidl}) is kept for if rsbinder ever gains
real-servicemanager register support.

(Diagnosis history below — incl. the rsbinder dead-end.)

**STATUS: diagnosed, NOT yet fixed (2026-06-05). Corrects an earlier wrong assumption
that audio "survives --no-art cleanly."**

**Symptom (user-reported):** the 1–2 s sound at wart-host startup played before
`--no-art` but not after. Audio is dead under `--no-art`.

**Root cause (device + AOSP-source confirmed):** `audioserver` (AudioFlinger +
AudioPolicyService + AAudioService, frameworks/av) IS a native, standalone process
(`class core`, survives ART-off like surfaceflinger) — UNLIKE InputDispatcher (which
was inside system_server and needed the path-A `wart-inputflinger`). BUT audioserver
has a runtime dependency on **ActivityManager**: `AudioPolicyService`'s `UidPolicy`
registers a UID observer via the native `ActivityManager` client
(`frameworks/native/libs/binder/ActivityManager.cpp:40`) →
**`waitForService("activity")`**, which BLOCKS FOREVER under `--no-art` (ActivityManager
is in the dead system_server). The restarted audioserver wedges in init → the
`media.audio_flinger` / `media.audio_policy` / `media.aaudio` binders never
(re)register → no audio. Device logcat: audioserver (uid 1041) + cameraserver loop
`W ServiceManagerCppClient: Waited one second for activity` every second;
servicemanager tries to lazy-start `activity` via `ctl.interface_start aidl/activity`
→ init "Could not find 'aidl/activity'". Same CLASS as sensorservice's
`waitForService("package_native")` hang ([[project_artless_sensors]]). Why the
startup sound played once: the PRE-stop audioserver (registered services) lingered
briefly into `--no-art`; then it restarted and the new instance wedged.

**FIX = a tiny stub `activity` service, NOT an audioflinger refactor, NOT extraction
from system_server.** The `'activity'` client is a hand-rolled C++ `IActivityManager`
(`DECLARE_META_INTERFACE(ActivityManager)`; header
`libs/binder/include_activitymanager/binder/IActivityManager.h` is in the vendored
AOSP tree → buildable on a-03 like wart-inputflinger, but far simpler). A stub
subclasses `BnActivityManager`, registers the binder name `"activity"`, and implements
~9 trivial methods: `registerUidObserver`/`registerUidObserverForUids`/
`unregisterUidObserver` (no-op OK), `isUidActive`→true, `getUidProcessState`→a
foreground/"top" state, `checkPermission`→granted, `openContentUri`/`logFgsApiBegin`/
`logFgsApiEnd` no-op. That one stub unblocks BOTH audioserver AND cameraserver (both
wait on `activity`). Launch via `wart-launch` (uid-system + sepolicy context, same as
how wart-inputflinger registers its binder name; bare root can't register under
--no-art). Later it can be backed by the arbiter's real foreground/UID knowledge
(wart IS the AMS — [[project_task74_surface_role_model]]) for proper audio focus /
ducking / mic-privacy; a permissive no-op stub is enough to get audio WORKING first.

**BUILD ATTEMPT 1 — pure-Rust rsbinder stub (2026-06-05): built + correct but
BLOCKED on registration.** Created `runtime/wart-activityms` (standalone crate):
minimal `aidl/android/app/IActivityManager.aidl` (the ~12 methods the native client
calls, declared IN the native transaction-code-enum order so AIDL codes match;
`IBinder` for the IUidObserver params), rsbinder-aidl codegen (needs
`.set_async_support(true)` + `async-trait` + `tokio` deps or the generated async
service trait won't compile), permissive stub impl (isUidActive=true,
getUidProcessState=PROCESS_STATE_TOP=2, checkPermission=GRANTED=0, observers no-op).
Compiles clean (aarch64). BUT `host.add_service("activity", …)` →
**`FailedTransaction`** from the REAL Android-15 servicemanager. Diagnosis: NOT
selinux (Permissive), NOT the name (a custom test name fails too), NOT pingBinder
(SM's addService doesn't ping). Binder-level `BR_REPLY` came back + servicemanager
logs NOTHING for the add → by elimination it's the SILENT `meetsDeclarationRequirements`
path (ServiceManager.cpp:528 → `Stability::requiresVintfDeclaration(binder)`): the
binder's **stability** is read as needing a VINTF declaration. rsbinder's generated
`BnActivityManager::new_binder` bakes in `Stability::default()` = **System**
(binder.rs `#[default] System`); C++ libbinder uses **Local** (=0) for a plain
addService — which is why C++ `wart-inputflinger` addService("wart.windowreg") WORKS
from the identical wart-launch/uid-system/Permissive context but rsbinder's doesn't.
rsbinder's stability WIRE-encoding (`From<Stability> for i32`) also only
special-cases Android 12 (sdk 31/32); its add/register path is validated against
rsbinder's own `rsb_hub`, NOT the real servicemanager. NEXT: either force the binder
to `Stability::Local` (needs an rsbinder patch/fork — the generated new_binder
hardcodes default System, no public Local knob) OR fall back to a small C++
registrar (BBinder + onTransact, ~12 codes) built on a-03 like wart-inputflinger
(proven addService under --no-art). Binary/crate kept at runtime/wart-activityms.

**UPDATE (2026-06-05): forcing Stability::Local did NOT fix it — deeper rsbinder
wire-incompat.** Patched build.rs to rewrite the generated
`Stability::default()`→`Stability::Local` (post-codegen string replace, like
wart-hal-display's float fix) — verified applied — but addService STILL fails. Calling
`rsbinder::hub::add_service` directly (returns full `Status`) reveals the real error:
**`BadParcelable: "Parcel data not fully consumed, unread size: 36"`** (exception=
BadParcelable, txn_err=Ok). So servicemanager replied with a real exception whose
36-byte message/stack rsbinder's addService reply-parser does NOT consume — rsbinder
BOTH mishandles the servicemanager reply wire-format AND hides the true rejection
reason behind a generic BadParcelable. rsbinder has no android_15 servicemanager
variant (maps sdk15→android_14) and its add/register path is validated only against
its own `rsb_hub`, never the real Android servicemanager. CONCLUSION: pure-Rust
registration via rsbinder needs real rsbinder fixes (proper servicemanager-reply /
exception parsing, likely an android_15 SM variant) — non-trivial and uncertain. The
RELIABLE path is the C++ registrar (BBinder + onTransact, the ~12 IActivityManager
codes from libs/binder/IActivityManager.h, addService via libbinder) built on a-03
like wart-inputflinger — C++ libbinder addService is PROVEN under --no-art
(wart.windowreg). Keep the rsbinder crate (runtime/wart-activityms) for if rsbinder
gains real-SM register support; build the C++ one for now.

Audio POLICY (volume/route/focus/mode) is already reimplemented as
[[project_arbiter_audio]] talking to `media.audio_policy` directly; only this
`activity`-dependency gates the mechanism under --no-art. See
[[project_art_shutdown]] (the KEEP/REIMPLEMENT/PATH-A service strategy; this is a 4th
pattern: STUB a system_server binder a native survivor depends on),
[[project_pathA_inputflinger]] (the C++ binder-service build pattern to mirror).
