# Task 119 — media-player real-client proof (Jellyfin + YouTube)

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
2. **YouTube (via Invidious)** — the messy real-world adaptive/DASH case. Do SECOND.

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

### Rust crates (evaluate; may prefer hand-rolling the ~4 endpoints)
- [`jellyfin-sdk`](https://crates.io/crates/jellyfin-sdk) — async, reqwest-based.
- [`jellyfin_api`](https://docs.rs/jellyfin_api/) — auto-generated from the OpenAPI spec (large surface).
- [`jellyfin-sdk-rust`](https://docs.rs/jellyfin-sdk-rust) — async, reqwest-based.

⚠️ All are `reqwest`-based. Under wasip2 we go through `wandr-reqwest` (`wasi:tls`), so a
crate that hard-pins reqwest's native-TLS/tokio features may not build for the guest. The
Jellyfin surface we actually need is tiny (auth + list + PlaybackInfo + stream URL), so
**hand-rolling those calls with the in-tree HTTP client is likely cleaner** than adopting a
generated SDK. Decide once the guest reqwest constraint is checked.

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

## Part B — YouTube via Invidious (do second)

The extraction landscape and why Invidious wins for a wasip2 guest:

| Option | Verdict for wandr |
|---|---|
| [`invidious`](https://docs.rs/invidious/) crate (0.7.8, active) | ⭐ **Chosen.** REST-JSON to an Invidious instance; **no YouTube token, no cipher work in-guest** (the server does the signature/n-sig). Returns `formatStreams`/`adaptiveFormats` = direct stream URLs + itag/mime/container. Guest does HTTPS-JSON (already works). |
| [`rusty_ytdl`](https://github.com/Mithronn/rusty_ytdl) / [`rustube`](https://github.com/DzenanJupic/rustube) | ❌ Pure-Rust in-guest extractors but **go stale constantly** (YouTube breaks them); their "wasm" is wasm-bindgen/**browser**, not wasip2 — need a JS host for the cipher. Won't run in a wandr guest as-is. |
| [`yt-dlp`](https://crates.io/crates/yt-dlp) crate | ❌ Most reliable extractor, but **shells out to the yt-dlp binary (Python)** — no subprocess in a wasip2 sandbox; host-side only, which breaks the guest-side design. |
| [`youtubei-rs`](https://crates.io/crates/youtubei-rs) | ⚠️ Wraps YouTube's internal InnerTube API; more official-ish but you still own the cipher problem for stream URLs. |
| `google-youtube3` (Data API v3) | ❌ Metadata only — **no downloadable stream URLs**. Useless for playback. |

### Approach
- Use the `invidious` crate against a **self-hosted Invidious instance** (public instances
  are rate-limited/flaky and YouTube periodically breaks Invidious — self-hosting keeps the
  demo stable; the *guest code* stays simple either way since the fragility is off-sandbox).
- B1: `formatStreams` (muxed 360p MP4, video+audio in one container) → simplest light-up,
  reuses the Jellyfin A2 path almost verbatim.
- B2: `adaptiveFormats` (separate video itag + Opus/AAC audio itag) → exercises the REAL
  DASH-style A/V-sync path (two range-fetched streams, guest muxes/syncs, host decodes video,
  Symphonia decodes audio, both against `wasi:audio position()`).

---

## Done when
1. A Jellyfin DirectPlay library item plays on-screen through the guest client + host
   GStreamer decoder, A/V in sync, seek works (A1–A4).
2. A YouTube video plays via Invidious — muxed first, then adaptive video+audio (B1–B2).
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
