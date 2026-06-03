---
name: project_audio_routing_arbiter
description: Task 76 routing — DECISION of record — audio routing policy lives in wart-arbiter-audio (arbiter decides); host = capability model + applier; guest expresses intent. dumpsys port-enum is a tracked follow-up.
metadata: 
  node_type: memory
  type: project
  originSessionId: 60a5ba7d-3852-4a04-bc9b-dc30175ddbfb
---

Architecture decision (user-confirmed 2026-06-03) for task 76 audio routing,
consistent with the project-wide arbiter split ([[project_arbiter_window_server_design]],
[[project_arbiter_audio]]): **the arbiter decides, the host applies.**

- **Guest** expresses *intent only* — via the arbiter (`war:audio-focus`:
  request/abandon, ring-start/stop, **call-start/call-end**) + a **coarse stream
  class** on `create-track` (media / voice-call / ringtone) so the host knows
  which streams follow the comms route. NO routing-policy enum interpreted by
  the host.
- **wart-arbiter-audio DECIDES** the route (earpiece↔speaker, comms mode,
  ducking) — it's stateful (depends on focus stack, user speaker toggle, comms
  session). It already owns `audio-route <pid> <speaker|earpiece>` +
  `audio-call-start/end`. Extends to **push the per-pid device route to the
  host**, not just `setForceUse`.
- **Host (`audio_routing.rs`) = capability model + applier.** Owns the
  AAudio/dumpsys binding; maps the arbiter's *abstract* route → *concrete* port
  for THIS device (earpiece=2/speaker=3, enumerated by type) and pins
  `StreamParameters.deviceIds` at `create-track`. `Route`/`StreamPlan` stay as
  the shared vocabulary; the host no longer *picks* the route for calls — the
  arbiter does.

**Why deviceIds (not just setForceUse):** wart's call streams are `USAGE_MEDIA`
(voice-comm → -889; see [[project_audio_capability_model]]), and
`setForceUse(COMMUNICATION,…)` does NOT redirect a MEDIA stream — which is why
task-75 hand-pinned `deviceIds=[2]`. So the arbiter's route decision must reach
the host as a **per-stream device pin**, applied at open (re-pin = close+reopen).
DEVICE-VERIFIED 2026-06-03 (user): `audio-route <pid> speaker` → call audio on
loudspeaker on redial; earpiece default works. **Live mid-call re-pin** (toggle
while the stream is open) is a tracked follow-up — applies at open-time only now.

**dumpsys caveat (user-flagged):** the host caps model currently parses
`dumpsys media.audio_policy` at startup to get the port table. Avoiding runtime
dumpsys is a **tracked follow-up** (adjacent to P8/API-of-record, not literally
P8) — blocked on `listAudioPorts` over binder (fragile framework `AudioPortFw`
parcelable, deferred during the probe). `getDevicesForAttributes` gives device
*type* but not port id. Keep startup dumpsys for now; replace later.

Non-stateful routes (media→policy default, ringtone→speaker) are pure mechanism
and stay host-side applier mappings; only the stateful call earpiece/speaker
decision must round-trip through the arbiter.
