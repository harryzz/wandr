---
name: reference_video_player_present_clock_anchor
description: macOS video choppy (5Hz) = playback clock anchored at decode-start not first-frame; HW reorder latency baked in permanently
metadata: 
  node_type: memory
  type: reference
  originSessionId: 25b6eb4c-9122-4870-8734-7e515af11a68
  modified: 2026-07-24T12:41:44.721Z
---

**Symptom (task 117 macOS):** on-screen video played choppy (~5 Hz) on macOS
VideoToolbox while the SAME player+host was smooth on Linux (VAAPI). Not a HW
limit — same-era MacBook on PopOS/Linux was smooth.

**NOT the bottleneck (all measured + ruled out):** Skia composite `image_for`
0.0ms; GL swap 0.4ms / flush 0.1ms / purge 0.0ms; IOSurface zero-copy upload;
decode throughput. The present/draw path is fast.

**Root cause = present PACING, host loop starved to 5Hz.** The desktop render
loop (`lib.rs::about_to_wait`) only wakes at video rate when a FUTURE frame is
scheduled: `video_desktop::schedule_present(at_ns)` inserts into `SCHEDULED`
(→ `time_until_next_scheduled` wake source) ONLY if `at_ns > now`; else it hits
`present_now()` immediately and registers no wake. With nothing scheduled the
loop idles at the guest UI's `next_frame_delay` (~199ms = 5Hz), so video
composites at 5Hz no matter how many frames the player hands over.

**Why every `at_ns` was in the past (measured -456ms and GROWING to -776ms):**
the player (`wandr.video.player/src/lib.rs`) anchored its playback clock
`origin_ns` at decode-START, then `at_ns = origin_ns + relative_pts`. But VT
holds a **16-frame reorder window** (`videotoolbox.rs REORDER_WINDOW`) decoded
synchronously (~28ms/frame ≈ 448ms) before the first frame surfaces — so the
clock was born ~448ms in the past and the 5Hz loop (which pumps the player
slower) made it fall further behind: a vicious cycle. Linux's shorter reorder
path never triggers it.

**Fix (player, 2 lines + startup-staleness correction):** anchor `origin_ns` to
the FIRST frame's actual emergence, not decode-start — what mpv/ffplay do (master
clock starts at first output). Because `nanos` is sampled at render-frame TOP and
is stale by the decode time, correct it with a monotonic `Instant` measured
across the pump: `origin_ns = nanos + pump_t0.elapsed()`. `Instant` shares the
wasi:clocks monotonic timeline with `nanos` (host implements both as
CLOCK_MONOTONIC), so elapsed = exact staleness. Then steady-state frames land in
the FUTURE → `present(at_ns)` schedules them → host wakes at 30Hz.

**Verified:** 5Hz→30Hz, 300/300 frames, wall 8872ms vs clip 10033ms (real-time),
out-of-order 0. See [[project_wasi_canvas_migration]], [[reference_libvpx_wandr_video]].

**Not yet done (optional):** VT `REORDER_WINDOW=16` is the H.264 MAX, not the
stream's real `max_num_reorder_frames` (typ 2-4) — shrinking it would cut the
~448ms startup delay AND per-refill decode stalls AND memory (16 held
CVPixelBuffers). Risk: out-of-order if a stream reorders deeper; must verify on
pixels. The clock-anchor fix makes playback correct regardless of window size.
