# wasi:media-source? — discussion notes (NOT a draft yet)

> task 108, 2026-06-14. The W3C **Media Source Extensions** (MSE:
> `MediaSource` / `SourceBuffer`) slot. Flagged as "needs additional talks"
> — recorded as open questions, deliberately NOT sketched as WIT yet.

## What MSE is, and why it's different from the other three

MSE lets a page feed **compressed media segments** into the playback pipeline
itself (instead of a single `src` URL) — the substrate for adaptive streaming
(HLS / DASH): fetch segments, append to a `SourceBuffer`, the browser demuxes +
decodes them. WebCodecs/Web Audio/Media Session each map to one wandr lane;
**MSE largely does not map to a host contract at all**, because in wandr's model
its job is already the guest's.

## The default answer: MSE ≈ guest-side, over the lanes we already have

In the layered design, streaming is guest-side:

```
wasi:http / wasi:tls  → fetch segments        (guest; reuse Signal's transport)
Symphonia              → demux + decode        (guest; or…)
wasi:audio-codec       → HW decode the segments (optional, when chosen — TUNNEL)
wasi:audio             → PCM sink              (the floor)
wandr:media-session    → now-playing/transport
```

So `SourceBuffer.appendBuffer(segment)` becomes either "Symphonia decodes the
segment" or "`audio-codec.decoder.submit(chunk)`". **ABR (adaptive bitrate)
logic, buffer management, live-vs-VOD, and gapless across segment boundaries
are all guest orchestration** — no new host verb. That's why MSE doesn't get a
WIT sketch alongside the other three: most of it isn't a host capability.

## What genuinely needs talks (the host-contract residue)

1. **DRM / EME (the big one).** MSE in the wild is paired with Encrypted Media
   Extensions (Widevine / PlayReady). A guest CANNOT hold content keys or do
   the license exchange in the clear — that's a hardware-backed host capability
   (Android `MediaDrm` / the TEE). This is a separate proposal entirely
   (`wasi:eme`? `wandr:drm`?) and the real reason MSE "needs more talk": a
   protected stream must decode in a secure path the guest never sees the PCM
   of — which interacts with TUNNEL decode (audio-codec) and with whether
   `read` (transcode) is even *permitted* for protected content. Decision
   needed: is DRM in scope for wandr at all, and if so, host-secure-decode-only?

2. **Live / low-latency buffering.** VOD is pure guest orchestration. Live
   (especially LL-HLS / LL-DASH) needs tight clock alignment and a jitter
   buffer policy. Open question: does any of that want a host primitive, or is
   `wasi:audio buffered-frames` + `playback.position` enough for the guest to
   self-pace? (Leaning: enough — but unproven.)

3. **A/V-synced streaming.** When audio MSE rides with video, the master clock
   is `wasi:audio playback.position` and the video decoder's timestamps. The
   present sketches assume that; a real A/V MSE player would exercise it and may
   surface a need for a true presentation timestamp (the ~40 ms lead noted in
   the M1 position impl).

4. **Container/segment edge formats.** fMP4 / CMAF init+media segments,
   HLS TS — all demuxable guest-side (Symphonia + a TS demuxer crate), so
   probably no host work; confirm no codec needs HW-only init data we can't
   pass through `audio-codec decoder-config.description`.

## Provisional verdict (to confirm in the talks)

No `wasi:media-source` package is needed: streaming is guest orchestration over
`wasi:http`/`wasi:tls` + `wasi:audio-codec` + `wasi:audio`. The ONE thing MSE
drags in that is a real, separate host-contract question is **DRM/EME** — now
sketched as `proposals/wasi-eme/`, **scoped ClearKey-only** (2026-06-14):
host-side AES-CTR, portable, covers self-served / Signal-style encrypted media;
Widevine deferred (TEE + Google-provisioned CDM, device-only) but reachable via
the key-system string with no contract change. Everything else MSE implies is
already covered by the floor + the codec lane.
