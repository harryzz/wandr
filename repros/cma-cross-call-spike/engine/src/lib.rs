//! Engine-like component (task 115 M2a spike). `start` (async-lifted, called by
//! the HOST via `call_async`) spawns a forever ticker on the native CM-async
//! executor — the pattern that replaces `wandr_step_executor::spawn(run()).detach()`.
//! `poll` is a sync drain the UI calls per frame, like the real `poll-events`
//! after `step()` is deleted.
wit_bindgen::generate!({ world: "engine", path: "wit", generate_all });

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use wit_bindgen::rt::async_support::spawn_local as spawn;

static TICKS: AtomicU32 = AtomicU32::new(0);
static FETCH_BG: Mutex<Option<Result<String, String>>> = Mutex::new(None);

struct EngineC;

impl exports::demo::cma::chat::Guest for EngineC {
    async fn start() {
        spawn(async {
            loop {
                // 100 ms tick — models the engine's idle 200 ms / in-call 10 ms cadence.
                crate::wasi::clocks::monotonic_clock::wait_for(100_000_000).await;
                TICKS.fetch_add(1, Ordering::Relaxed);
            }
        });
    }

    fn poll() -> u32 {
        TICKS.load(Ordering::Relaxed)
    }

    /// Gate E: spawn the whole fetch as a background task (the engine's real
    /// shape) — progress must come from pump windows, not this call.
    async fn fetch_bg(host: String) {
        spawn(async move {
            eprintln!("fetch-bg: task started");
            let r = <EngineC as exports::demo::cma::chat::Guest>::fetch(host).await;
            eprintln!("fetch-bg: task finished: {r:?}");
            *FETCH_BG.lock().unwrap() = Some(r);
        });
    }

    fn fetch_bg_result() -> Option<Result<String, String>> {
        FETCH_BG.lock().unwrap().take()
    }

    /// Gate F1: the Signal engine's exact stalling path — WSS upgrade via the
    /// reqwest-websocket shim, then wait for the first server frame.
    async fn ws_probe(host: String) -> Result<String, String> {
        ws_probe_impl(host).await
    }

    /// Step-bisect of the upgrade (see wit).
    async fn ws_steps(host: String) -> Result<String, String> {
        let stream = wandr_reqwest::tls::TlsStream::connect(&host, 443).await?;
        eprintln!("ws-steps: connected");
        let rb = wandr_reqwest::random_bytes(16);
        eprintln!("ws-steps: random ok ({} bytes)", rb.len());
        let req = format!("GET /v1/websocket/provisioning/ HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n");
        stream.write_all(req.as_bytes()).await?;
        eprintln!("ws-steps: request written");
        let head = stream.read_until(b"\r\n\r\n").await?;
        eprintln!("ws-steps: headers read ({} bytes)", head.len());
        Ok(String::from_utf8_lossy(&head).lines().next().unwrap_or("").to_string())
    }

    /// Keep-alive discriminator (see wit).
    async fn ka_probe(host: String) -> Result<String, String> {
        eprintln!("ka-probe: testing bare task::sleep(10ms)");
        wandr_reqwest::task::sleep(std::time::Duration::from_millis(10)).await;
        eprintln!("ka-probe: bare sleep RETURNED");
        let stream = wandr_reqwest::tls::TlsStream::connect(&host, 443).await?;
        eprintln!("ka-probe: connected");
        let req = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n\r\n");
        stream.write_all(req.as_bytes()).await?;
        eprintln!("ka-probe: request written (connection stays open)");
        let head = stream.read_until(b"\r\n\r\n").await?;
        eprintln!("ka-probe: headers read ({} bytes)", head.len());
        Ok(String::from_utf8_lossy(&head).lines().next().unwrap_or("").to_string())
    }

    /// Gate F2: same, spawned (engine shape); result via fetch-bg-result.
    async fn ws_probe_bg(host: String) {
        spawn(async move {
            eprintln!("ws-probe-bg: task started");
            let r = ws_probe_impl(host).await;
            eprintln!("ws-probe-bg: task finished: {r:?}");
            *FETCH_BG.lock().unwrap() = Some(r);
        });
    }

    /// Phase-3 kill-gate: live HTTPS GET over the full wandr-reqwest p3 stack
    /// (Client → http1 → tls_p3, zero step-executor).
    async fn fetch(host: String) -> Result<String, String> {
        let url = url::Url::parse(&format!("https://{host}/")).map_err(|e| format!("url: {e}"))?;
        let client = wandr_reqwest::Client::builder()
            .build()
            .map_err(|e| format!("client: {e:?}"))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("send: {e:?}"))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| format!("text: {e:?}"))?;
        Ok(format!("{} ({} body bytes)", status, body.len()))
    }

    /// Cancellation-safety torture: raw TlsStream, reads repeatedly DROPPED
    /// mid-select (the engine's select_biased! pattern) while the response
    /// streams in. Any lost byte corrupts the header parse / body length.
    async fn fetch_chopped(host: String) -> Result<String, String> {
        use futures::FutureExt;
        let stream = wandr_reqwest::tls::TlsStream::connect(&host, 443).await?;
        let req = format!("GET / HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await?;

        // Read the full response, but drop the pending read on every 1ms tick
        // — the same shape as the engine's `select_biased!{ ws.next(), sleep }`.
        let mut all = Vec::new();
        let mut drops = 0u32;
        loop {
            let mut read = std::pin::pin!(stream.read_to_end().fuse());
            let mut tick = std::pin::pin!(wandr_reqwest::task::sleep(
                std::time::Duration::from_millis(1)
            )
            .fuse());
            futures::select_biased! {
                r = read => { all.extend(r?); break; }
                _ = tick => { drops += 1; }
            }
            // `read` (and its in-flight fill) is dropped here; the reader task
            // must have retained everything already received. Drain it in
            // arrival order before the next read attempt starts.
            all.extend(stream.take_buffered());
            if drops > 20_000 {
                return Err("gave up: response never completed".into());
            }
        }
        let head_end = find(&all, b"\r\n\r\n").ok_or("no header terminator")?;
        let head = String::from_utf8_lossy(&all[..head_end]).to_string();
        let status = head.lines().next().unwrap_or("<none>").to_string();
        // sanity: Content-Length (if present) must match the received body.
        let body_len = all.len() - (head_end + 4);
        if let Some(cl) = head
            .lines()
            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_string()))
        {
            let expect: usize = cl.parse().map_err(|_| "bad content-length")?;
            if body_len != expect {
                return Err(format!(
                    "BYTE LOSS: content-length {expect} but received {body_len} (drops={drops})"
                ));
            }
        }
        all.truncate(head_end);
        Ok(format!("{status} ({body_len} body bytes, {drops} dropped reads)"))
    }
}

async fn ws_probe_impl(host: String) -> Result<String, String> {
    use wandr_reqwest_websocket::RequestBuilderExt;
    let url = url::Url::parse(&format!("wss://{host}/v1/websocket/provisioning/"))
        .map_err(|e| format!("url: {e}"))?;
    let client = wandr_reqwest::Client::builder()
        .build()
        .map_err(|e| format!("client: {e:?}"))?;
    eprintln!("ws-probe: upgrading");
    let resp = client
        .get(url)
        .upgrade()
        .send()
        .await
        .map_err(|e| format!("upgrade: {e:?}"))?;
    let mut ws = resp
        .into_websocket()
        .await
        .map_err(|e| format!("into_websocket: {e:?}"))?;
    eprintln!("ws-probe: upgraded (HTTP 101) — waiting for first server frame");
    match ws.next().await {
        Some(Ok(m)) => {
            let kind = match m {
                wandr_reqwest_websocket::Message::Text(_) => "text",
                wandr_reqwest_websocket::Message::Binary(_) => "binary",
                wandr_reqwest_websocket::Message::Ping(_) => "ping",
                wandr_reqwest_websocket::Message::Pong(_) => "pong",
                wandr_reqwest_websocket::Message::Close { .. } => "close",
            };
            Ok(format!("first server frame: {kind}"))
        }
        Some(Err(e)) => Err(format!("frame error: {e:?}")),
        None => Err("EOF before first frame".into()),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

export!(EngineC);
