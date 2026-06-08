# Task 97 — `--no-art` call-audio stalls + audioserver instability (investigation)

> Status: 🔬 OPEN — handoff for a fresh session (spun out of the task-91 call-audio
> work, 2026-06-08). Several call-audio bugs were fixed this session; the RESIDUAL
> ones below recur and need a focused, source-grounded investigation.
> Primary memory: `[[project_call_audioserver_crash]]` (full diagnosis + the fixes).

## TL;DR

Signal calls under `--no-art` intermittently go **silent (I can't hear the peer; they
hear me)**. The engine is ALWAYS fine (`peak=1.000`, decrypts+decodes). The silence is
host-side, and across this session showed **three distinct host failure modes** — two
are fixed, the residual one (the output stream stalling) is the main open bug.

## ✅ FIXED this session (committed, do NOT re-litigate)

- **`setPhoneState` SIGABRTs audioserver** → `9dbb29d7`: gated behind
  `WART_AUDIO_SETPHONESTATE=1` (default off). It hung the vendor HAL `setMode` (no
  telephony path under --no-art) → audioserver TimeCheck 5s watchdog → SIGABRT.
- **Uninitialized volume range after audioserver (re)start** (`min=max=-1` → no gain →
  silent) → `03730627`: `audio_policy_impl::ensure_initialized()` self-heals on every
  `create_track` (re-runs `init_audio_policy` if range is -1).
- **`setForceUse(COMMUNICATION)` in the route-toggle path** disrupts the open stream
  (same `setOutputDevices→installPatch` family as the setPhoneState crash) → removed
  from `controls::set_route` (deviceId pin + track re-open only). [uncommitted as of
  handoff — see git status]
- Also fixed (unrelated): wrapped-paragraph line-spacing (`946b8602`); exported
  route/volume/mute controls + Signal call-UI panel (`d26eac47` + UI commit); guest
  re-opens the call track on route change (uncommitted).

## 🔬 OPEN BUGS (the investigation)

### 1. The output stream STALLS — `wr_ok` freezes, HAL stops pulling (MAIN)
**Symptom:** `calldbg.log` shows `peak=1.000` (engine decoded loud audio) but `wr_ok`
climbs briefly then **freezes** while `wr_zero` climbs (the host output ring fills and
the HAL output thread stops consuming) → silence. Seen as `wr_ok=1` (stalled at open)
and `wr_ok=~284 then frozen` (stalled after a bit). audioserver is ALIVE (no crash).
**Leading theory (source-grounded):** the audio_impl note (`audio_impl.rs:183-188`):
*"MUST drain the service→client up-message queue or the service's writeUpMessageQueue
fills, it decides the client stopped, and it SUSPENDS + closes the stream."* → the
guest may not be draining that queue (or the legacy SHARED USAGE_MEDIA output path is
flaky under --no-art). Check: is the up-message queue drained each tick? Does the
AAudio/AudioFlinger output thread suspend the stream? A/B a stream that pulls (working
call) vs one that stalls — what differs in the up-message/stream state.

### 2. Why does audioserver keep degrading / needing restart?
Even with setPhoneState gated, audioserver ended up respawned/uninitialized repeatedly.
Remaining triggers to find: mediametrics SIGSEGV crash-loop (logcat, fires near init's
`setPhoneState NORMAL`); `init_audio_policy` STILL calls `setPhoneState NORMAL` at the
end (worked at boot but it's the toxic call — consider gating it); the task-96 bringup's
deliberate audioserver pkill.

### 3. Guest UI freezes after call-audio trouble
After a stalled/broken call, the Signal UI goes unresponsive — guest main thread in
`hrtimer_nanosleep`, flat CPU (idle, NOT binder-blocked, NOT spinning); input doesn't
wake it. Recover = relaunch host (`wart-arbiter kill war.signal; launch war.signal`);
bg→fg does NOT fix it. Find why the guest falls into a non-waking idle after the call
loop hits errors (frame-pacing next-delay computed too long? the call loop stalling the
single-threaded guest?). On-demand-render: `[[reference_on_demand_rendering]]`.

### 4. Missing system_server services in wart-framework-shim (completeness)
audioserver/AudioFlinger look up `power` (IPowerManager) + `audio` (IAudioService),
neither in the shim. BOTH via non-blocking `checkService` (Threads.cpp:1261 for power;
PlayerBase/AudioRecord for audio) → they degrade gracefully (no playback wakelock, no
player tracking), NOT a blocking stall — so they are a real COMPLETENESS gap (the
playback thread runs without its wakelock) but probably NOT the direct cause of bug #1.
Worth adding `power`+`audio` GenericStubs (cheap, a-03 rebuild) as hygiene + to rule
them out. (Contrast: `batterystats`/`appops` DID block via getService/sleep-loop —
those were the proximity/sensor fix.)

### 5. Earpiece/speaker route-switch reliability
The route re-open (guest closes+reopens the call track on `controls::get_route` change)
is correct in principle (the deviceId pin moves the route at open), but each re-open
re-rolls the dice on bug #1 (stream-pull flakiness), and re-opening mid-call has been
fragile (contributed to bug #3 freezes). Stabilize after #1 is understood.

## Diagnostic tooling (use these)
- **Engine media counters:** `/data/local/tmp/wart-apps/apps/war.signal/0.1.0/state/calldbg.log`
  (dbg_line → `/state/calldbg.log`). Key fields: `peak` (decoded amplitude, 0=silent),
  `wr_ok`/`wr_zero` (output ring accepted vs full), `audio rx` (decoded frames),
  `srtp ok/err`. `wr_ok` climbing = stream pulling; frozen = stalled.
- **Host logs:** android_logger → logcat, tag `wasm_android_host::au..`
  (setPhoneState/onUpdateContextualVolumes/ensure_initialized/create_track route=).
- **Volume range:** `wart-host --probe-audio-volume` (logs `media volume range min/max`;
  -1 = uninitialized). **Audio caps A/B:** ART-up vs --no-art (the high-signal tool).
- **Shim trace:** `WART_SHIM_TRACE=1` on `wart-framework-shim` logs every transaction to
  the services it DOES host (see the missing-service-logging note below).

## How to verify (done when)
A `--no-art` Signal call: peer audio is **audible and stays audible** for the whole call
(`wr_ok` climbs continuously, never freezes); audioserver does not crash/degrade; the
earpiece↔speaker toggle switches the audible output without stalling; the UI stays
responsive after the call. No manual audioserver restart / Signal relaunch needed.

See `[[project_call_audioserver_crash]]`, `[[project_artless_call_audio]]` (task 91),
`[[project_call_audio_output]]` (task 75), `[[project_artless_audio]]` (task 87).
