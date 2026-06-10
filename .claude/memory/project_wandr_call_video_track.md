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

**Phase 5 additions (2026-06-10, implemented + deployed; LIVE CALL PENDING):**
- Video on/off = `rtp_data.Message.senderStatus.video_enabled` (tag 3/2),
  ACCUMULATED + resent ~1 Hz (ringrtc merges state — `update_and_send_rtp_data_
  message`); `receiverStatus.max_bitrate_bps` (tag 5/2) = the peer's requested
  SEND bitrate (wins over REMB); rtp_data `hangup` (tag 2) honored in-band.
  API: `set_video_enabled(call_id, on)` / `take_peer_video_toggle()` /
  `peer_max_bitrate_bps()`; `send_accepted` now sends the accumulated message.
- CVO rotation = header extension id **4** (`urn:3gpp:video-orientation`,
  fixed in ringrtc's generated SDP): out via `set_video_rotation(deg)`
  (= encoder `display-rotation()`: back = sensor orientation, front =
  (360−sensor)%360), in via sticky parse → `VideoFrame.rotation` → decoder
  `rotation-degrees` at open (REOPEN to change — mid-call rotation = follow-up).
- **Receive advert = VP8 ONLY** (`receive_video_codecs`): the peer encodes from
  OUR set; VP9-first (real ringrtc order) would get us VP9 we don't depacketize.
- wandr:video WIT `z-layer`: `behind-ui` (hole model) vs `above-ui` — the Signal
  call screen uses above-ui (dioxus-canvas has no clear-blend primitive yet).
- Signal app: engine `pump_video` beside `pump_audio` (lazy encoder on
  toggle+layout; decoder on first KEYFRAME with its CVO rotation —
  keyframe-gated mid-stream join requests a sync point); chat WIT `set-video` /
  `set-video-layout` / `video-status` through the CallIntent queue; UI camera
  button in the in-call header → full-screen `VideoCallScreen` (rects from
  surface size, engine dedups layout). Enable video only AFTER Connected.

**LIVE-CALL HARDENING (2026-06-10 evening — TASK COMPLETE, user-confirmed):**
- Peer video arrives **RED-wrapped (RFC 2198, PT 120)** — demux extracts the
  primary block; RED-ULPFEC on the same SSRC = transparent seq continuity.
- **TWCC receiver feedback is MANDATORY** (twcc.rs, ~10 Hz over SRTCP): a
  transport-cc-negotiated peer's send BWE ramps ONLY on it — without it it
  parks at ~36 kbps (the 'blurry video' bug). Live: 25→200 pkts/s after.
- **Never follow the peer's REMB below a floor (250 kbps)**: its receive
  estimate only grows from observed throughput — obeying a starvation
  estimate (live: 8 kbps!) death-spirals. Sending at the floor IS the probe.
- Advertise our receive budget (2 Mbps): RTCP REMB + rtp_data receiverStatus.
- **Aspect-fit from the peer's VP8 KEYFRAME coded dims** (RFC 6386 header
  bytes 6-9) — peers send landscape-coded (1280x720) always; shape ≠ ours.
- **‼️ ROTATION MECHANISM (3 failed attempts, the keeper):** layer
  `setTransform` is OVERWRITTEN per-buffer by BLASTBufferQueue;
  `native_window_set_buffers_transform` (producer window) is OVERWRITTEN by
  MediaCodec per queued buffer. The ONLY producer-proof place = the
  **CONTAINER's transform matrix** (`sf_media_set_geometry`: pre-rotation
  crop box, dims swapped at 90/270, matrix+position rotate it into the final
  rect; degrees CW, NOT HAL enums). CVO wire value = libwebrtc
  getFrameOrientation (front (sensor+dev)%360); device rotation fed live
  from the arbiter's Geometry orient push (dihedral 0/4/3/7 → degrees).
- Hit the STALE-ZYGOTE trap mid-debug: pushed the host binary without a
  stack restart → Signal forked the old image → HAL enums hit the degrees
  shim → identity matrix. The `[media] slot N geometry` logcat line exists
  to catch exactly this.
- Follow-ups (cosmetic, unscheduled): mirrored self-view; behind-ui hole
  (dioxus-canvas clear blend); 4-pose landscape face-validation.
