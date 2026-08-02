# Task 119 — media-player real-client proof (Jellyfin + open DASH/HLS)

> **Status: 🔲 NOT STARTED.** This is the open tail of task 117 M2 (see
> `tasks/117-wandr-video-consolidation.md`). 117's playback *engine* is done and
> user-verified on all four platforms (own `wandr.video.player` + host GStreamer
> decode + zero-copy); 117's stated "done when" for M2 is proving it end-to-end
> against a **real streaming client**, then feeding the upstream `wandr:video`
> proposal ("prove with real consumers first"). This task is that proof.

## Goal

Play real streamed media through the SHIPPED pipeline, unchanged:

```
guest (wasip2): resolve → HTTPS range-fetch (wandr-reqwest/wasi:tls) → demux → feed
host: GStreamer decode (H.264 / H.265 / VP9 / AV1) → zero-copy present
sync: guest-side, video present(at-ns) vs wasi:audio position(); audio decode guest-side (Symphonia)
```

The point is to stress the parts we have NOT yet exercised with real-world input:
container demux variety, HTTPS range + seek, and **adaptive (separate video+audio)
A/V sync** — with the codecs the GStreamer consolidation just unified. Two clients,
easy-first:

1. **Jellyfin** — a clean, stable, self-hosted HTTP media server. Do this FIRST.
2. **Open DASH/HLS** (CMAF test streams; optionally PeerTube) — the messy real-world
   adaptive/DASH case, no Google fortress. Do SECOND. (YouTube parked — server-assisted only.)

## Binding architecture constraint

The client is a **wasip2 WASM guest**. That rules out the whole class of "call a
native binary" extractors and forces the resolve step to be either pure-Rust-in-guest
or a plain HTTPS-JSON API call. Extraction/demux stay guest-side by design
(`proposals/wasi-media-source/NOTES.md`); the host only decodes.

‼️ **DirectPlay, never server transcode.** Both servers can transcode server-side —
which would mean *their* CPU does the decode and hands us a re-encoded stream. That
defeats the proof. We must negotiate DIRECT PLAY (server ships the original container
bytes untouched) so OUR guest demux + host GStreamer decoder do the work. Advertise a
device profile listing exactly the codecs/containers we support so the server picks
DirectPlay.

---

## Part A — Jellyfin (do first)

Stable, self-hostable, well-specified. OpenAPI 3.0 at <https://api.jellyfin.org/openapi>;
interactive Swagger/ReDoc served by any instance at
`http://<host>:8096/api-docs/swagger/index.html`.

### Rust crates — DECISION: hand-roll over the in-tree HTTP client
- [`jellyfin-sdk`](https://crates.io/crates/jellyfin-sdk) — newest (0.1.0, 2026-01-03),
  best-built: reqwest 0.13 + **rustls**, retries/backoff, pagination, streaming download,
  `set_token`/`clear_token`, raw escape hatch. BUT built on **tokio** (`time`/`fs`/`io-util`)
  + reqwest's native connector — **no wasm32/wasip2 positioning** (confirmed on lib.rs).
- [`jellyfin_api`](https://docs.rs/jellyfin_api/) — OpenAPI-generated, large surface, reqwest.
- [`jellyfin-sdk-rust`](https://docs.rs/jellyfin-sdk-rust) — async, reqwest-based.

⚠️ Our client is a **wasip2 guest** → HTTP goes through `wandr-reqwest`/`wasi:tls`, no tokio
runtime; all three SDKs are tokio+reqwest-native and won't drop in. The surface we need is
tiny (auth + list + PlaybackInfo + stream URL + image), so **hand-roll those calls** over the
in-tree HTTP client and use `jellyfin-sdk` / the OpenAPI types only as a SHAPE reference.

### Client state & thumbnail cache — all guest-side in the `/state` preopen (no host change)
Both persistence needs sit in the guest's private `/state` preopen (the universal
`/assets`·`/state`·`/system-fonts` convention) — no new WIT, no host code, no per-app
hardcoding; same pattern Signal uses for its session.
- **Token / session** — `/state/jellyfin/session.json` = `{ server_url, user_id, device_id,
  access_token }`. `device_id` is stable per install (part of the `MediaBrowser` auth header)
  — generate once, keep. Jellyfin tokens are long-lived (no expiry unless revoked / password
  change) → store-once; on `401`, re-run `AuthenticateByName` and rewrite the blob.
- **Cover thumbnails** — `/state/cache/thumbs/<itemId>_<imageTag>_<w>.webp`. Jellyfin's image
  URL (`/Items/{id}/Images/Primary?tag=<imageTag>&maxWidth=…&format=Webp`) carries an image
  **`tag`** (content hash) = a BUILT-IN cache-busting key (tag change ⇒ refetch). Request
  **WebP + capped `maxWidth`** to keep disk/bandwidth small; guest-side decode; simple
  size-capped LRU sweep so the cache can't grow unbounded. ‼️ Server-side IMAGE resize is
  fine — it's poster scaling, NOT media transcoding; the "DirectPlay, never transcode" rule
  is about the VIDEO stream only.

### The 4-call flow
1. **Auth** — `POST /Users/AuthenticateByName` `{ "Username":…, "Pw":… }` → `AuthenticationResult`
   with `AccessToken` + `User.Id`. Every later call carries the header:
   `Authorization: MediaBrowser Client="wandr", Device="…", DeviceId="…", Version="…", Token="<AccessToken>"`
   (or `X-Emby-Token: <token>`).
2. **Browse** — `GET /Users/{userId}/Items?IncludeItemTypes=Movie,Episode&Recursive=true`
   → item `Id` + `MediaSources[]` (container, codecs, size).
3. **Negotiate DirectPlay** — `POST /Items/{itemId}/PlaybackInfo` with a **DeviceProfile**
   whose `DirectPlayProfiles` advertise our containers (mp4/mkv/webm) + video
   (h264,hevc,vp9,av1) + audio (opus,aac) and whose `TranscodingProfiles` are empty/minimal.
   Response resolves to DirectPlay → gives the `MediaSourceId` + confirms no transcode.
4. **Stream** — `GET /Videos/{itemId}/stream.{container}?static=true&mediaSourceId=…&api_key=<token>`.
   `static=true` = raw byte stream (no transcode), honors HTTP **Range** → range-fetch →
   guest demux → host decode. (Transcode fallback, which we AVOID, is the `master.m3u8` /
   `stream` non-static variants.)

### Transport / seek — static + HTTP Range, NOT HLS
Pause / FF / REW do **not** need HLS. `static=true` serves the raw file with
`Accept-Ranges: bytes`, and all three controls are guest-side over Range:
- **Pause** = stop the clock + stop pulling bytes (no request).
- **Seek/rewind** = new `GET` with `Range: bytes=N-` (→ `206`), then decoder
  `flush`/`reset` (already shipped, 117 M2) + re-anchor the clock.
- **Fast-forward** = same seek; visible FF = feed **keyframes only** (demux decision).

The ONE requirement: byte offset ≠ time offset, so the demuxer needs the container
**index** to map timestamp→byte→nearest keyframe — MP4 `moov`/`stss`, MKV/WebM `Cues`.
⚠️ MP4 gotcha: if the file isn't faststart, `moov` is at the END → range-fetch the tail
first to get the index (Jellyfin serves whatever's stored — don't assume faststart).
This is the guest-side demux job; the host only decodes.

‼️ HLS is the WRONG tool here: on Jellyfin, HLS (`master.m3u8`/segments) means the
server transcodes/segments — i.e. the server does the decode, which DEFEATS the
DirectPlay proof. HLS exists for adaptive-bitrate / transcode / live, none of which a
static VOD file needs. (The legitimate adaptive/DASH case is Part B YouTube, where the
"two streams" are still each range-fetched and muxed guest-side.)

### Jellyfin milestones
- A1: auth + browse + resolve a DirectPlay MP4/H.264 URL from the guest.
- A2: range-fetch + guest demux + host GStreamer decode → plays on-screen, A/V synced.
- A3: seek (Range re-request + decoder `flush`/`reset`) works.
- A4: an HEVC and a VP9/AV1 library item to exercise the other GStreamer lanes.

---

## Part B — open adaptive streaming (DASH/HLS), NOT YouTube (do second)

**Decision (2026-08-01): reject YouTube as the finalizer.** Prove the adaptive
engine on OPEN protocols (DASH/HLS) — no Google anti-bot fortress, no server to
run, genuinely standalone. YouTube is PARKED (below).

### Why not YouTube (parked, not chosen)
YouTube has no open stream API and gates streams behind an anti-abuse moat —
**PoToken/BotGuard + DroidGuard + SABR**. Getting stream URLs means running
Google's obfuscated, *rotating* attestation program (BotGuard = JS web VM;
DroidGuard = native MBA-obfuscated VM) in a genuine-enough environment. For the
software (web) path it's **not a crypto wall** — it's a perpetual **maintenance
treadmill** (Google rotates; you re-reverse forever). And a wandr `--no-art` guest
can't use the LineageOS-style *genuine Play Services* path (no ART → no Play
Services → no genuine DroidGuard; rooted also fails hardware attestation). So
YouTube is viable only **server-assisted** (Invidious + `invidious-companion`, which
runs BotGuard server-side) on a residential IP with hotfix upkeep — a moving target,
not a finalizer. **→ PARKED: revisit only if you specifically want the demo and will
run the server.** Rejected in-guest extractors (`rusty_ytdl`/`rustube` = wasm-bindgen
browser, stale; `yt-dlp` crate = Python subprocess; Data-API-v3 = metadata only)
still apply. Full attestation reasoning: this session's YouTube deep-dive.

### The open adaptive path — everything reuses the shipped engine
Goal: separate video-only + audio-only streams, **segmented + byte-range**,
guest-demuxed, host-decoded, `wasi:audio`-synced, with **bitrate switching**. DASH
and HLS both converge on **CMAF / fragmented-MP4 (fMP4)** segments → one container
path covers both.

| Piece | Component | Status |
|---|---|---|
| HTTPS + byte-range | `wandr-reqwest` (wasi:tls) | ✅ shipped |
| DASH manifest (`.mpd`) | `dash-mpd` 0.20.4 | new (small) |
| HLS manifest (`.m3u8`) | `m3u8-rs` 6.0.1 | new (small) |
| **fMP4/CMAF segment demux** | **`oxideav-mp4` `=0.0.9` — streaming CMAF demuxer, EMPIRICALLY PROVEN on Tears-of-Steel (`mp4` 0.14 `read_sample` is broken for fragments; see note)** | ✅ verified (probe), wire `Demux::Fmp4` |
| WebM-DASH segment demux (VP9/Opus) | `matroska-demuxer` (vendored+patched) | ✅ shipped |
| Audio decode (AAC/Opus) | `symphonia` / `ropus` | ✅ shipped |
| Video decode + `present(at-ns)` | host `wandr:video` | ✅ shipped |
| A/V sync | `wasi:audio` | ✅ shipped |

**fMP4 CMAF demux — CORRECTED (2026-08-01, verified by reading `mp4` 0.14 source;
the earlier "PROVEN" claim was DOC-ONLY — `repros/fmp4-probe` is empty, commit
`bd892f17` changed only docs):** the `mp4` crate 0.14 *parses* the fragment box
tree — `moofs: Vec<MoofBox>` with `traf`/`trun`/`tfhd`/`tfdt`, `is_fragmented()`,
and codec config from the init segment's `moov` `stsd` all work. **BUT its
`read_sample()` cannot read fragmented samples**: `track.rs::sample_offset()` for
the `!trafs.is_empty()` path returns a *constant* `tfhd.base_data_offset.unwrap_or(0)`
per fragment — it ignores the sample index, never adds `trun.data_offset` or the
accumulated prior-sample sizes, and yields `0` when `tfhd` omits `base_data_offset`
(the normal CMAF "default-base-is-moof" case). So `read_sample` seeks to the same
wrong byte for every sample and returns garbage. **⇒ we DO need a small fragmented
sample reader.** Plan: use `mp4::read_header` (or a tiny box walker) to get the
init-segment codec config + the parsed `trun`/`tfhd`/`tfdt` per fragment, then
compute sample byte-range (`moof_start + trun.data_offset + Σsizes`) and PTS
(`tfdt.base_media_decode_time + Σdurations`, `+ trun.sample_cts` for composition)
OURSELVES — reusing the crate's box parsing, replacing only the offset math. This is
the `Demux::Fmp4` variant. New code = manifest parse + segment fetch + this reader.

### Milestones
- **B1 — raw DASH (CMAF) test stream** ✅ **DONE (2026-08-02, user-confirmed on
  desktop WSLg).** `wandr.dash` (new app) plays Tears of Steel on-screen with
  VIDEO + AUDIO, A/V synced. Chain: `dash-mpd` parse → pick 1 video + 1 audio
  Representation → resolve `$RepresentationID$`/`$Time$` SegmentTimeline → download
  a bounded prefix (`SEG_LIMIT`) → **`oxideav-mp4` CMAF demux** (NOT `mp4`
  `read_sample` — that's broken for fragments) → host gstreamer H.264 decode →
  paced `present(at-ns)` + `symphonia` AAC → `wasi:audio`. Runs on the EXTRACTED
  `wandr-media-engine` crate (shared with jellyfin). Two bugs fixed live: a pump
  pacing race (fast tiny rep raced ~2.5× — added a time-based submit clock-gate,
  see `[[reference_engine_present_pacing_fast_decode]]`) and stale-player audio
  contention. NOW STREAMING (2026-08-02): per-segment demux (`SegStream`) replaces
  download-then-play — ~1 s startup, bounded memory, fetch-on-demand, seek =
  segment-jump; handles `$Time$` AND `$Number$` addressing; default stream = DASH-IF
  Big Buck Bunny. Remaining polish: higher video rep (quality), bounded-prefetch
  tuning. **Zero infrastructure — Part B's technical goal is proven.**
- **B2 — adaptive bitrate switch.** ✅ **DONE (2026-08-02, user-confirmed).** Mid-stream
  video-rep switch: `wandr.dash` resolves ALL video reps (sorted by bandwidth), `b`
  cycles them; the engine's `switch_video_rep` re-opens the decoder with the new rep's
  config (resolution/SPS change ⇒ genuine re-init), swaps the video `SegStream`, and
  re-syncs to the current position (segment-aligned/keyframe). Audio is only briefly
  re-synced (the master clock keeps running). MANUAL trigger proves the mechanism;
  automatic ABR (bandwidth/buffer-driven rep selection) is the follow-up.
- **B3 — HLS (CMAF) test stream.** `m3u8-rs` master+media playlists → same fMP4 demux
  → proves the second protocol with one container path. (Skip TS-HLS: needs a separate
  `mpeg2ts` demuxer; CMAF-HLS avoids it.)
- **B4 (optional) — PeerTube real-client.** Federated, open API, serves HLS — the
  "real streaming service" story without a fortress: browse a PeerTube instance → play
  its HLS. Same engine.

### Verified test streams (checked live 2026-08-01 — clear/no-DRM, CMAF/fMP4, H.264+AAC)
All HTTPS (wandr-reqwest/wasi:tls, trusted CDN roots), HTTP-Range-capable, multi-bitrate,
separate A/V — exactly the adaptive path. Codecs avc1 (host `wandr:video`) + mp4a
(`symphonia`), already shipped.
- **⭐ Unified Streaming — Tears of Steel** (one asset serves BOTH DASH + HLS → covers B1 & B3):
  - DASH: `https://demo.unified-streaming.com/k8s/features/stable/video/tears-of-steel/tears-of-steel.ism/.mpd`
  - HLS:  `https://demo.unified-streaming.com/k8s/features/stable/video/tears-of-steel/tears-of-steel.ism/.m3u8`
- **DASH-IF Big Buck Bunny** (11 Representations → ideal for B2 bitrate switching):
  `https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd`
- **Axinom CMAF clear 1080p H.264** (11 reps): `https://media.axprod.net/TestVectors/Cmaf/clear_1080p_h264/manifest.mpd`
- Catalogs for more: DASH-IF reference player list, Unified Streaming demos, Axinom test vectors.

---

## Done when
1. A Jellyfin DirectPlay library item plays on-screen through the guest client + host
   GStreamer decoder, A/V in sync, seek works (A1–A4).
2. An open **DASH (CMAF)** test stream plays on-screen — separate video+audio, segmented,
   A/V-synced, with a mid-stream bitrate switch (B1–B2); **HLS** proven with the same
   fMP4 demuxer (B3). No infrastructure, no Google. (YouTube parked — server-assisted only.)
3. The upstream `wandr:video` proposal is drafted with these two as the "real consumers"
   that justify the playback contract (opaque `decoded-frame` + `present(at-ns)` +
   `flush`/`reset` + `timestamp-us`). → closes the last of task 117 M2.

## Explicitly NOT doing
- **Server-side transcoding as the path** — must be DirectPlay/`static=true`, or the proof
  is meaningless (the server would be doing the decode).
- **yt-dlp subprocess / any native-binary extractor** — incompatible with the wasip2 sandbox.
- **Host-side demux or extraction** — stays guest-side (`wasi-media-source`).
- A full Jellyfin/YouTube UI — this is a playback PROOF, not a product client. Minimal
  browse + play is enough to validate the pipeline and feed the proposal.

## Starting points
- Engine already shipped: `apps/user/wandr.video.player`, host
  `runtime/wandr-host/crates/wandr-video` (GStreamer decode) + `src/video.rs` present path.
- Guest HTTP: `wandr-reqwest` (arbitrary headers, HTTPS range — proven for Signal).
- Guest audio decode: Symphonia (already in `wandr.audio.player`).
- Contract shape + what exists: task 117 M2 "What already exists (do NOT rebuild)" table.
- Jellyfin: <https://api.jellyfin.org/openapi>; DeepWiki playback-lifecycle overview
  (<https://deepwiki.com/jellyfin/jellyfin/3-media-streaming>).
