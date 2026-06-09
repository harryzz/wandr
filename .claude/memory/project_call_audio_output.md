---
name: project_call_audio_output
description: Task 75 — how Signal call audio OUTPUT was made audible on the Pixel 2 XL (4 device-specific AAudio root causes); receive-quality is the remaining problem
metadata: 
  node_type: memory
  type: project
  originSessionId: 917f8d71-aa61-49a1-a703-e5103ad6af82
---

Task 75 — call-audio **output** path SOLVED + device-verified (silent → continuous audio) 2026-06-02, UNCOMMITTED. Four independent root causes, all on the Pixel 2 XL (taimen, API 35). See `[[project_wandr_call]]`, `[[project_arbiter_audio]]`, `[[reference_audio_policy_calls]]`.

1. **`USAGE_VOICE_COMMUNICATION` output is UNOPENABLE on this device.** The audio policy routes it to a voice/telephony output with no AAudio mixer profile → MMAP open returns `-19` (ENODEV) for every format, **no legacy fallback** → `openStream -889`, in *every* phone mode. Only `USAGE_MEDIA` opens (primary mixer → Shared/legacy fallback succeeds). So call audio MUST go out as `USAGE_MEDIA`/`CONTENT_TYPE_MUSIC` (`audio_impl.rs` `create_track`).
2. **`USAGE_MEDIA` + `IN_COMMUNICATION` mode = ducked to ~1%** (`onVolumeChanged 0.01`) and parked (service readCounter frozen, mixer won't pull). Fix: DON'T enter comms mode — guest flag `COMMS_MODE=false` (`engine.rs`) gates both `focus::call_start()` sites. Then `onVolumeChanged=0.555` (full). Trade-off: no hardware AEC / earpiece routing (speakerphone-MEDIA path).
3. **Host playback ring is only 1536 frames (~32 ms).** Writing each decoded frame straight through dropped most of a jittery burst (`wr_zero` ≫ `wr_ok`). Fix: guest-side stereo playout FIFO (`ActiveCall.play_buf` in `call.rs`) — drain all decoded frames, write what the ring accepts, carry the remainder, 200 ms latency cap. `wr_zero`→0.
4. **The engine pump is coupled to the UI frame rate.** `wart-step-executor` advances ONE step per `poll_events`, called once per UI frame; on-demand rendering idled the call screen to ~2/s → call pumped ~1.7/s → ring underruns + UDP socket overflow. Fix: UI `pre_frame` sets `min_frame_delay=10ms` while `chat::call_status()` is active (`ui/lib.rs`), overriding the idle ramp AND the backgrounded floor. Repaints stay dirty-gated, so it's pump-rate not 100 fps render.

**DEAD END — do not retry:** forcing `aaudio.mmap_policy=1` (legacy, as reverted commit 23ec92e3 did) makes AAudioService **not register `media.aaudio` at all** on this device → all audio dead. Keep `mmap_policy=2` (AUTO).

**Receive quality SOLVED → INTELLIGIBLE VOICE (device-verified 2026-06-02).** The "~18 pkt/s / garbled" was NOT loss and NOT crypto — the peer sends **60 ms Opus frames** (ts_step=2880, plen=240 ⇒ ~17 pkt/s IS real-time), and we were decoding into a 20 ms (960-sample) buffer, dropping 2/3 of every packet. Fix: `opus_packet_samples()` TOC parser in `media.rs` → decode at the packet's **exact** sample count. CRITICAL: opus-rs 0.1.22 **panics** (`quant_bands` `E_PROB_MODEL[lm]`, lm=4 out of bounds → SIGILL kills the zygote child) on a *mismatched* frame_size — a fixed oversized buffer (5760 *or* 1920) crashes; only the exact size is safe (verified: 960@960 ok, 960@1920 PANIC, 960@2880 ok). **Crypto ruled out** by `repros/crypto-bench`: soft-wasm AES-256-GCM = 159k pkt/s (3000× real-time). **TURN ruled out**: host-only LAN path (`NO_TURN=true`) same rate.

**Pump rate FIXED → REAL VOICE (device-verified 2026-06-02).** Root: the host **fps-caps the foreground render loop** (`min_frame_delay=10ms` floored by `frame_interval`), so the engine pump (which rode inside `render_frame`) ran only ~20/s and the ~32 ms ring underran → glitches. Fix: made `bg_tick` **render-independent** — host (`standalone.rs`) now calls `call_bg_tick` in **all** roles on its own `next_bg_tick_at` timer (render stays foreground-only), and the guest `bg_tick` returns `10` ms during a call (host clamps to ~16 ms ≈ 60/s). `ticks/s` 20→48, `peak` settled 0.81, glitches gone. (This is the render-independent pump the user predicted.)

**POLISH LIST (user-noted, PAUSED here — RX voice works, these are follow-ups):**
- **(P1) Mic/TX** — `RX_ONLY=true` (`call.rs`) disables the wandr mic, so the desktop hears nothing from wandr. Flip false + wire mic→`send_audio`; watch in+out MMAP `-889` (`[[project_audio_mic_capture]]`).
- **(P2) Echo/"microphony"** — speakerphone (USAGE_MEDIA→speaker, no HW AEC since we avoid comms mode) feeds the mic back. Need software AEC or earpiece/headset routing.
- **(P3) Audio routing** — earpiece / wired headset / BT (speaker is the default). The comms/voice route is unopenable here, so routing must work on the MEDIA path.
- **(P4) TURN test** — re-test with `NO_TURN=false` over the Signal relay (confirm rate/quality hold off-LAN).
- **(P5) RTCP/2nd-SSRC into the decoder** — garbage `ts_step` + `gaps` jumping ~65535 + ~15% `decode_err`: filter so only audio-RTP (SSRC + payload type) hits decode.
- **(P6) Jitter buffer + reorder + PLC** — `session.rs` `audio_in` is a plain `Vec` FIFO.
- **Cleanup before commit:** strip diag (tick counter, rtp_diag fields, calldbg media line); finalize `COMMS_MODE`/`NO_TURN`/`RX_ONLY`. Restore: `pm enable org.thoughtcrime.securesms`, `settings put system accelerometer_rotation 1`.
