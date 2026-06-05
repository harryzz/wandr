---
name: project-artless-audio
description: "ART-off audio FULLY SOLVED (audible): audioserver needs 4 system_server binder stubs in wart-activityms — activity, sensor_privacy, scheduling_policy, AND permission (IPermissionController). The last unblocks MMAP playback START."
metadata: 
  node_type: memory
  type: project
  originSessionId: 023c2492-85e0-4052-bd04-4dc23f02fd88
---

**✅ TASK 87 FULLY SOLVED + USER-CONFIRMED AUDIBLE (2026-06-05).** ART-off audio output
works: a `--no-art` play-tone is heard on the Pixel 2 XL speaker. The complete recipe is
**4 system_server binder stubs in `wart-activityms` generics[]** + native volume init:
`activity` + `sensor_privacy` (audioserver init un-wedge), `scheduling_policy`
(REGISTER_AUDIO_THREAD requestPriority loop), **`permission` =
`android.os.IPermissionController` (LAYER 4, the final blocker)**, plus
`audio_policy_impl::init_audio_policy()` for volume levels.

**LAYER 4 root cause (found via ART-up vs --no-art play-tone A/B):** `MmapThread::start`
(audioflinger/Threads.cpp:10508, the START_CLIENT path) → `afutils::
checkAttributionSourcePackage` → `PermissionController::getPackagesForUid` →
`getService()` (frameworks-native/libs/binder/PermissionController.cpp:30) loops
`checkService("permission"); sleep(1)` for **10s then "giving up"** when system_server's
IPermissionController is dead. That 10s block runs ON the audioserver command thread
inside START_CLIENT → host `startStream` (AAudioServiceStreamBase TIMEOUT_NANOS=3s) times
out → `Command 6/7/10 time out`, MMAP PCM never RUNNING, `QUAT_MI2S_RX` Off → silence.
Smoking gun: audioserver logs `"Waiting for permission service"` ×N during a tone. FIX =
`{"permission","android.os.IPermissionController"}` in generics[] (GenericStub
writeNoException+writeInt32(0) = empty getPackagesForUid Vector<String16>, fine). Built
a-03 ninja-direct, redeployed. Now matches ART-up: pcm4p RUNNING, route On, no timeouts.

**DEAD ENDS RULED OUT (don't re-try):** (1) EXCLUSIVE vs SHARED sharing mode — host
stream-open path works under ART, must NOT change it; the shared mixer thread is not the
wedge (reverted). (2) `AudioSystem.systemReady()` (replicate
AudioService.onIndicateSystemReady via a media.audio_flinger IAudioFlingerService stub) —
lands correctly (`AudioFlinger: systemReady` logged at transaction code 53) but does NOT
fix the wedge; inert under --no-art (power service dead → no wakelock) (reverted). (3)
`"Could not set MMAP stream volume: no volume callback!"` — appears under ART-up TOO
(audible there) = irrelevant symptom; the MmapStreamCallback is the in-process
AAudioServiceEndpointMMAP (passes `this`), not a system_server thing. **Method = the
ART-up A/B is the high-signal tool; "works under ART" means diff what system_server
provides, don't change the host audio path.** a-03 IPermissionController descriptor +
the generic-stub reply shape are the reusable bits if a 5th audioserver stub surfaces.

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

**FOLLOW-UP BUGS from the audioserver restart (2026-06-05, device-observed, NOT yet
fixed):** the stub brings audio services back, but `run-hybrid-stack`'s `pkill
audioserver` (to un-wedge it after the stub registers) causes two issues:
(1) **slow app start** — an app that opens audio WHILE audioserver is (re)starting
blocks ~20s in `openStream` (the bringup restarts audioserver right before apps
launch → they race it; steady-state openStream is instant);
(2) **no sound (intermittent)** — `startStream` sometimes returns **-895
(AAUDIO_ERROR_INVALID_STATE)** → started=false → no beep, with HAL
`out_get_mmap_position: pcm_sync_ptr failed: File descriptor in bad state` (flaky
MMAP-exclusive on taimen, aggravated by the restart corrupting the HAL MMAP FD).
FIX DIRECTION (not done): avoid the hard `pkill audioserver` — let the stub un-wedge
audioserver on its own (register activity/sensor_privacy BEFORE it (re)inits); if a
restart is unavoidable, do it EARLY + wait for media.audio_policy registered BEFORE
launching any app (kills the 20s race); optionally retry startStream once on -895
host-side. NOTE earlier observation that "the stub alone didn't register media.audio_*
without a restart" needs re-confirming — if true, the restart is needed and the timing
+ MMAP-FD issues must be handled instead. The ~3.5s Compose startup (smoke tests +
208 fonts/typefaces) is inherent to the demo app, not this regression.

**✅ FIX (2026-06-05): host re-resolves `media.aaudio` on a dead handle + `play-tone`
gains a volume arg & optional target.** Two changes, device-verified:
(1) `audio_impl.rs::service()` was a write-once `OnceLock` → a stale handle after an
`audioserver` restart made `openStream` fail with `DeadObject`/`TransactionFailed`.
Now it's a `Mutex<Option<Strong>>`: each call `ping_binder()`s the cached handle and,
if dead, drops it and re-resolves (+ re-`registerClient`); returns an owned `Strong`
clone. `media.audio_policy` (audio_policy_impl.rs) already resolved fresh per-call, so
it needed nothing. VERIFIED: warm handle → instant reuse (no "media.aaudio ready"
re-log, sub-second openStream); after `pkill audioserver` → re-resolves + plays (3×,
no DeadObject). The ~20s first-open after a restart is the inherent taimen MMAP
cold-start, unrelated. (Curiosity: the "handle dead — re-resolving" warn didn't appear
in logcat even though the re-resolve provably ran — re-resolve confirmed by OUTCOME,
not that log line.)
(2) `play-tone` is now `play-tone [pid|app-id] [ms] [hz] [vol0-1]` — target OPTIONAL,
defaults to the foreground host (`store.display(PRIMARY).visible_app()`); a numeric
first token is treated as a target ONLY if it's a KNOWN pid (else it's the start of the
`ms hz vol` triple — `resolve()` maps any int to `Some(pid,"?")`, so don't use it for
detection). `vol` is the tone's digital amplitude clamp[0,1], default 0.5 (NOT device
master volume — absolute volume under --no-art is still unsolved). Arbiter
`cmd_play_tone` → `deliver_to_host(pid, "play-tone {ms} {hz} {vol}")` → host applier.
Why a target at all: the arbiter never opens audio — a HOST process is the applier, so
play-tone must name which host plays; pid = the applier's socket address.

**✅ NATIVE VOLUME + DEVICE INIT (2026-06-05, implemented + verified) — but NOT
sufficient alone; MMAP routing still blocks audible output.** Why no sound under
--no-art was NOT volume in the simple sense: `AudioService.java` (Java, system_server,
DEAD under --no-art) normally runs `onReinitVolumes()` at boot → `AudioSystem.
initStreamVolume(stream, MIN, MAX)` + per-device `setStreamVolumeIndex` + NORMAL phone
state. Without it the policy returns volume index range **-1** and streams sit at
**-inf dB**. We now replicate this NATIVELY in Rust (NOT Java, NOT C++): new
`audio_policy_impl::init_audio_policy()` calls `initStreamVolume`+`setStreamVolumeIndex`
for the 12 public streams (MIN/MAX copied verbatim from `AudioService.MIN_/MAX_
STREAM_VOLUME`; MUSIC full-scale, rest ~80%) on OUT_DEFAULT/SPEAKER/EARPIECE/HEADPHONE/
HEADSET, then `setPhoneState(NORMAL)`+`setForceUse(COMMUNICATION,NONE)`. CLI flag
`wart-host --init-audio-policy` (run by run-hybrid-stack right after `media.audio_policy`
registers; needs `LD_LIBRARY_PATH=/data/local/tmp` or libc++_shared.so won't link).
VERIFIED: speaker (AudioDeviceType 140) range went **[-1..-1] → [0..15] idx 15**, and
the arbiter `volume up/down` keys now work (were no-ops). AudioStreamType MUSIC=3,
declare_binder_enum newtype so `AudioStreamType(n)` constructs. **REMAINING BLOCKER
(still silent):** the tone uses an AAudio **MMAP-exclusive** endpoint whose DSP route
to the taimen loudspeaker (`QUAT_MI2S_RX Audio Mixer MultiMediaN` per
`mixer_paths_tavil_taimen.xml` — the speaker is an external MI2S smart-amp, NOT the WCD
codec RX) **never turns On**, and no playback PCM substream goes RUNNING — the HAL
hits `out_get_mmap_position: pcm_sync_ptr failed: File descriptor in bad state`. So the
NEXT step is to steer OFF the MMAP-exclusive path (force a legacy/shared AudioFlinger-
mixed AudioTrack, e.g. performanceMode=NONE) so AudioPolicyManager+HAL actually apply
the speaker route — volume is now correct, routing is the last gap. Tools: cross-built
`tinymix` (from github tinyalsa, NDK clang, dynamic+`-ldl`) at /data/local/tmp/tinymix
for live ALSA mixer inspection; `setStreamVolume` on media.audio_flinger returns -38
(INVALID_OPERATION) = Android-15 port-based volume mgmt, so stream-volume must go via
media.audio_policy index API (which init_audio_policy now sets up).

**🎯 ROOT CAUSE FOUND via ART-up vs --no-art A/B (2026-06-05) — it's a MISSING
system_server binder `scheduling_policy`, NOT volume/routing/MMAP-rewrite.** Ran the
SAME host MMAP tone (`wart-host --play-tone`, new CLI) under ART-up vs --no-art and
diffed audioserver state. IDENTICAL on both: audio_policy device config, the 6 audio
patches (outputs→AUDIO_DEVICE_OUT_SPEAKER), phone state NORMAL, force-use NONE,
`Output devices: 0x2 SPEAKER`. The ONLY difference is the **stream START**: ART-up
logs `startClient port 29 + 30`, PCM runs (`pcmC0D4p RUNNING`), `QUAT_MI2S_RX Audio
Mixer MultiMedia3 = On` → audible. --no-art logs `startClient port 29` then
**`AAudioCommandQueue: Command 6 time out`** + HAL `pcm_sync_ptr failed: FD in bad
state` → no PCM, route stays Off, silent. **Command 6 = REGISTER_AUDIO_THREAD**
(enum START=0..DISCONNECT=5, REGISTER_AUDIO_THREAD=6; confirmed by ART-up "opcode 8 =
GET_DESCRIPTION"). Source chain (all vendored): `AAudioServiceStreamBase::
registerAudioThread_l` (AAudioServiceStreamBase.cpp:594) → `android::requestPriority(
ownerPid, tid, prio, isForApp=TRUE)` → `media/utils/SchedulingPolicyService.cpp:34`.
With isForApp=true it SKIPS the audioserver fast-path and enters `for(;;){ binder =
defaultServiceManager()->checkService("scheduling_policy"); if(binder==0){sleep(1);
continue;} ... }` — an **infinite 1s-retry loop** because `scheduling_policy`
(`ISchedulingPolicyService`, descriptor `android.os.ISchedulingPolicyService`, hosted
in system_server) is DEAD under --no-art. So the data-thread RT-priority registration
hangs forever → MMAP stream never starts the PCM → HAL route never applied → silence.
Under ART-up the service exists → returns instantly → stream starts. SAME host code,
only the environment differs (= why "works under ART"). **FIX (one line, established
pattern):** add `{"scheduling_policy","android.os.ISchedulingPolicyService"}` to
wart-activityms's `generics[]` (wart_activityms.cpp:171) — its `GenericStub` already
replies `writeNoException()+writeInt32(0)`, which is EXACTLY what
`BpSchedulingPolicyService::requestPriority` reads (`readExceptionCode()==0` then
`readInt32()` → 0 = NO_ERROR; REQUEST_PRIORITY_TRANSACTION=FIRST_CALL=1, non-oneway).
Rebuild wart-activityms on a-03, redeploy. This is the 3rd system_server binder a
native audio survivor consumes (after `activity`, `sensor_privacy`). NOTE: the volume
init ([[project-artless-audio]] above) is STILL needed for level; scheduling_policy is
needed for the stream to START at all. So NEITHER earlier option (rewrite to AudioTrack
/ big device init) was right — MMAP is fine, it just needs this one stub.

**⏳ scheduling_policy stub BUILT+DEPLOYED+WORKS for its purpose, but a 2ND blocker
remains (2026-06-05).** Added `{"scheduling_policy","android.os.ISchedulingPolicyService"}`
to wart-activityms generics[], rebuilt on a-03 (ninja-direct the soong intermediate —
source-only change, no .bp edit: `prebuilts/build-tools/linux-x86/bin/ninja -f
out/combined-aosp_arm64.ninja out/soong/.intermediates/external/wart-activityms/
wart-activityms/android_arm64_armv8-a/wart-activityms`), scp'd back to
runtime/wart-activityms/cpp/wart-activityms, deployed via run-hybrid (push_if_newer).
CONFIRMED WORKING: `scheduling_policy: []` registers, and the **Command 6
(REGISTER_AUDIO_THREAD) infinite-loop is GONE** — the AAudio stream now advances
START→REGISTER→START_CLIENT→`setState ...→4 (STARTED)` (it never reached STARTED
before). BUT still NO audible output: a 2nd blocker — **`AAudioCommandQueue: Command 7
(UNREGISTER_AUDIO_THREAD) time out`** on the SHARED-endpoint stream's command thread +
`AudioFlinger: Could not set MMAP stream volume: no volume callback!` + the MMAP PCM
never goes RUNNING + `QUAT_MI2S_RX` stays Off (clean-audioserver retest = same, so not
accumulated stuck streams). unregisterAudioThread_l itself is trivial (no binder), so
the Command-7 timeout = the shared-endpoint command thread is wedged after START_CLIENT.
Hypothesis for next session: it's the AAudioServiceEndpointShared mixer thread
(startSharingThread_l) since our host requests SHARING_MODE_SHARED → service opens a
shared endpoint that runs a mixer thread over an EXCLUSIVE MMAP backing; try requesting
**EXCLUSIVE** directly (we bypass the libaaudio client builder, so the
client-side EXCLUSIVE→SHARED downgrade doesn't apply) so the host writes the MMAP
buffer directly with no service-side mixer thread — may sidestep the Command-7 dance.
OR investigate the "no volume callback" (MmapThread volume) as the real stall. Host got
a `--play-tone [ms] [hz] [vol]` CLI for A/B testing (same media.aaudio path).

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
