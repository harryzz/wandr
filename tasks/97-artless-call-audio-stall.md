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

### 1. The output stream STALLS — `wr_ok` freezes, HAL stops pulling (MAIN) — ✅ ROOT CAUSE CONFIRMED + ✅ FIXED 2026-06-08
**Symptom:** `calldbg.log` shows `peak=1.000` (engine decoded loud audio) but `wr_ok`
climbs briefly then **freezes** while `wr_zero` climbs (the host output ring fills and
the HAL output thread stops consuming) → silence. Seen as `wr_ok=1` (stalled at open)
and `wr_ok=~284 then frozen` (stalled after a bit). audioserver is ALIVE (no crash).

**✅ CONFIRMED MECHANISM (source `vendor/aosp-frameworks-av/services/oboeservice` +
device A/B via `wart-host --probe-call-stall`):**
1. A SHARED output stream that **underflows** (client wrote < a full burst) gets an
   `XRUN` service event written into its up-message FIFO **every mixer cycle**
   (`AAudioServiceEndpointPlay::callbackLoop` → `incrementXRunCount` → `sendXRunCount`
   → `writeUpMessageQueue`). Unlike timestamps (gated by `isUpMessageQueueBusy() >= 0.5`),
   **XRUN events are written UNTHROTTLED**.
2. The up-message FIFO is **128** deep (`QUEUE_UP_CAPACITY_COMMANDS`). The client drains
   it via `drain_up_messages` — called at the **top of `write_pcm_f32` on every call**
   (even the call that returns 0). If the client stops draining for ~240 ms of continuous
   underflow, the 128-deep FIFO overflows.
3. Overflow → `writeUpMessageQueue(): Queue full. Did client stop? Suspending stream.
   what = 3, Shared` (`what=3` = `AAUDIO_SERVICE_EVENT_XRUN`) → `setSuspended(true)`.
4. A suspended stream is **skipped by the mixer** (`if (clientStream->isSuspended())
   continue; // dead stream`) → `mMixer.mix()` never advances the client FIFO read
   counter → **`r` freezes** → `in_flight` pegs at capacity → `write_pcm_f32` returns 0
   → silence. **This is the stall.**
5. **Self-recovery:** un-suspend only happens when a later `writeUpMessageQueue`
   *succeeds*. Since `write_pcm_f32` drains the up-queue on every call, resuming writes
   empties the FIFO → the next service timestamp write succeeds → `Queue no longer full.
   Un-suspending the stream.` → the mixer resumes → `r` advances again.

**Device A/B (decisive, `--probe-call-stall 8 1 <drain>`):**
- **drain OFF during underflow:** `up_fill` climbed `0 → 128`, `r` froze at 252 ms,
  `Suspending stream what=3` logged; recovered only when Phase-3 writes resumed.
- **drain ON during underflow:** `up_fill` stayed at **5–7**, `r` kept advancing,
  **no** suspend — even though the underflow window was *longer*.
- The **only** variable was whether the client drained the up-queue during underflow.

**‼️ KEY IMPLICATION — bug #1 ⟺ bug #3 are the same event.** Because `write_pcm_f32`
always drains, a call **self-recovers as long as the guest keeps calling `write_pcm_f32`
at ~10 ms cadence** (even while it returns 0). A *permanent* silent call (needs host
relaunch) therefore requires the **guest to stop calling `write_pcm_f32` for ~240 ms+
during an underflow** — i.e. the guest call loop falling into the `hrtimer_nanosleep`
idle of bug #3. The underflow→suspend latch and the guest idle are one failure.

**✅ FIX IMPLEMENTED + device-verified (2026-06-08) — host call-output silence pump.**
`spawn_call_silence_pump` (`audio_impl.rs`) runs a per-call-output thread (spawned from
the guest `create_track` VoiceCall path) that, every 5 ms: (1) **always drains** the
up-message queue (so a suspend can never latch even if the guest stalls — covers fix
option a), and (2) **only when the guest has been silent > 25 ms**, tops the ring up to
~½ capacity with silence (so the mixer never underflows → never floods XRUN → no suspend
at the source — option b). Guest-staleness is tracked via `TrackState.last_guest_write_ns`
(set by `write_pcm_f32`, NOT by the pump's own writes) so the pump never fights the guest's
normal near-empty just-in-time feeding. The ring-write core was extracted to `ring_write()`
and shared by both. Scoped to VoiceCall (Media/Notification feed themselves); the
`--probe-call-stall` diagnostic opens via `open_routed` (pump-free) so it can still
reproduce the bug. **A/B proof (`--probe-call-stall 5 1 0 <pump>`):** pump OFF → `up_fill
0→128`, `r froze @252 ms`, `Suspending stream what=3`; pump ON → `up_fill` stayed 0, `r`
kept advancing the whole guest-stall window, no suspend, no fighting on resume
(`cum_zero=0`). NOTE: bug #3 (the guest idle freeze itself) is separate — the pump keeps
the *audio stream* alive through a guest stall (no relaunch needed), but the guest UI
still needs its own wake-up fix.

**Repro tooling (committed as permanent `--probe-*`):** `wart-host --probe-call-stall
[secs] [speaker0|1] [drain0|1]` (`audio_impl::probe_call_stall`) — primes a Call-route
SHARED output, forces a sustained underflow, logs ring + up-message cursors per tick;
A/B `drain` reproduces/​prevents the suspend. `play_tone` never trips it because it writes
as fast as the ring frees (never underflows). NOTE: the earpiece deviceId pin `[2]`
`-889`s when opened standalone (taimen quirk → bug #5); the probe reproduces on speaker
`[3]` (USAGE_MEDIA legacy SHARED — the same mixer/​suspend path).

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

### 5. Earpiece/speaker route-switch reliability — ✅ ROOT-CAUSED + ✅ FIXED 2026-06-08
**Root cause (source + device):** the call output's per-stream `deviceIds` pin made
AAudio open a SECOND MMAP "direct output" on the pinned device. The taimen
`mmap_no_irq_out` profile is **`maxOpenCount=1`**, so when another output already holds
it (e.g. on the speaker) the earpiece pin `[2]` returns **`-889`**
(`APM_AudioPolicyManager: ... can't open new mmap output maxOpenCount reached`). Speaker
`[3]` "worked" only because it *reused* the existing MMAP output (`openDirectOutput
reusing direct output 213`). So the toggle was unreliable: whichever device didn't
already hold the single MMAP slot failed.

**Fix (committed + device-verified):** the call output **no longer pins a deviceId**
(`audio_routing.rs` `Route::Call → device_ids = []`) — it shares the existing MMAP
output (never `-889`s). Routing is done by **re-routing that shared output** via a
PREFERRED device-role on its product strategy:
`audio_policy_impl::set_media_strategy_route(speaker)` →
`setDevicesRoleForStrategy(getProductStrategyFromAudioAttributes(MEDIA), PREFERRED,
[OUT_SPEAKER | OUT_SPEAKER_EARPIECE])`. Applied from `set_comms_route` (so a
speakerphone toggle takes effect **mid-call with no re-open**) and at call-track open;
**cleared on call-end** (`CommMode{comm:false}` → `clear_comms_route`). `setForceUse`
(no earpiece option for MEDIA) and the deviceId pin (the `-889`) are both retired from
the routing path. **Verify:** `--probe-route-toggle` — `getDevicesForAttributes(MEDIA)`
followed `(140) speaker → (141) earpiece → speaker → default` on ONE no-pin stream, no
`-889`, no re-open; and `--probe-call-stall 3 0 0 1` (guest path, earpiece) now opens
(`deviceIds=[] … track ready`, was `-889`). Audible earpiece confirm (cover the
receiver) / real-call A/B still worth a pass.

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
