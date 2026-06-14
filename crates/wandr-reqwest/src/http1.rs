//! Minimal HTTP/1.1 client over `TlsStream`. Signal uses `.http1_only()` and most
//! traffic is on the websocket, so this only needs request/response with
//! content-length or chunked bodies. One TLS connection per request
//! (`Connection: close`) — simple and correct; connection reuse is a v2 perf TODO.

use http::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::tls::TlsStream;

pub struct RawResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

/// Headers the transport sets itself; drop any caller-supplied duplicates.
fn is_reserved(name: &str) -> bool {
    name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("content-length")
        || name.eq_ignore_ascii_case("connection")
}

pub async fn request(
    method: &str,
    url: &Url,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<RawResponse, String> {
    let host = url.host_str().ok_or("url has no host")?;
    let port = url.port_or_known_default().unwrap_or(443);
    let stream = TlsStream::connect(host, port).await?;

    let body = body.unwrap_or(&[]);

    let mut target = url.path().to_string();
    if let Some(q) = url.query() {
        target.push('?');
        target.push_str(q);
    }
    let host_hdr = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };

    let mut req = format!("{method} {target} HTTP/1.1\r\n");
    req.push_str(&format!("Host: {host_hdr}\r\n"));
    for (k, v) in headers {
        if !is_reserved(k) {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    req.push_str("Connection: close\r\n\r\n");

    stream.write_all(req.as_bytes()).await?;
    if !body.is_empty() {
        stream.write_all(body).await?;
    }

    let head = stream.read_until(b"\r\n\r\n").await?;
    let (status, headers) = parse_head(&head)?;
    let body = read_body(&stream, &headers).await?;
    Ok(RawResponse { status, headers, body })
}

fn parse_head(head: &[u8]) -> Result<(u16, HeaderMap), String> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().ok_or("empty response")?;
    // "HTTP/1.1 200 OK"
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| format!("bad status line: {status_line}"))?;

    let mut headers = HeaderMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(k.trim().as_bytes()),
                HeaderValue::from_str(v.trim()),
            ) {
                headers.append(name, val);
            }
        }
    }
    Ok((status, headers))
}

async fn read_body(
    stream: &TlsStream,
    headers: &HeaderMap,
) -> Result<Vec<u8>, String> {
    let chunked = headers
        .get(http::header::TRANSFER_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("chunked"))
        .unwrap_or(false);

    if chunked {
        return read_chunked(stream).await;
    }

    if let Some(len) = headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        return stream.read_exact(len).await;
    }

    // No framing info: we sent Connection: close, so read to EOF.
    stream.read_to_end().await
}

async fn read_chunked(stream: &TlsStream) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let line = stream.read_until(b"\r\n").await?;
        let size_str = String::from_utf8_lossy(&line);
        let size_str = size_str.trim();
        // chunk-ext after ';' is ignored
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| format!("bad chunk size: {size_str}"))?;
        if size == 0 {
            // consume trailing CRLF (and any trailers up to the blank line)
            let _ = stream.read_until(b"\r\n").await;
            break;
        }
        let chunk = stream.read_exact(size).await?;
        body.extend_from_slice(&chunk);
        // each chunk is followed by CRLF
        let _ = stream.read_exact(2).await?;
    }
    Ok(body)
}
