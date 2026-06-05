---
name: project-call-screen-power
description: "DESIGN (not built): mode-aware screen-power policy during calls — video/route priority over proximity; the idle-timeout-during-call gate; the latent speaker/video hand-wave-blank bug"
metadata: 
  node_type: memory
  type: project
  originSessionId: 023c2492-85e0-4052-bd04-4dc23f02fd88
---

**STATUS: scoped design, NOT implemented (2026-06-04). Future task.** Captures the
full screen-power-during-a-call picture so it's built coherently, not patched
piecemeal. Decided after the task-86 screen-off-timeout follow-on
([[project-artless-autobrightness]]).

**The model: one authority (`wart-arbiter-power`) arbitrates screen power by call
MODE.** Today it only knows `CommsActive{pid, active}` — NOT the route (earpiece vs
speaker) nor whether it's video. The correct policy is mode-aware; priority is
**video > proximity > idle**, and proximity is NOT a blanket winner/loser — only
earpiece-voice lets it win:

| Situation | panel | proximity-blank | idle-timeout |
|---|---|---|---|
| Video call | ON (forced) | OFF (watching — near-face must not blank) | OFF |
| Voice, earpiece (at ear) | proximity decides | ON (near→blank) | OFF (during call) |
| Voice, speaker (hands-free) | ON | OFF (on a table; hand-wave must not blank) | off, or relaxed |
| No call | POWER key | n/a | ON |

**LATENT BUG this fixes:** today proximity (task 78 / [[project_proximity_screen_off]])
blanks on ANY `CommsActive` regardless of route → a **speakerphone or video call
blanks on a hand-wave**. Route-aware gating fixes it (proximity blank only for
earpiece voice).

**Missing signals to build it:**
1. **Route → power.** `wart-arbiter-audio` already decides earpiece/speaker
   (`comms_speaker`, set by the `audio-route` verb) but it's PRIVATE to that module.
   It must reach `wart-arbiter-power` — carry route on `Event::CommsActive`, or add an
   `Event::CommsRouteChanged`.
2. **Video flag.** Nothing knows about video. The call app (Signal / `wart-call`) is
   the only authority and it can toggle mid-call → add a guest verb `comms-video <pid>
   <on|off>`. (No camera-in-use signal exists to derive it; coarse fallback = treat
   speaker-route as "screen likely in use", but that can't tell speaker-voice from
   video.)

**Quick partial already understood + pending:** the one-line idle gate — in
`PowerModule::on_idle_tick`, `if !self.comms.is_empty() { return; }` (power already
tracks `comms`) so the screen doesn't time out + lock mid-call. Clearly right for
video; for **voice-speaker** it's a PRODUCT CHOICE (keep screen on the whole call vs.
let it sleep when set down) — AOSP tends to let audio-only screens time out but keeps
video on. Decide before building.

**Open policy questions for the user:**
- Does a voice (audio-only) call still idle-time-out when away from the ear? (AOSP:
  roughly yes for audio-only, no for video.)
- Video signal source: guest verb (authoritative, recommended) vs derive-from-route.

Touches: `wart-arbiter-power` (mode-aware gating), `wart-arbiter-audio` (emit
route/video), `wart-arbiter-core` (Event fields), the call guest (video verb). See
[[project_proximity_screen_off]] (task 78), [[project_arbiter_audio]] (route/comms),
[[project-artless-autobrightness]] (the idle-timeout this extends).
