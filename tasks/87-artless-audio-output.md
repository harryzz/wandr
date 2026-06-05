# Task 87 — ART-off audio output (make sound actually play under `--no-art`)

> Status: ✅ SOLVED + USER-CONFIRMED AUDIBLE (2026-06-05). Four layers, each a
> thing `system_server` does that's missing under `--no-art`; the last (Layer 4)
> was the missing `permission` binder. See "Layer 4 — RESOLVED" below.
> Follow-on to task 85 (ART-off sensors) / 86 (ART-off auto-brightness) / the
> ART-off audio-stub work (`wart-activityms`). Detail + history:
> `[[project-artless-audio]]`. Written as a fresh-head handoff — everything needed
> to resume is here; you should NOT have to re-derive the chain below.

## Goal

Under `run-hybrid-stack.sh --no-art` (Java framework / `system_server` stopped), a
guest/host that opens an AAudio output stream must produce **audible** sound on the
Pixel 2 XL loudspeaker. Today: streams open and "start" but no audio comes out.

It WORKS under ART-up with the *identical* host code — so this is purely about
replacing the orchestration `system_server` normally provides, native-side. See
"Why it works under ART" below.

---

## TL;DR of the diagnosis (all source-grounded, device-verified)

The `--no-art` audio path is a multi-layer onion. Each layer is a thing
`system_server` (Java) normally does that's missing when it's stopped, while the
native engine (`audioserver`: AudioFlinger / AudioPolicyManager / AAudioService +
the vendor HAL) survives and is fine.

| Layer | Symptom when missing | Fix | State |
|---|---|---|---|
| 1. `activity` / `sensor_privacy` binders | audioserver wedges in init, `media.audio_*` never register | `wart-activityms` stub (C++/libbinder, a-03) | ✅ shipped (pre-87) |
| 2. stream volume init | policy volume range `-1`, every stream `-inf dB` | `audio_policy_impl::init_audio_policy()` (Rust) | ✅ this task |
| 3. `scheduling_policy` binder | `Command 6 REGISTER_AUDIO_THREAD` infinite-loops → stream never `STARTED` | add to `wart-activityms` generics[] | ✅ this task |
| 4. **stream actually starts the PCM** | reaches `STARTED` but `Command 7` times out, MMAP PCM not RUNNING, `QUAT_MI2S_RX` Off → **SILENT** | `permission` (`IPermissionController`) binder stub in `wart-activityms` | ✅ this task |

---

## Layer 4 — RESOLVED (the `permission` binder)

**Root cause (ART-up vs `--no-art` A/B + source-confirmed):** `MmapThread::start`
(`audioflinger/Threads.cpp:10508`, the START_CLIENT path) calls
`afutils::checkAttributionSourcePackage` → `PermissionController::getPackagesForUid`
→ `PermissionController::getService()`
(`frameworks-native/libs/binder/PermissionController.cpp:30`), which loops
`checkService("permission"); sleep(1);` for **10 s** then "giving up" when
`system_server`'s `IPermissionController` is dead. That 10 s block runs **on the
audioserver command thread inside START_CLIENT**, so the host's `startStream`
(`TIMEOUT_NANOS = 3 s`) times out → `Command 6/7/10 time out` → no PCM RUNNING →
`QUAT_MI2S_RX` Off → silence. Device-confirmed: audioserver logs
`"Waiting for permission service"` / `"Waiting too long … giving up"` ×N during a tone.

**Fix:** add a 4th generic stub binder — `{"permission",
"android.os.IPermissionController"}` — to `wart-activityms` `generics[]`
(`runtime/wart-activityms/cpp/wart_activityms.cpp`), alongside `activity` /
`sensor_privacy` / `scheduling_policy`. Registering any binder makes
`checkService("permission")` return instantly (no block); `GenericStub`'s
`writeNoException()+writeInt32(0)` decodes as `getPackagesForUid`'s empty
`Vector<String16>`, which `checkAttributionSourcePackage` handles fine. Built on
a-03 (ninja-direct the soong intermediate — source-only change), redeployed.
**Result:** `--no-art` play-tone is **audible** (user-confirmed), `pcm4p` RUNNING,
`QUAT_MI2S_RX … MultiMedia3` On, no Command timeouts — identical to the ART-up trace.

**Dead ends ruled out (both via the ART-up A/B):**
- *EXCLUSIVE vs SHARED sharing mode* — changing the host stream-open path is wrong
  (it works under ART); reverted. The shared mixer thread is not the wedge.
- *`AudioSystem.systemReady()`* (replicate `AudioService.onIndicateSystemReady()`
  via a `media.audio_flinger` `IAudioFlingerService` stub) — lands correctly
  (`AudioFlinger: systemReady` logged) but does **not** fix the wedge; inert under
  `--no-art` (the power service it would gate the wakelock on is also dead). Reverted.
- *"Could not set MMAP stream volume: no volume callback!"* — appears under **ART-up
  too** (where audio is audible), so it's an irrelevant symptom, not the cause. The
  `MmapStreamCallback` is the in-process `AAudioServiceEndpointMMAP` (passes `this`),
  not anything `system_server` registers — nothing for wart to supply.

---

## What is DONE + device-verified (do not redo)

### Layer 2 — native volume init (Rust)
`runtime/wart-host/src/audio_policy_impl.rs::init_audio_policy()` replicates the
slice of `AudioService.onReinitVolumes()` we need: for the 12 public streams,
`initStreamVolume(stream, MIN, MAX)` (values copied verbatim from
`AudioService.MIN_/MAX_STREAM_VOLUME`) + `setStreamVolumeIndex` per device
(`OUT_DEFAULT/SPEAKER/EARPIECE/HEADPHONE/HEADSET`; MUSIC full-scale, rest ~80%) +
`setPhoneState(NORMAL)` + `setForceUse(COMMUNICATION, NONE)`.
- CLI: `wart-host --init-audio-policy` (`main.rs`). Needs
  `LD_LIBRARY_PATH=/data/local/tmp` or libc++_shared.so won't link.
- Wired into `run-hybrid-stack.sh` right after `media.audio_policy` registers.
- VERIFIED: speaker volume range went `[-1..-1]` → `[0..15]` idx 15, and the
  arbiter `volume up/down` keys now work (were no-ops).

### Layer 3 — `scheduling_policy` stub (C++, a-03)
Added `{"scheduling_policy","android.os.ISchedulingPolicyService"}` to
`runtime/wart-activityms/cpp/wart_activityms.cpp` `generics[]`. The existing
`GenericStub` replies `writeNoException()+writeInt32(0)` — exactly what
`BpSchedulingPolicyService::requestPriority` reads (`readExceptionCode()==0`,
`readInt32()`→0 = `NO_ERROR`; `REQUEST_PRIORITY_TRANSACTION=FIRST_CALL=1`, non-oneway).
- Built on a-03 ninja-direct (source-only change, no `.bp` edit):
  `prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja
  out/soong/.intermediates/external/wart-activityms/wart-activityms/android_arm64_armv8-a/wart-activityms`
  then `scp` back to `runtime/wart-activityms/cpp/wart-activityms`.
- VERIFIED: `scheduling_policy: []` registers; **the `Command 6` infinite-loop is
  gone** — the AAudio stream now advances `START→REGISTER→START_CLIENT→setState→4
  (STARTED)`, which it NEVER reached before.

### Diagnostic tooling added this task
- `wart-host --play-tone [ms] [hz] [vol]` — standalone tone via the *exact* host
  `media.aaudio` MMAP path; used for the ART-up vs `--no-art` A/B (no arbiter needed,
  so it runs under ART-up too).
- `wart-arbiter play-tone [pid|app] [ms] [hz] [vol]` — arbiter→host tone (target
  optional, defaults to foreground host).
- Cross-built `tinymix` (from github.com/tinyalsa, NDK clang, dynamic + `-ldl`) at
  `/data/local/tmp/tinymix` for live ALSA-mixer inspection.

---

## The remaining blocker (Layer 4) — START reaches STARTED but PCM never runs

After layers 2+3, on every tone (incl. on a freshly-restarted clean audioserver, so
NOT accumulated stuck streams):

```
AAudioServiceEndpointMMAP: startClient(): returning port NN, result 0
AAudioServiceStreamBase: run() got COMMAND opcode 6 (REGISTER_AUDIO_THREAD)   # now OK
AAudioServiceStreamBase: run() got COMMAND opcode 10 (START_CLIENT)
AAudioStream: setState(...) from 3 to 4                                        # STARTED
AudioFlinger: Could not set MMAP stream volume: no volume callback!
AAudioCommandQueue: Command 7 (UNREGISTER_AUDIO_THREAD) time out               # <-- wedge
audio_hw_primary: out_get_mmap_position: pcm_sync_ptr failed: FD in bad state  # (maybe benign hack call)
```

Result: `QUAT_MI2S_RX Audio Mixer MultiMediaN` stays **Off**, no playback PCM goes
**RUNNING**, the host's `start()` (`startStream`) never returns ("play-tone:" host log
never prints) → silence.

Key facts about the wedge:
- `unregisterAudioThread_l` is trivial (no binder call) — so a `Command 7` *timeout*
  means the command thread is **wedged after `START_CLIENT`**, not in unregister itself.
- The wedged thread is the **`AAudioServiceEndpointShared`** stream's command thread
  (there are two streams: the client-facing SHARED stream + its EXCLUSIVE MMAP backing).
- We request `SHARING_MODE_SHARED`, so the service builds a shared endpoint that runs
  a **mixer thread** (`AAudioServiceEndpointShared::startSharingThread_l`) over an
  EXCLUSIVE MMAP backing. That mixer thread is the likely owner of the stuck
  register/unregister + "no volume callback".

---

## Leading hypotheses for Layer 4 (start here, cheapest first)

1. **Request EXCLUSIVE instead of SHARED** (cheapest, host-only, no a-03).
   `audio_impl.rs::open_pcm_stream`/`create_track` currently sets
   `AAUDIO_SHARING_MODE_SHARED`. Because we talk to `IAAudioService` directly we
   BYPASS the libaaudio client builder's EXCLUSIVE→SHARED downgrade, so we can ask the
   service for EXCLUSIVE → `openExclusiveEndpoint` → `AAudioServiceEndpointMMAP`
   directly, **no service-side shared mixer thread**. Our host already writes the MMAP
   shm buffer itself, so a direct exclusive endpoint should suit it and may sidestep
   the Command-7 dance entirely. Try this FIRST. Risk: EXCLUSIVE may collide if another
   client holds the MMAP (kStealing) — fine for the test.

2. **Chase "Could not set MMAP stream volume: no volume callback!"**
   AudioFlinger `MmapThread` can't set the stream volume because no volume callback is
   registered. Under ART-up something registers it (likely via AudioService /
   `AudioSystem`). If this is the actual stall (not Command 7), find who registers the
   MMAP volume callback under ART and replicate it natively. Source:
   `frameworks/av/services/audioflinger/MmapTracks`/`MmapThread` +
   `AAudioServiceEndpointMMAP` volume wiring.

3. **A second missing system_server binder / a hung internal priority call.**
   Re-run the A/B (below) and diff the FULL AAudio command sequence + any
   `checkService`/`waitForService` between ART-up and `--no-art` for the SHARED
   stream's thread. The shared mixer thread boosts its own priority — verify it isn't
   hitting another `requestPriority`/service path that hangs.

---

## How to reproduce / the A/B (this is how Layer 3 was found)

```bash
# bring up (pushes host+arbiter+activityms, runs --init-audio-policy)
bash tools/scripts/run-hybrid-stack.sh --restore-art      # then wait for boot
bash tools/scripts/run-hybrid-stack.sh --no-art

# play the host's exact MMAP path, 30s so it survives the ~20s MMAP cold-start
adb shell 'su -c "/data/local/tmp/wart-arbiter unlock"'
adb shell 'su -c "/data/local/tmp/wart-arbiter foreground war.launcher"'
adb shell 'su -c "/data/local/tmp/wart-arbiter play-tone war.launcher 30000 440 0.6"' &
sleep 24      # MUST wait past cold-start, then check DURING the playing window

# did the route + PCM come up?  (the pass/fail signal)
adb shell 'su -c "/data/local/tmp/tinymix contents"' | grep -i "QUAT_MI2S_RX Audio Mixer Multi" | grep -i "On$"
adb shell 'su -c "for s in /proc/asound/card0/pcm*p/sub0/status; do grep -q RUNNING \$s && echo RUN:\$s; done"'

# the AAudio start sequence + wedge
adb logcat -d | grep -iE "got COMMAND opcode|Command .* time out|startClient|no volume callback|play-tone:" | grep -iv hwservice
```

A/B vs ART-up: same `--play-tone` standalone under ART-up shows
`QUAT_MI2S_RX MultiMedia3 On` + `pcmC0D4p RUNNING` + `startClient 29 AND 30`, no
Command timeouts → audible. That's the working reference to diff against.

**PASS criteria for this task:** `QUAT_MI2S_RX ... On` + a playback PCM `RUNNING`
during a tone, and the user CONFIRMS they hear it (audible output needs the user —
`[[feedback_visual_verification]]`). Bonus: the live-app startup chime is audible.

---

## Why it works under ART (sanity anchor)

Same host Rust code, same `media.aaudio` MMAP path in both modes. Under ART-up,
`AudioService` (Java, `system_server`) supplies the orchestration: stream volume init,
`scheduling_policy` for thread-priority boosts, the MMAP volume callback, etc. Under
`--no-art` those are gone; we re-supply them natively (the wart pattern). So "fix" =
keep finding the specific native calls/services `AudioService` provides for the MMAP
*start* path and replicate them — NOT rewrite to the AudioTrack path and NOT a bigger
device/routing init (the audio_policy device config + the 6 output→SPEAKER patches are
already IDENTICAL to ART-up under `--no-art`; verified via `dumpsys media.audio_policy`).

---

## Key source pointers (vendored, so a fresh head doesn't re-derive)

- `frameworks/av/services/oboeservice/AAudioServiceStreamBase.cpp`
  — command enum (`START=0 … REGISTER_AUDIO_THREAD=6, UNREGISTER_AUDIO_THREAD=7,
  GET_DESCRIPTION=8, START_CLIENT=10`), the command loop (~line 470-560),
  `registerAudioThread_l` (594 → `android::requestPriority(..., isForApp=true)`),
  `unregisterAudioThread_l` (trivial).
- `frameworks/av/services/oboeservice/AAudioEndpointManager.cpp` — `openEndpoint`
  (147: EXCLUSIVE→`openExclusiveEndpoint`, else `openSharedEndpoint`).
- `frameworks/av/services/oboeservice/AAudioServiceEndpointShared.cpp` — `open()`
  forces `setSharingMode(EXCLUSIVE)` on its internal stream; `startSharingThread_l`
  (the mixer thread, prime Layer-4 suspect).
- `frameworks/av/services/oboeservice/AAudioServiceEndpointMMAP.cpp` — `open()` /
  `openWithConfig()` → `MmapStreamInterface::openMmapStream`.
- `frameworks/av/media/utils/SchedulingPolicyService.cpp::requestPriority` — the
  `for(;;){ checkService("scheduling_policy"); if(0){sleep(1);continue;} }` loop
  (Layer 3 root); `ISchedulingPolicyService.{h,cpp}` for the proxy/reply shape.
- `frameworks/base/.../server/audio/AudioService.java` — the SPEC for what to
  replicate (`onReinitVolumes`, `MIN_/MAX_STREAM_VOLUME`, `onAudioServerDied`).
- Loudspeaker route = `/vendor/etc/mixer_paths_tavil_taimen.xml` "low-latency-playback
  speaker" → `QUAT_MI2S_RX Audio Mixer MultiMediaN=1` (taimen speaker is an external
  MI2S smart-amp, NOT the WCD codec RX volumes).

## Gotchas / constraints

- `setStreamVolume` on `media.audio_flinger` returns `-38 INVALID_OPERATION`
  (Android-15 port-based volume mgmt) — stream volume MUST go via the
  `media.audio_policy` index API (what `init_audio_policy` uses).
- First `openStream` after any audioserver restart has a ~20s MMAP cold-start on
  taimen — always wait past it before judging.
- Re-running `--no-art` while already in `--no-art` exits 20 (`cmd package` needs the
  framework) — always `--restore-art` first.
- Don't thrash the device — read the source path before each new probe
  (`[[feedback_read_source_first]]`). The A/B (ART-up vs `--no-art` diff) is the
  high-signal tool; use it.
- a-03 rebuild of `wart-activityms` only needed if you change the C++ stub; the
  Layer-4 hypotheses (1)/(2) are host-Rust-only (no a-03).
