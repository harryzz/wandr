//! Engine-like component (task 115 M2a spike). `start` (async-lifted, called by
//! the HOST via `call_async`) spawns a forever ticker on the native CM-async
//! executor — the pattern that replaces `wandr_step_executor::spawn(run()).detach()`.
//! `poll` is a sync drain the UI calls per frame, like the real `poll-events`
//! after `step()` is deleted.
wit_bindgen::generate!({ world: "engine", path: "wit", generate_all });

use std::sync::atomic::{AtomicU32, Ordering};
use wit_bindgen::rt::async_support::spawn;

static TICKS: AtomicU32 = AtomicU32::new(0);

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

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

export!(EngineC);
