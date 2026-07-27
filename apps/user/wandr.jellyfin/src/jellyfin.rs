//! Jellyfin REST client — hand-rolled over the in-tree `wandr-reqwest`
//! (`wasi:tls`), because the published SDKs are all tokio+reqwest-native and
//! won't run in a wasip2 guest (task 119, the crate decision). The surface we
//! need is tiny — Quick Connect auth, browse, PlaybackInfo/DirectPlay, and a
//! byte-range GET — so `serde_json::Value` against the OpenAPI shapes is enough;
//! a typed SDK would be papering over nothing.
//!
//! Every call here maps 1:1 to a request that was validated with curl against a
//! live Jellyfin 10.11.5 before this file existed (task 119 Part A validation).

use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};
use url::Url;

/// Demo target, used ONLY when `/state/jellyfin/server` does not override it.
/// Not a hardcoded policy value: it is one named, overridable default (write the
/// state file to point elsewhere), the single source of truth for "which server
/// when the user hasn't said". `session.json` records the real server once paired.
pub const DEFAULT_SERVER: &str = "https://movies.zaharievi.eu";

/// Persisted session — the whole of `/state/jellyfin/session.json`. Jellyfin
/// tokens are long-lived (no expiry unless revoked), so this is store-once; on a
/// 401 the engine clears it and re-pairs.
#[derive(Clone, Debug)]
pub struct Session {
    pub server_url: String,
    pub user_id: String,
    /// Stable per install — part of the `MediaBrowser` auth header, generated
    /// once at first pairing and kept, so the server sees one device.
    pub device_id: String,
    pub access_token: String,
}

impl Session {
    pub fn from_json(v: &Value) -> Option<Session> {
        Some(Session {
            server_url: v.get("server_url")?.as_str()?.to_string(),
            user_id: v.get("user_id")?.as_str()?.to_string(),
            device_id: v.get("device_id")?.as_str()?.to_string(),
            access_token: v.get("access_token")?.as_str()?.to_string(),
        })
    }
    pub fn to_json(&self) -> Value {
        json!({
            "server_url": self.server_url,
            "user_id": self.user_id,
            "device_id": self.device_id,
            "access_token": self.access_token,
        })
    }
}

/// The `Authorization: MediaBrowser …` header every call carries. Before pairing
/// there is no token (Quick Connect Initiate still needs the DeviceId); after,
/// the token authenticates the session.
pub fn auth_header(device_id: &str, token: Option<&str>) -> String {
    let mut h = format!(
        "MediaBrowser Client=\"wandr\", Device=\"wandr\", DeviceId=\"{device_id}\", Version=\"0.1.0\""
    );
    if let Some(t) = token {
        h.push_str(&format!(", Token=\"{t}\""));
    }
    h
}

fn url(base: &str, path_and_query: &str) -> Result<Url, String> {
    Url::parse(&format!("{}{}", base.trim_end_matches('/'), path_and_query))
        .map_err(|e| format!("bad url: {e}"))
}

// ---- Quick Connect ---------------------------------------------------------

/// A pending Quick Connect pairing: the human-facing 6-digit `code` to type into
/// the Jellyfin web UI, and the `secret` we poll/exchange with.
#[derive(Clone, Debug)]
pub struct QuickConnect {
    pub secret: String,
    pub code: String,
}

pub async fn qc_enabled(client: &Client, server: &str) -> bool {
    match client.get(match url(server, "/QuickConnect/Enabled") {
        Ok(u) => u,
        Err(_) => return false,
    }).send().await {
        Ok(r) => r.text().await.map(|t| t.trim() == "true").unwrap_or(false),
        Err(_) => false,
    }
}

pub async fn qc_initiate(client: &Client, server: &str, device_id: &str) -> Result<QuickConnect, String> {
    let v: Value = client
        .get(url(server, "/QuickConnect/Initiate")?)
        .header("Authorization", auth_header(device_id, None))
        .send().await.map_err(|e| format!("qc initiate: {e}"))?
        .json().await.map_err(|e| format!("qc initiate json: {e}"))?;
    Ok(QuickConnect {
        secret: v.get("Secret").and_then(|s| s.as_str()).ok_or("no Secret")?.to_string(),
        code: v.get("Code").and_then(|s| s.as_str()).ok_or("no Code")?.to_string(),
    })
}

/// Poll once → is the code approved yet?
pub async fn qc_poll(client: &Client, server: &str, device_id: &str, secret: &str) -> Result<bool, String> {
    let v: Value = client
        .get(url(server, &format!("/QuickConnect/Connect?Secret={secret}"))?)
        .header("Authorization", auth_header(device_id, None))
        .send().await.map_err(|e| format!("qc poll: {e}"))?
        .json().await.map_err(|e| format!("qc poll json: {e}"))?;
    Ok(v.get("Authenticated").and_then(|b| b.as_bool()).unwrap_or(false))
}

/// Exchange an approved secret for the access token + user id.
pub async fn qc_exchange(
    client: &Client, server: &str, device_id: &str, secret: &str,
) -> Result<(String, String), String> {
    let v: Value = client
        .request(Method::POST, url(server, "/Users/AuthenticateWithQuickConnect")?)
        .header("Authorization", auth_header(device_id, None))
        .json(&json!({ "Secret": secret }))
        .send().await.map_err(|e| format!("qc exchange: {e}"))?
        .json().await.map_err(|e| format!("qc exchange json: {e}"))?;
    let token = v.get("AccessToken").and_then(|s| s.as_str()).ok_or("no AccessToken")?.to_string();
    let user_id = v.get("User").and_then(|u| u.get("Id")).and_then(|s| s.as_str())
        .ok_or("no User.Id")?.to_string();
    Ok((token, user_id))
}

// ---- Browse ----------------------------------------------------------------

/// One library item, flattened to what the player needs.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub media_source_id: String,
    pub container: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub size: u64,
    /// Total runtime in Jellyfin ticks (100 ns) — /10_000_000 = seconds.
    pub run_time_ticks: i64,
    /// Primary-image content hash = built-in cache-busting key for the thumb cache.
    pub image_tag: Option<String>,
}

impl Item {
    pub fn duration_s(&self) -> f64 { self.run_time_ticks as f64 / 10_000_000.0 }
}

fn first_stream_codec<'a>(ms: &'a Value, kind: &str) -> &'a str {
    ms.get("MediaStreams").and_then(|s| s.as_array()).and_then(|arr| {
        arr.iter().find(|s| s.get("Type").and_then(|t| t.as_str()) == Some(kind))
            .and_then(|s| s.get("Codec")).and_then(|c| c.as_str())
    }).unwrap_or("?")
}

/// Browse Movies (recursive) with MediaSources so we can filter by codec.
pub async fn browse_movies(client: &Client, s: &Session, limit: u32) -> Result<Vec<Item>, String> {
    let path = format!(
        "/Items?IncludeItemTypes=Movie&Recursive=true&Limit={limit}\
         &Fields=MediaSources&SortBy=SortName&userId={}",
        s.user_id
    );
    let v: Value = client
        .get(url(&s.server_url, &path)?)
        .header("Authorization", auth_header(&s.device_id, Some(&s.access_token)))
        .send().await.map_err(|e| format!("browse: {e}"))?
        .json().await.map_err(|e| format!("browse json: {e}"))?;
    let mut out = Vec::new();
    for it in v.get("Items").and_then(|i| i.as_array()).cloned().unwrap_or_default() {
        let ms = it.get("MediaSources").and_then(|m| m.as_array())
            .and_then(|a| a.first()).cloned().unwrap_or(json!({}));
        out.push(Item {
            id: it.get("Id").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            name: it.get("Name").and_then(|s| s.as_str()).unwrap_or("?").to_string(),
            media_source_id: ms.get("Id").and_then(|s| s.as_str())
                .or_else(|| it.get("Id").and_then(|s| s.as_str())).unwrap_or("").to_string(),
            container: ms.get("Container").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            video_codec: first_stream_codec(&ms, "Video").to_string(),
            audio_codec: first_stream_codec(&ms, "Audio").to_string(),
            size: ms.get("Size").and_then(|s| s.as_u64()).unwrap_or(0),
            run_time_ticks: it.get("RunTimeTicks").and_then(|s| s.as_i64()).unwrap_or(0),
            image_tag: it.get("ImageTags").and_then(|t| t.get("Primary"))
                .and_then(|s| s.as_str()).map(|s| s.to_string()),
        });
    }
    Ok(out)
}

// ---- DirectPlay negotiation ------------------------------------------------

/// What PlaybackInfo resolved to. The proof lives in `direct_play == true` +
/// `transcode_url == None`: the server ships original bytes, our pipeline decodes.
#[derive(Clone, Debug)]
pub struct Playback {
    pub media_source_id: String,
    pub container: String,
    pub play_session_id: String,
    pub direct_play: bool,
    pub transcode_url: Option<String>,
}

/// The DeviceProfile we advertise: exactly the containers/codecs the shipped
/// pipeline decodes, and EMPTY TranscodingProfiles so the server has no choice
/// but DirectPlay (or fail) — never a server-side transcode (task 119 ‼️ rule).
fn device_profile(user_id: &str) -> Value {
    json!({
        "UserId": user_id,
        "MaxStreamingBitrate": 400_000_000u64,
        "DeviceProfile": {
            "Name": "wandr",
            "MaxStreamingBitrate": 400_000_000u64,
            "DirectPlayProfiles": [
                { "Container": "mp4,m4v,mov", "Type": "Video",
                  "VideoCodec": "h264,hevc,vp9,av1", "AudioCodec": "aac,mp3,opus,flac" },
                { "Container": "mkv,webm", "Type": "Video",
                  "VideoCodec": "h264,hevc,vp9,av1", "AudioCodec": "aac,mp3,opus,flac,ac3,eac3" }
            ],
            "TranscodingProfiles": [],
            "CodecProfiles": [],
            "SubtitleProfiles": [ { "Format": "srt", "Method": "External" } ]
        }
    })
}

pub async fn playback_info(client: &Client, s: &Session, item_id: &str) -> Result<Playback, String> {
    let v: Value = client
        .request(Method::POST, url(&s.server_url, &format!("/Items/{item_id}/PlaybackInfo"))?)
        .header("Authorization", auth_header(&s.device_id, Some(&s.access_token)))
        .json(&device_profile(&s.user_id))
        .send().await.map_err(|e| format!("playbackinfo: {e}"))?
        .json().await.map_err(|e| format!("playbackinfo json: {e}"))?;
    let play_session_id = v.get("PlaySessionId").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let ms = v.get("MediaSources").and_then(|m| m.as_array())
        .and_then(|a| a.first()).cloned().ok_or("PlaybackInfo: no MediaSources")?;
    Ok(Playback {
        media_source_id: ms.get("Id").and_then(|s| s.as_str()).unwrap_or(item_id).to_string(),
        container: ms.get("Container").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        play_session_id,
        direct_play: ms.get("SupportsDirectPlay").and_then(|b| b.as_bool()).unwrap_or(false),
        transcode_url: ms.get("TranscodingUrl").and_then(|s| s.as_str()).map(|s| s.to_string()),
    })
}

/// The raw byte-stream URL. `static=true` = original file, no transcode, honors
/// HTTP Range. `api_key` carries the token as a query param (the stream GET is
/// range-fetched many times; a query key avoids re-sending the header each time,
/// and Jellyfin accepts it).
pub fn stream_url(s: &Session, item_id: &str, media_source_id: &str, container: &str, play_session_id: &str) -> String {
    let cont = if container.is_empty() { "mp4" } else { container };
    format!(
        "{}/Videos/{item_id}/stream.{cont}?static=true&mediaSourceId={media_source_id}\
         &api_key={}&playSessionId={play_session_id}",
        s.server_url.trim_end_matches('/'), s.access_token
    )
}

/// Poster image URL (WebP, capped width) for the thumbnail cache. Server-side
/// IMAGE resize is fine — it is poster scaling, not media transcoding.
pub fn image_url(s: &Session, item_id: &str, image_tag: &str, max_width: u32) -> String {
    format!(
        "{}/Items/{item_id}/Images/Primary?tag={image_tag}&maxWidth={max_width}&format=Webp",
        s.server_url.trim_end_matches('/')
    )
}

// ---- Byte-range fetch ------------------------------------------------------

/// A 206 partial response: the bytes plus the total resource length parsed from
/// `Content-Range: bytes A-B/TOTAL`.
pub struct Range {
    pub bytes: Vec<u8>,
    pub total_len: u64,
}

/// GET `url` with `Range: bytes=start-end` (end inclusive; None = open-ended to
/// EOF). Returns the partial bytes + the total length. wandr-reqwest buffers the
/// whole 206 chunk, so keep ranges bounded (we do the chunking in the engine).
pub async fn fetch_range(client: &Client, media_url: &str, start: u64, end: Option<u64>) -> Result<Range, String> {
    let range = match end {
        Some(e) => format!("bytes={start}-{e}"),
        None => format!("bytes={start}-"),
    };
    let u = Url::parse(media_url).map_err(|e| format!("bad media url: {e}"))?;
    let resp = client.get(u)
        .header("Range", range)
        .send().await.map_err(|e| format!("range fetch: {e}"))?;
    let status = resp.status();
    if status != StatusCode::PARTIAL_CONTENT && status != StatusCode::OK {
        return Err(format!("range fetch: unexpected status {status}"));
    }
    let total_len = resp.headers().get("content-range")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.rsplit('/').next())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .or_else(|| resp.headers().get("content-length").and_then(|h| h.to_str().ok()).and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    let bytes = resp.bytes().await.map_err(|e| format!("range body: {e}"))?.to_vec();
    Ok(Range { bytes, total_len })
}
