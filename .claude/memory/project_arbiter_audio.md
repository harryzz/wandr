---
name: project_arbiter_audio
description: "wart-arbiter-audio — the AudioService arbiter module (audio focus, comms mode, routing). M1 (focus stack) done; M2 WIT + M3 comms/routing pending."
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**wart-arbiter-audio — arbiter module #7, the AudioService role** (decided over
[[reference_audio_policy_calls]]; distinct from a future `wart-arbiter-media` =
MediaSessionService/now-playing/transport). Driven by call audio (Signal VoIP).

**M1 — audio-focus stack: DONE + device-verified (commits `8543b125` +
`cc470049`), 6 unit tests + on-device CLI smoke.**
The cross-app arbitration (a guest can't pause another guest → the arbiter
must). Module-local `Vec<FocusEntry{pid,app_id,kind}>`, top = owner (focus is
runtime-only, NOT in the durable Store). Verbs `audio-focus-request <pid|app-id>
<kind>` / `audio-focus-abandon <pid|app-id>` / `audio-focus-list`. Kinds: `gain`
(evict all → each prior owner gets `loss`), `gain-transient` (prior owner
`loss-transient`/pauses), `gain-transient-may-duck` (prior owner `duck`). Abandon
restores the next owner with `gain`. `on_event(SurfaceRemoved)` drops a dead
owner + restores next. Each transition = `on-focus-changed <kind>` push via
`deliver_to_host` (HostLine). Registered in the bin with one line.
- Test harness pattern: `Registry::new()` + register + `dispatch_command(verb,
  args, &mut Store)` → `(Reply, Vec<Effect>)`, assert on `Effect::HostLine`.
  Run on HOST target (`cargo test --target x86_64-unknown-linux-gnu`) — the
  workspace cross-compiles to aarch64-android so default `cargo test` = Exec
  format error.
- Device CLI smoke (2026-06-02): restarted the daemon (the running one was the
  old binary) — `audio-focus-request 111 gain` → `222 gain-transient` → list
  (count=2, 222 owner) → abandon 222 (111 regains) → `333 gain` (evicts all,
  count=1). Arbiter restart was CLEAN: it reloads the persisted registry, the
  running hosts stay tracked (`list` showed all 6 apps incl. live Signal) — no
  session disruption. GOTCHA: a module verb must ALSO be added to the binary's
  client-side argv allow-list (run_client_multi match, main.rs ~line 134) or the
  CLI prints "unknown command" before forwarding to the daemon (fixed cc470049).
  Daemon restart: `setsid sh -c "LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=
  /data/local/tmp/wart-apps /data/local/tmp/wart-arbiter --daemon" </dev/null &`.

**M2 — `war:audio-focus` WIT + host wiring: DONE + device-verified (commit
`ef4986e8`).** Package `war:audio-focus` (no skiko-gfx WIT-sync, like alarm/
notify): interface `focus` (request(focus-kind)->focus-result + abandon),
interface `focus-handler` (on-focus-changed(focus-change)), worlds
audio-focus-host/audio-focus-events. Host: `audio_focus_host_impl` forwards
request→`audio-focus-request <pid> <kind>` (reads reply: OK granted/delayed →
Granted/Delayed) + abandon→fire-and-forget; 2 bindgen worlds + add_to_linker on
BOTH linker sites (app_loader.rs); `InboundEvent::FocusChanged` parses
`on-focus-changed <token>` (loss/loss-transient/duck/gain→0..3); standalone drain
calls the guest's focus-handler export (.ok()-probed). Test guest = war.alarm.test
(focus request on first frame + bar recolour). Device-verified via a STANDALONE
host (new binary, avoids restarting the live zygote): guest request(gain)→OK
granted; a competing `audio-focus-request 99999 gain` evicted it → host logged
`focus-inbound: dispatched on-focus-changed(0)` reaching the guest. Round-trip
both directions. (Verify trick: `wart-host --standalone --app <id>` runs the new
binary in isolation; the live app-hosts are zygote-forked from the OLD binary, so
a zygote/stack restart would otherwise be needed. **SIDE-EFFECT GOTCHA:** a
standalone `orientation=auto` guest (war.alarm.test IS auto) enables the
device-orientation sensor + issues `set-orientation-lock 0`, which UNLOCKS the
GLOBAL orientation and fans the accelerometer reading to EVERY surface — left the
locked launcher stuck landscape after the guest was killed. Restore with
`wart-arbiter report-orientation 0` + `set-orientation-lock 1`, or use an
`orientation=locked` test guest. 2026-06-02.)

**M3 — comms session + routing: DONE + device-verified (commit `24b3fcfe`), 10
unit tests + on-device host-applies test.** Arbiter verbs `audio-call-start/end
<pid|app-id>` + `audio-route <pid|app-id> <speaker|earpiece>`. call-start grabs
**GAIN_TRANSIENT** (music pauses + RESUMES after — like Android telephony, NOT
permanent gain) + marks `comms: Option<i32>` + HostLine `audio-policy set-mode
comm` + posts an ongoing-call badge (CALL_NOTIF_ID via the notify Store).
call-end → `set-mode normal` + drop_pid (music regains) + cancel badge. A dead
comms owner (SurfaceRemoved) ends the session. Host appliers: `audio_policy_impl`
`set_mode` (setPhoneState IN_COMMUNICATION/NORMAL, uid=getuid) + `set_route`
(setForceUse COMMUNICATION SPEAKER/NONE); `ime_inbound` parses `audio-policy
set-mode/set-route` → InboundEvent::CommMode/CommRoute → standalone drain applies.
The OWNER host (holds the binder connection) makes the global call. Client
allow-list updated for the new verbs (the M1 forwarding gotcha).
- **Device-verified (2026-06-02, user-authorized)**: standalone host pid 21790;
  `audio-call-start` → host logged `setPhoneState IN_COMMUNICATION (uid=0)` +
  badge `Ongoing call` in notify-list; `audio-route … speaker` → `setForceUse
  COMMUNICATION SPEAKER`; `audio-call-end` → `setPhoneState NORMAL`, badge
  cleared. Left clean: getPhoneState=0 (NORMAL), orientation re-pinned. setPhone-
  State accepts uid=0 (root). (Standalone auto guest = orientation gotcha again —
  restored after.) This IS a GLOBAL audio-mode change, so gate future runs.
**M3b — power keep-alive (no doze mid-call): DONE (commit `da5411d9`),
unit-verified.** A live call must keep running screen-off. New core
`Event::CommsActive { pid, active }`: the audio module emits it on
audio-call-start(true)/end(false); the power module ([[project_chrome_coherence]]
sibling) keeps the pid in a `comms` keep-alive set + `cadence_for` returns 0
(never doze) for it (overrides bg-service/normal); re-fans immediately if a call
starts/ends mid-doze; cleaned on SurfaceRemoved. WM exhaustive Event match needed
the ignore arm (recurring gotcha when adding a core Event). 5 power + 10 audio
tests green. Device-verify gated (needs a real call = setPhoneState + screen-off).

**wart-arbiter-audio is COMPLETE: M1 focus + M2 WIT + M3 comms/routing + M3b
doze-exemption.** Remaining downstream = the WebRTC/ringrtc call engine in the
Signal guest (the real consumer). [[reference_audio_policy_calls]] has the policy
surface. `wart-arbiter-media` (MediaSessionService) is the future sibling.

**OPERATIONAL GOTCHA (cost ~20min this session): repeated arbiter restarts DROP
runtime surface/role state.** The statusbar/taskbar self-register as Role::Chrome
ONCE at host startup; an arbiter restart reloads the persisted registry (apps +
[fg]) but NOT the runtime Chrome surfaces, so `fan_overlays`→Chrome stops
reaching them → the app rotates but chrome stays portrait (orientation
incoherence). Surgical fix: `register-chrome <app-id> <pid> <top|bottom-bar>`
(statusbar=top, taskbar=bottom-bar) re-adds the Chrome surface. Robust fix: a
clean `tools/scripts/run-hybrid-stack.sh` (re-registers ALL surfaces + brings the
new host live everywhere; preserves apps + /state; does NOT wipe APPS_ROOT). For
device-verifying host changes, prefer a full stack restart over repeated arbiter
restarts + standalone hosts.
