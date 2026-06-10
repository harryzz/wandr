---
name: project_wandr_call_video_track
description: "Task 93 Phase 3 ✅ wandr-call VP8 video track device-verified (camera→SRTP/UDP→decode 17.4fps + PLI/SR on-wire); ringrtc video wire constants — VP8 PT 108, SSRC base 1000/2000 (+2 audio/+3 video)"
metadata: 
  node_type: memory
  type: project
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**✅ Task 93 Phase 3 DONE + DEVICE-VERIFIED (2026-06-10).** `crates/wandr-call/src/
video.rs` (`VideoStream`, owned by `MediaSession`): the VP8 RTP video track rides
the SAME per-direction SRTP contexts as audio on its own SSRC (the property
`send_rtp_data` already used). Engine stays WIT-agnostic: encoded frames + 90 kHz
ts in/out; the guest bridges to `wandr:video` ([[project_wandr_video_host]]).

**ringrtc wire constants (source-grounded, signalapp/webrtc rffi
`peer_connection.cc` — fetch raw from GitHub, the rffi is NOT in the ringrtc repo):**
- PTs: DATA 101, **OPUS 102, VP8 108**, VP8-RTX 118, VP9 109, H264 103/104.
- 1:1 SSRCs: `BASE = offerer 1000 / answerer 2000`; audio = BASE+2, video =
  BASE+3, video-RTX = BASE+13. (Retro-explains the captured peer audio ssrc
  0x7d2 = 2002.) wandr-call video uses 1003/2003; audio still 0xA/0xB (works —
  libwebrtc accepts unsignaled SSRCs when the PT is registered; Phase 5 may align).

**Design points / gotchas:**
- RTP payload budget = 1200 − 12 (RTP hdr) − 16 (GCM tag) = 1172; PictureID ON
  (`Vp8Payloader.enable_picture_id` — field is private, set after `default()`).
- RTCP = SRTCP through the same contexts; **RFC 5761 demux on byte 1 ∈ 192..=223**
  (our PTs ±marker never collide). The Phase-2 `external-aead` cipher implements
  `encrypt_rtcp/decrypt_rtcp` → host-AES covers RTCP too.
- Reassembly is in-order, torn frames NEVER reach the decoder; any loss → AUTO-PLI
  (rate-limited 300 ms) because VP8 without RTX recovers only via keyframe. The
  PLI needs the peer's video SSRC (learned from its first packet) — `pli_wanted`
  stays pending until then.
- Peer PLI/FIR → `take_keyframe_request()` → guest must call encoder
  `request-keyframe` (the qcom encoder ignores i-frame-interval, so this is the
  ONLY keyframe source mid-call).
- v1 congestion control = fixed bitrate + `peer_remb_bps()` (REMB parsed);
  `external/rtc` has RTCP TWCC types but no BWE engine — deferred.
- A/V sync: ~1 Hz video SenderReport out; `peer_sender_report()` = (ssrc, ntp,
  rtp_ts) anchor in.

**Verified:** 21/21 engine tests (fragmented round-trip, PLI+SR cross the wire,
lost-fragment→drop+auto-PLI). Device (`wandr.video.test` Part 2): camera → HW VP8
→ PeerSession A → SRTP over real wasi:sockets UDP → B → 88/88 frames 0 broken →
HW decode 17.4 fps; PLI round-trip answered; 2× runs, no cameraserver wedge.
Next: Phase 4 `Role::Video` decode-to-surface render, Phase 5 Signal protocol.
