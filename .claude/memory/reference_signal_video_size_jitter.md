---
name: reference_signal_video_size_jitter
description: KNOWN ISSUE (low-pri) — Signal call remote video jumps ~10% mid-call; fix = fit-rect hysteresis
metadata: 
  node_type: memory
  type: reference
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-04T19:08:13.110Z
---

Reported by the user during task 120 video-rotation work; **noted in source, not
yet fixed** (deferred to future).

**Symptom:** on a Signal video call the remote (peer) video occasionally changes
size ~10% mid-call, seemingly around rotations. Rotation geometry itself is CORRECT
(fixed in task 120) — this is separate.

**Root cause (confirmed from logs):** the remote rect is aspect-fit to the peer's
coded dims (`fit_rect(l.remote, peer_dims, rotation)`). `peer_dims` comes from VP8
keyframe headers; the peer's WebRTC encoder adapts resolution (bandwidth/CPU) and
VP8 codes in 16px macroblocks, so the coded width snaps to multiples of 16 → the
aspect wobbles slightly (observed 0.50↔0.55) even for a stationary peer. The engine
re-fits on ANY `peer_dims` change, so every wobble moves the layout. It's driven by
signalling/the peer, not our geometry.

**Fix plan (future):** hysteresis on the RESULT, not a time debounce (a debounce
would lag genuine portrait↔landscape flips). In `apps/user/wandr.signal/engine/src/
call.rs` (the keyframe `peer_dims` re-fit ~line 600 AND the peer-rotation re-fit
~line 647): recompute `fit_rect` and only call `set_rect` when the new rect differs
from the current by more than a few percent. Source carries a `KNOWN ISSUE` comment
at the site. Related: [[project_wandr_call_video_track]].
