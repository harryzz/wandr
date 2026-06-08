---
name: project_call_audioserver_crash
description: "ROOT CAUSE — Signal call \"can't hear other side / silent everywhere\" under --no-art = setPhoneState crashes audioserver (HAL setMode hang → TimeCheck SIGABRT)"
metadata: 
  node_type: memory
  type: project
  originSessionId: 0469217c-e18c-466d-a654-cb7321915922
---

**Signal call symptom: they hear me, I hear nothing on ANY route (earpiece+speaker);
and the UI can freeze after.** Device-diagnosed 2026-06-08 via the engine's own
`calldbg.log` + logcat. NOT a volume duck, NOT routing, NOT the engine.

**The engine receive path is FINE.** `media Connected` counters during the call:
`udp rx` climbing, `srtp ok` climbing, `audio rx` climbing, **`peak=1.000`** (decoded
full-amplitude audio, not silent), **`wr_ok` climbing** (samples accepted by the host
output track, HAL pulling them). So the engine receives, decrypts, decodes loud audio,
and writes it. Silence is DOWNSTREAM, in the host audio policy.

**ROOT CAUSE — `setPhoneState` SIGABRTs audioserver under --no-art.** Chain (logcat):
1. host comms applier `on_update_audio_mode(comm)` (audio_policy_impl.rs:84) calls
   `set_phone_state` → `AudioPolicyService::setPhoneState`.
2. → `AudioPolicyManager::setPhoneState → setOutputDevices → installPatch →
   DeviceHalHidl::setMode` on the vendor audio HAL **HANGS** (logcat: "No HAL process
   pids available"; the qcom audio HAL's `setMode(IN_COMMUNICATION)` tries to engage the
   telephony/voice-call audio path, which doesn't exist under --no-art / no-SIM → blocks).
3. → audioserver **TimeCheck watchdog (5s) → SIGABRT** (`F/DEBUG Abort message:
   TimeCheck timeout for IAudioPolicyService::setPhoneState`).
4. → `media.audio_policy` DIES → host gets `DeadObject`, then `NameNotFound` ~30s.
5. → audioserver respawns (class core) but volume ranges NOT re-initialized →
   `setVolumeIndexForAttributes err: IllegalArgument` → MUSIC stream at -inf →
   **silent everywhere** (stream still renders → wr_ok climbs, but at no gain).
6. Downstream: the guest's call loop hits the audio chaos and can fall idle (main
   thread `hrtimer_nanosleep`, flat CPU — NOT blocked on binder, NOT spinning); input
   doesn't wake it → **frozen UI** (recover: relaunch the host `wart-arbiter kill
   war.signal; launch war.signal` — bg→fg does NOT fix it; session persists in state/).

**This SUPERSEDES [[project_artless_call_audio]]** which used `setPhoneState
IN_COMMUNICATION` as the earpiece fix — it now crashes audioserver under --no-art.

**FIX (proposed, not yet implemented):**
1. **Stop calling `setPhoneState` under --no-art** — it's the crash trigger. Earpiece
   routing already works via `setForceUse(COMMUNICATION)` + the per-stream `deviceId`
   pin (`set_comms_route`), neither of which goes through the crashing setMode/patch
   path. (host has no clean --no-art runtime flag — all implicit; this stack is always
   --no-art.) Verify earpiece still routes without it (the deviceId pin does the work).
2. **Self-heal on audioserver restart** — detect `media.audio_policy` DeadObject/
   NameNotFound→ready and re-run `init_audio_policy()` (re-init volume ranges) so a
   restart never leaves the stream at IllegalArgument/-inf. Safety net regardless of #1.

**SELF-HEAL WIRED (2026-06-08):** the dominant silent-call cause turned out to be the
audioserver coming up with its **MUSIC volume range UNINITIALIZED (`min=max=-1`)** after
ANY (re)start — under --no-art system_server's boot `initStreamVolume` is dead, so a
respawned audioserver has no valid range → every index invalid (`setVolumeIndexForAttributes
failed ... wrong index 0 min=-1 max=-1`) → stream plays at no gain → silent (engine fine:
peak=1.0, wr_ok climbs). A manual `--init-audio-policy` right after a restart can RACE the
audioserver and not apply (saw it: first run no-op, second run `12/12 streams`). Fix:
`audio_policy_impl::ensure_initialized()` (reads `media_volume_range`; if `min<0||max<0` or
service unreachable → re-runs `init_audio_policy`), called at the top of
`audio_impl::create_track` so every call self-heals before the stream opens. Host change,
needs rebuild+deploy.

‼️ **TWO OPEN FOLLOW-UPS (2026-06-08):**
1. **WHY does audioserver keep restarting/degrading?** The setPhoneState SIGABRT is fixed,
   yet audioserver still ended up respawned/uninitialized repeatedly this session. Find the
   remaining trigger (candidates: the route toggle's `setForceUse`→setOutputDevices→installPatch
   HAL path — same family as the setPhoneState crash; mediametrics SIGSEGV crash-loop seen in
   logcat right after init's setPhoneState NORMAL; or the task-96 bringup's deliberate
   audioserver pkill). NOTE: `init_audio_policy` ITSELF still calls setPhoneState NORMAL at the
   end (worked at boot, but it's the toxic call — consider gating it too).
2. **Earpiece/speaker toggle has NO effect mid-call.** `create_track` reads the route ONCE at
   open (`Route::Call { speaker: comms_route_speaker() }`, audio_impl.rs:526); `set_comms_route`
   only changes the deviceId pin for the NEXT open, and `setForceUse(COMMUNICATION)` doesn't move
   the USAGE_MEDIA call stream. So `controls::set_route` mid-call changes nothing audible. FIX =
   on route change, RE-OPEN the call output track with the new route (guest re-creates the track,
   or host re-routes the open stream). The deviceId pin works — it's just only applied at open.

Live recovery used 2026-06-08: `wart-host --init-audio-policy` (restores volume ranges)
+ relaunch Signal host. calldbg.log at
`/data/local/tmp/wart-apps/apps/war.signal/0.1.0/state/calldbg.log` (dbg_line →
`/state/calldbg.log`). Host logs via android_logger → logcat (tag
`wasm_android_host::au..`). See [[project_call_audio_output]], [[project_arbiter_audio]].
