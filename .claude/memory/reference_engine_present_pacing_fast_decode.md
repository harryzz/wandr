---
name: reference_engine_present_pacing_fast_decode
description: "wandr-media-engine pump paces video via a TIME-based submit clock-gate, not just DECODE_AHEAD count — a fast-decoding stream (tiny DASH rep) races without it"
metadata: 
  node_type: memory
  type: reference
  originSessionId: c6bfd2e3-58ed-44e9-8de8-85655ad45867
  modified: 2026-08-02T06:57:15.729Z
---

**The `wandr-media-engine` pump (`pump_stream`) needs a TIME-based submit gate, not
just the `DECODE_AHEAD` frame-count cushion.** The present loop does
`p.presented += 1` for EVERY frame `next_decoded()` returns — including frames it
then late-drops (`pts < media_now - LATE_DROP_US`) or schedules far in the FUTURE
via `present(at_ns)`. So the count gate `submitted < presented + DECODE_AHEAD` is
DEFEATED whenever the decoder outpaces realtime: `presented` races, the gate stays
open, and the guest submits the whole movie in seconds.

Large video (jellyfin, 1080p) is DECODE-BOUND (~realtime), so it never trips this
and looked fine — masking the bug. A tiny fast-decoding stream (task 119 DASH
`video_eng=401000` = 224×100) decodes far faster than realtime → video raced
~2.5-3× and stuttered; the audio-master clock stayed correct (`media_now` tracked
wall-time), so audio was fine but video ran away.

**FIX (task 119 B1):** a second, TIME-based cushion in the submit loop —
`const SUBMIT_LEAD_US: i64 = 2_000_000;` and `if vf.pts_us > media_now +
SUBMIT_LEAD_US { break; }` before `submit_timed`. Caps how far ahead of the
playback clock the decoder is fed, so present schedules ≤2 s of future frames and
the host paces them at their `at_ns` deadlines. `SUBMIT_LEAD_US` ≫ `DECODE_AHEAD`
(20) frames' worth, so the reorder cushion is still satisfiable. Realtime-decode
video never hits the cap → jellyfin pacing unchanged (verified builds green;
device re-verify pending). Diagnosed live on desktop (WSLg) via a pump heartbeat
logging `presented`/`media_now`/`submit pts`.

Desktop gotcha (unrelated but same session): the host `video_desktop` counter
`live textures` reads 0 even when video shows, because the desktop path presents
via `decode-to-surface` GL, not the texture registry that counter tracks — don't
treat `live textures 0` as "no video" on desktop. And **kill old `wasm-android-host`
processes between desktop runs** — stale players contend for the WSLg PulseAudio
device and later launches get no audio. Related: [[project_desktop_dev_loop]],
[[reference_mp4_014_fragmented_read_sample_broken]].
