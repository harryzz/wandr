---
name: reference_bluesky_atproto_wandr
description: "Bluesky/atproto client on wandr = clean fit via Atrium (sugyan/atrium). atrium-xrpc's HttpClient trait is transport-agnostic (http::Request→http::Response); atrium-api + atrium-xrpc are reqwest/TLS-free → plug our wandr-reqwest (wasi:tls) via a ~40-line WandrXrpcClient, same pattern as Signal/jellyfin. SPIKE 2026-08-01: atrium-api+atrium-xrpc PROVEN wasip2; bsky-sdk fails (hardcoded Send+Sync, single-thread wasm) → skip it, hand-roll sessions."
metadata:
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-08-01T14:03:58.059Z
---

Researched 2026-08-01 (source-grounded, "little research"). **A Bluesky client is
a clean wandr guest fit — arguably the cleanest third-party fit evaluated,**
because Atrium was explicitly designed transport-agnostic. NOT built yet.

## The library: Atrium (`github.com/sugyan/atrium`, actively maintained)
Versions: atrium-api 0.25.8, atrium-xrpc 0.12.4, atrium-xrpc-client 0.5.15,
bsky-sdk 0.1.24, atrium-oauth 0.1.7 (all 2025-12..2026-03). Workspace also has
atrium-common/crypto/identity/repo + lexicon.

**The plug point = `atrium-xrpc::HttpClient` (transport-agnostic trait):**
```rust
pub trait HttpClient { fn send_http(&self, req: http::Request<Vec<u8>>)
    -> ... http::Response<Vec<u8>> ...; }
pub trait XrpcClient: HttpClient { fn base_uri(&self)->String; fn authorization_token(...); }
```
So you bring ANY HTTP backend. And the core crates are **reqwest/TLS-free**:
- `atrium-xrpc` (traits) — deps just `http` + `serde`. No reqwest/tokio/TLS.
- `atrium-api` (generated lexicon types + XRPC methods) — `http`, `serde`, `chrono`,
  `ipld-core`, `langtag`, `regex`, `thiserror`; **`tokio` OPTIONAL** (only `agent`
  feature → `tokio/sync`). No reqwest, no native TLS.

## wandr fit = same pattern as Signal/jellyfin (verified surfaces)
Implement `HttpClient` over our in-tree **`wandr-reqwest`** (wasi:tls). It already
exposes `Client`/`ClientBuilder`/`RequestBuilder` (`.header()`/`.json()`/`.bytes`)
+ `Response` (`.status()`/`.headers()`) and re-exports `http::{Method,StatusCode,
HeaderMap}` — so the bridge is a **~40-line `WandrXrpcClient`** (http::Request →
wandr-reqwest call → http::Response). **DON'T use `atrium-xrpc-client`** — its wasm
target is reqwest+wasm-bindgen (browser, NOT wasip2) and its default is native-TLS;
the trait lets us bypass it entirely. Stack:
```
atrium-api  (features: bluesky[, agent])       lexicon types + XRPC builders
atrium-xrpc (HttpClient/XrpcClient traits)     pure http+serde
bsky-sdk    (default-features=false, ["rich-text"])  optional: agent + session refresh + facets
  + WandrXrpcClient: HttpClient → wandr-reqwest (wasi:tls)
```
Two bonuses (verified in Cargo.tomls):
- **Size gate-able:** `atrium-api` gates the big generated surface by namespace —
  `default=["agent","bluesky"]`; `bluesky=[namespace-appbsky,namespace-chatbsky]`;
  drop `ozone`/moderation → `.wasm` carries only used lexicons.
- **bsky-sdk accepts our client:** default pulls the native client via
  `default-client`, but `default-features=false` → `BskyAgent` is generic over any
  `XrpcClient`; keep `rich-text` (facet detect, pure regex) + inject our transport.

## Scope
- **Easy (plain XRPC-over-HTTPS):** timeline, posts, likes, follows, profiles,
  notifications, search, auth. Auth = app-password `com.atproto.server.createSession`
  (agent handles JWT refresh). OAuth = `atrium-oauth` (DPoP/ES256 via
  `atrium-crypto`) heavier → defer (Bluesky nudging toward OAuth long-term).
- **Images/video:** blobs over HTTPS/CDN → existing image/video decode.
- **Deferred/harder:** real-time firehose (`com.atproto.sync.subscribeRepos`) =
  WebSocket + DAG-CBOR + CAR/MST (`atrium-repo`) → needs wasi websockets, heavy.
  Poll `getTimeline`/`listNotifications` (or Jetstream JSON-over-WS) first.
- **UI:** any shipped framework (dioxus-canvas/Slint/Compose) — separate choice.

## Confidence + SPIKE RESULT (2026-08-01, `repros/atrium-wasip2-probe`)
Architecture fit = source-verified. **wasip2 build PROVEN:**
- ✅ **`atrium-xrpc` + `atrium-api` (bluesky+agent) COMPILE for `wasm32-wasip2`** —
  the whole dep chain (chrono, ipld-core, regex, langtag, tokio/sync) builds clean.
  The chrono/ipld-core unknown is RESOLVED. The recommended stack (atrium-api +
  atrium-xrpc + our WandrXrpcClient) is de-risked.
- ❌ **`bsky-sdk` does NOT compile on wasip2** — it hardcodes `T: XrpcClient + Send +
  Sync` (`BskyAtpAgentBuilder::new`, `detect_facets`), but on single-threaded wasm the
  async client is `!Send` → E0277. Classic single-thread-wasm snag ([[feedback_wasi_threading]]).
**⇒ Plan: SKIP bsky-sdk.** Build on atrium-api + atrium-xrpc directly and do session
management (`com.atproto.server.createSession`/`refreshSession` + JWT) ourselves in
the client (minimal deps, full control — same as Signal/jellyfin). If bsky-sdk's
niceties (rich-text facets) are ever wanted: vendored patch relaxing `Send+Sync`→
`?Send` behind a wasm cfg (matroska-demuxer-style). Follow-up: confirm atrium-api's
own `AtpAgent` is `!Send`-friendly, or just hand-roll sessions.

Related: [[reference_wandr_wasi_tls_transport]], [[project_signal_client_architecture]]
(same wasi:tls transport pattern), [[reference_jellyfin_container_demux_and_mkv_seek]]
(wandr-reqwest client precedent), [[reference_dioxus_taffy_rust_ui]] / [[reference_slint_wasip2]] (UI).
