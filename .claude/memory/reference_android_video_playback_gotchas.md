---
name: reference_android_video_playback_gotchas
description: "Two non-obvious Android gotchas that made HW video playback show black/choppy — no host scheduled-wake source (guest must self-pace at frame rate), and the standalone EGL config lacked alpha (behind-ui hole-punch rendered opaque black). Both device-fixed task 117 M2."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 215f1733-fbc2-4004-aac8-cacd9719553d
  modified: 2026-07-24T15:35:16.012Z
---

Task 117 M2 wired `present(at-ns)` → `AMediaCodec_releaseOutputBufferAtTime` for
Android MediaCodec playback (`src/video.rs mod android`). The present-path wiring
itself worked first try on device (Pixel 2 XL, --no-art: `OMX.qcom.video.decoder.avc`
decode-to-surface). Two SEPARATE integration gotchas then made it look broken —
both apply to ANY future Android media/translucent guest, not just the player.

**1. No host-side scheduled-present wake source on Android → a guest that idles
its UI cadence runs at ~0.5×.** On desktop, `present(at-ns)` parks the frame in a
host `SCHEDULED` thread-local and `video_desktop::time_until_next_scheduled()`
wakes the render loop at frame rate to drain it — an INDEPENDENT video wake source.
So a player can request a lazy ~5 Hz UI cadence (`next_frame_delay`) and still get
pumped at 30 Hz. **On Android that source does not exist**: present hands the buffer
straight to SurfaceFlinger (`releaseOutputBufferAtTime`), there is no host queue to
wake on. A guest woken only at its requested ~5 Hz cannot feed/pull/schedule fast
enough to stay ahead of real time — every `at-ns` lands in the past (measured
−178 ms → −6.5 s and growing), frames present immediately, playback runs ~0.5× and
choppy. FIX: the guest must request a frame-rate cadence during playback
(`wandr.video.player next_frame_delay` 200 ms→16 ms). Then `at_ns` sits a stable
~+150 ms ahead and SF displays on the HW timestamps → true 30 fps. Distinct from
the desktop present-clock anchor ([[reference_video_player_present_clock_anchor]]),
which was a different root cause (clock origin) with the same "everything in the
past" symptom.

**2. The standalone EGL config had no alpha → behind-ui hole-punch composites
opaque black over the video.** `src/egl.rs` requested `EGL_RED/GREEN/BLUE_SIZE`
but no `EGL_ALPHA_SIZE`, so `eglChooseConfig` picked an RGBX (no-alpha) config.
Skia's transparent clear (`0x00000000`, the behind-ui hole the guest punches so the
decode-to-surface video BELOW shows through) is then stored opaque → SF composites
black over the video. Frames were decoding and reaching the SF media layer the
whole time (layer buffer counter advancing) — the picture was just occluded. FIX:
add `EGL_ALPHA_SIZE, 8`. Safe for every other app: the app SF layer is created
`eLayerOpaque` by default (`cpp/sf_surface.cpp`), so SF ignores the alpha until a
guest opts in via `sf_set_opaque(false)` — which a behind-ui decoder open already
does. Diagnostic that isolated it: flip the guest to `above-ui` (video covers its
rect, no hole-punch) → video appeared → proved it was the transparency path, not
decode.

Debugging notes for the Android --no-art stack:
- `adb shell input` is DEAD under --no-art (framework stopped). Inject touch via
  evdev `sendevent /dev/input/event1` (type-B MT) to unlock the keyguard.
- Guest `println!`/stdout is `inherit_stdout()` → LOST on the detached host; only
  `eprintln!`/stderr reaches logcat (`LogcatStderr`), and it emits one entry per
  guest write, so pre-format into a single String before printing or Rust's
  multi-fragment format args show up truncated.
- `wandr-arbiter launch <app>` forks the app from the ZYGOTE, so a stale in-memory
  zygote serves old code — redeploy the host and restart the zygote
  (`run-hybrid-stack.sh --wandr-only`; the full script bails on a mis-gated ART
  launcher-resolve when the device is already --no-art). Forked apps show as
  `wandr-host` in `ps`, not the app id.
