# Task 120 — `wandr-media`: a pluggable media backend abstraction (PROPOSAL)

> **Status: PROPOSAL / not started.** Captured 2026-07-28 while swapping the
> jellyfin MKV demuxer (matroska-demuxer → oxideav-mkv). The swap was easy at the
> demux layer but touched app code directly — motivating a stable abstraction so
> demuxers/decoders/backends are swappable *without* rewiring each app per codec.

## The idea

Today `wandr.jellyfin` (and `wandr.video.player`, `wandr.audio.player`) wire media
handling **per container + per codec** directly in app code:
- demux: `mp4` crate (MP4) + `oxideav-mkv` (MKV) — hand-routed by codec id;
- audio decode: `ropus` (Opus) + `oxideav-ac3` (AC-3/E-AC-3) + `symphonia` (AAC/MP3),
  each behind a bespoke `AudioDec` arm;
- video decode + surface: host `wandr:video` + `wasi:audio` (present-at-ns / position).

Proposal: a guest-side **`wandr-media`** crate that virtualizes all of this behind
**one set of traits** — a `Demuxer` (per *container*, not per codec), a `Decoder`
(audio + video), and a `Surface`/present sink — so an app opens a stream and gets
frames/PCM without knowing which container crate or codec decoder is underneath.
Swapping a demuxer (like matroska→oxideav) or adding a codec becomes a registry
change, not an app rewrite.

## Why now / what it buys

- **Codec/container churn is isolated.** The matroska→oxideav swap edited the app's
  open path, `fill_queues`, and `do_seek`. Behind `wandr-media` it would be a
  one-line demuxer-registry change.
- **App code shrinks to policy** (which stream, seek targets, UI) — no `match
  codec_id` ladders, no per-codec `AudioDec` arms.
- **Reuse across apps** — jellyfin / video.player / audio.player share one media core.
- **Surface virtualization** — present/scale/rotate behind `wandr:video` already;
  fold it into the same abstraction so guests target `wandr-media`, not raw WIT.

## Strong lead: oxideav-core already models this

`oxideav-core` (already a dep via oxideav-ac3 + now oxideav-mkv) defines exactly
these traits: `Demuxer` (`streams()`/`next_packet()`/`seek_to()`), `Packet`,
`CodecParameters`/`StreamInfo`, `CodecResolver`, `Decoder`, `MediaType`. `oxideav-mkv`
IS an oxideav `Demuxer`. So `wandr-media` could largely BE **oxideav-core as the
backbone** + adapters:
- MP4: wrap the `mp4` crate as an oxideav `Demuxer` (or adopt `oxideav-isomp4` if it
  exists / is viable — evaluate like we did oxideav-mkv).
- Audio decode: adapt `ropus`/`symphonia` to oxideav's `Decoder` (oxideav-ac3 already is).
- Video decode + surface: a `Decoder`/sink backed by host `wandr:video` (present-at-ns)
  + `wasi:audio` for the A/V clock.

## Scope sketch (if pursued)

1. Evaluate oxideav-core as the trait backbone (maturity, 0.0.x/0.1.x churn, wasip2
   build, coverage) — the same verify-with-a-probe discipline used for oxideav-mkv.
2. `wandr-media` crate: demuxer registry (container-sniff → `Box<dyn Demuxer>`),
   decoder registry (codec id → audio/video decoder), a `Surface` present sink.
3. Migrate `wandr.jellyfin` onto it (proof), then `video.player` / `audio.player`.
4. Keep the host contracts (`wandr:video`, `wasi:audio`, `wasi:canvas`) unchanged —
   this is a GUEST-side library layer, not a WIT change.

## Open questions / risks
- oxideav ecosystem maturity (0.0.x crates churn — we pin exact today). Is it a safe
  backbone, or do we define our own thin traits and adapt crates behind them?
- Does an oxideav MP4/isomp4 demuxer exist and expose video like oxideav-mkv? If not,
  wrap the `mp4` crate.
- Subtitle tracks + multi-audio (task 119 Tier 3) should inform the `Demuxer` surface.
- NOT a WIT change; if any host verb is implied, that stops and asks (WIT-approval rule).

Related: [[reference_jellyfin_container_demux_and_mkv_seek]] (the swap that motivated
this), [[reference_gstreamer_desktop_backend_spike]] (host-side decode consolidation —
the same "one abstraction, swappable backend" lesson, but for the HOST decoder).

## Backlog — cue-less MKV seek polish (DEFERRED; reported, not blocking)

The vendored oxideav-mkv fork now does a true cue-less seek by **byte-offset
bisection** (`seek_by_bisection`, superseding the linear `seek_by_cluster_scan`),
paired with `open_streaming` (no open-time cue scan). Device-tested on "Home Alone 2":
seek WORKS but needs polish — a short FF (~1 s) has ≥~1 s delay, and the delay grows
with seek delta. The bisection range-request count is FLAT across distance, so the lag
is NOT the search I/O; it's two other costs. Polish when it matters (not now):

1. **Interpolation search** (kills the ~constant floor). Midpoint bisection takes
   ~log2(clusters) probes, each an HTTP round-trip. Cluster time is ~linear in byte
   offset, so seed the probe from the time fraction:
   `byte ≈ first + (target/duration)·(end−first)`; refine, keeping bisection bounds as
   the VBR fallback. ~10 probes → ~2–3, roughly independent of seek distance.
2. **In-cluster keyframe refinement** (kills the delta-proportional decode catch-up).
   We land on the CLUSTER ≤ target, not the KEYFRAME ≤ target, so the decoder grinds
   every frame from the cluster's keyframe up to target. Scan the target cluster's
   blocks for the last keyframe ≤ target (step back one cluster if it opens mid-GOP —
   usually a no-op for remuxes) so catch-up is < 1 GOP regardless of delta.
3. **Small-seek fast path**: a short FF/RW should keep demuxing forward + drop to
   target WITHOUT a full decoder reset + whole-file reseek.
4. **Measure first** (project rule): add probe-count / seek wall-time /
   frames-to-catch-up counters to confirm I/O-bound (→ #1) vs decode-bound (→ #2) on
   the real device network before patching.

Recommendation: #1 + #2 together (then #3 for nudge feel); these also strengthen the
upstream oxideav-mkv PR (a proper interpolation+keyframe seek, not a plain bisection).
Prototype numbers + the bisection impl live in `crates/wandr-media-engine/vendor/
oxideav-mkv/src/demux.rs` and `repros/fmp4-probe/src/bin/mkv_probe.rs`. See
[[reference_jellyfin_container_demux_and_mkv_seek]].
