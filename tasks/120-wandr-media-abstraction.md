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
