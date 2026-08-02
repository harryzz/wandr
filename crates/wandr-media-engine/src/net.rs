//! Generic HTTPS byte-range media fetch over wandr-reqwest (wasi:tls).
//!
//! Extracted verbatim from wandr.jellyfin's `jellyfin.rs` — the container demux
//! (`httprange`, `open_mp4_sync`, `open_mkv_sync`) pulls `moov`/EBML-header and
//! `mdat`/Cluster windows through this. It is server-agnostic: any HTTP server
//! that honours `Range: bytes=A-B` works.

use reqwest::{Client, StatusCode};
use url::Url;

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
