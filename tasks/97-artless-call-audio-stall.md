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

### 1. The output stream STALLS — `wr_ok` freezes, HAL stops pulling (MAIN) — ✅ ROOT CAUSE CONFIRMED 2026-06-08
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

**FIX SPACE (NOT implemented — confirm-only session):** (a) host-side timer-driven
up-queue drain independent of guest writes (suspend can never latch even if the guest
stalls); (b) host feeds silence into the ring when the guest underruns (no underflow →
no XRUN flood → no suspend at the source — cleanest); (c) keep the guest call loop alive
(also fixes bug #3). Prefer (b) or (a).

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
