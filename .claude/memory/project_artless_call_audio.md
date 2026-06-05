---
name: project_artless_call_audio
description: "Task 91 — ART-off Signal call earpiece audio + mic + proximity-screen-off, both device-verified"
metadata: 
  node_type: memory
  type: project
  originSessionId: 8a79d726-5989-436c-93ca-fceb8f26e051
---

Task 91 — two-way voice in a `--no-art` Signal call: ✅ SOLVED + device-verified
(2026-06-05). Two independent bugs, both fixed:

**Earpiece audio + mic** — the earpiece output only opens in IN_COMMUNICATION on
this device (NORMAL → `-889`), but the old bare `setPhoneState(IN_COMMUNICATION)`
ducked the USAGE_MEDIA call stream to ~1% (task 75) so it had been removed →
earpiece dead. FIX = replicate `AudioService.onUpdateAudioMode`: setPhoneState
**then** re-apply volume for the new mode (`onUpdateContextualVolumes`). The
volume re-apply was the missing half. Host `audio_policy_impl.rs` now has
Java-mirrored `on_update_audio_mode()` = `set_phone_state()` +
`on_update_contextual_volumes()` (re-assert MUSIC full-scale on earpiece+speaker).
Arbiter `cmd_call_start/end` send `audio-policy set-mode comm/normal`. Verified:
AudioFlinger output thread → `0x1 AUDIO_DEVICE_OUT_EARPIECE`, patch flips
SPEAKER→EARPIECE at call start, SHARED/legacy path, no `-889`. WATCH: earpiece
MUSIC stream read −32 dB in dumpsys (audible but verify vs ART loudness).

**Proximity screen-off during call** — root cause: Signal engine called
`focus::call_start()` nowhere (only `call_end`) → no `CommsActive` → proximity
never armed. Added it at both connect sites (incoming Accept + outgoing
Connected). Verified: `set_display_power` toggles false/true on cover/uncover.

Commits 47cde354 (guest `engine.rs`) + 3f75ddb0 (host+arbiter). Builds on
[[project_call_audio_output]] (task 75 un-duck), [[project_artless_audio]]
(task 87 MMAP stubs), [[project_proximity_screen_off]] (task 78 chain).

**STABILITY ROOT CAUSE (2026-06-05, commit c0feef34): run exactly ONE
wart-activityms.** That binary hosts the stub system_server binder services
(activity/permission/sensor_privacy/scheduling_policy) the audioserver needs under
--no-art. It was spawned by every --no-art bringup but killed by NO teardown →
instances accumulated (16 found live). Each re-addService'd the same names,
churning the registrations the audioserver caches handles to; the
permission/attribution path (output/RX stream open) then hit a shadowed/dead stub
→ audioserver wedged → restarted (NEW tid each time) → stream volumes wiped to
-inf → earpiece silent + live capture torn down (mic stops). FIX = pkill old
wart-activityms before the spawn + in all teardowns (--restore-art critical: else
the stub shadows the REAL system_server when ART returns). Verified: 1 instance,
audioserver pid STABLE through a full call, two-way audio.

GOTCHA (cost ~1 call): **do NOT run `dumpsys media.audio_flinger` during a live
--no-art call** — it forces the audioserver down the permission/attribution path
into our stub, which (when unstable) crashes → kills the live call's mic. It is
NOT a passive read here. Check audioserver health with `ps -o ELAPSED` / pgrep pid
stability instead (a changed pid = restart = the bug). Audioserver restart wipes
all stream volumes to -inf (no onAudioServerDied recovery under --no-art) → silence.

GOTCHA: arbiter `log::info` → STDERR, NOT the logfile — `wart-arbiter.log` only
shows the wart-screen subprocess `set_display_power` lines, not call-start/set-mode
decisions. Confirm via `ps`/pgrep + the user's ear, NOT dumpsys media.audio_flinger.

GOTCHA: the recurring --no-art DNS breakage was the **Intra DoH VPN app**
(`app.intra`, IntraVpnService) creating tun0/netId 101 + hijacking DNS to its
internal resolver. Uninstalled 2026-06-05 — DNS now clean across --no-art cycles.

DNS-on-`--no-art`-entry breakage (task-88 teardown gap): a stale VPN netId (101)
resolver from the ART→`--no-art` transition points app/guest DNS at the dead VPN
DNS. Manual self-heal: `adb shell "su 1000 -c '/data/local/tmp/wart-net
--netd-config 101 wlan0 <ip>/<prefix> <gw> <gw> 8.8.8.8'"` (uid system; route part
errors harmless — `15000: from all lookup wlan0` catch-all already routes). Durable
fix = wart-net re-asserts the live-netId resolver at bring-up. See [[project_artless_network]].
