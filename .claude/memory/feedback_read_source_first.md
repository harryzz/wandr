---
name: feedback_read_source_first
description: "STANDING RULE: read the authoritative source/docs/issues FIRST — do not instrument-probe + patch-by-assumption + cycle."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7d7dad2e-750c-4658-9e32-fc4c95e9f48e
---

**RULE (the user's #1 process complaint, 2026-06-04): when a working reference
implementation, official docs, or upstream issues exist, READ THEM FIRST — before
instrumenting, before patching, before assuming.** Do not solve by trial-and-error
patches. Clone/grep the reference, read the relevant module end-to-end, then act.

**Why:** On task #16 (incoming Signal calls / ringrtc interop) I burned ~5 hours
black-box instrumenting rtc-ice and stacking assumption-patches while CYCLING. The
actual fixes were plainly in ringrtc's source and took minutes to find once read:
- `connection.rs` — a 1:1 *caller* sits in `ConnectingBeforeAccepted` and streams
  NO media until the *callee* sends an `accepted` message over RTP-data (PT 101 /
  SSRC 0xD, `rtp_data.proto`). wart never sent it → no audio. [[project_wart_call]]
- The multi-device `Hangup{type=Accepted, device_id=N}` is coordination: ringrtc
  IGNORES a hangup whose `device_id` is its own. wart was hanging up on itself the
  instant it answered (wart = device_id 4).
- SRTP KDF was IDENTICAL to ours (verified by reading `negotiate_srtp_keys`) — so
  the hours I spent suspecting keys were wasted; reading would have ruled it out.

The user said this is in CLAUDE.md / their memory rules already ("check latest
versions / official sources") and that it matters a lot to them. It does.

**Why it matters THIS much here: wart is an OS runtime** (replacing Android's ART),
i.e. the foundation everything else stands on. A wrong assumption baked low
(SRTP key direction, ICE role model, a hardcoded inset) does not stay a local bug —
it propagates up and the only honest fix becomes a ground-up rebuild. The project
has already restarted from scratch 3× and done 2 ground-up refactors this cycle;
that IS the compounded cost of moving fast on a foundation. So on this project
"slow and source-correct" is the FAST path — the alternative to a careful read
isn't a bug, it's the 4th restart. Never trade correctness for urgency here.

**How to apply:** At the START of any interop/protocol/library task: (1) locate the
reference (clone ringrtc/libwebrtc/the crate source into /tmp or repros/, find the
upstream docs, search the repo's GitHub issues) and READ the relevant code paths
end-to-end; (2) only then form a plan; (3) instrument to CONFIRM a specific
reading, not to fish; (4) prefer one source-grounded change over many speculative
ones. If I catch myself on the 2nd+ "try this patch and test on device" without
having read the source path that governs the behavior, STOP and go read it.
Related: [[feedback_check_latest_versions]], [[feedback_visual_verification]].
