//! `war:notify/notifier` host impl (Signal bg-receipt M3).
//!
//! A guest calls `post` / `cancel`; the host forwards to the arbiter's
//! `notify-post` / `notify-cancel` socket commands (one-shot, mirroring
//! `alarm_host_impl`). The arbiter owns the active list, surfaces it in the
//! status bar, and on a tap delivers `notification-clicked <id>` to this host's
//! control socket → the standalone loop calls the guest's `on-notification-click`
//! export.
//!
//! Owner identity = the host's own pid (`std::process::id()` — the zygote-forked
//! child the arbiter registered), resolved arbiter-side to the app-id. `title`/
//! `body` are percent-encoded so they survive the whitespace-delimited control
//! line (the arbiter decodes them; mirror of `wart_arbiter_notify::pct_decode`).

use std::io::Write;
use std::os::unix::net::UnixStream;

use crate::notify_host_bindings::war::notify::notifier::Host;

const ARBITER_SOCK_PATH: &str = "/data/local/tmp/wart-arbiter.sock";

fn send_oneshot(line: &str) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(ARBITER_SOCK_PATH)?;
    stream.write_all(line.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

/// Conservative percent-encoding: keep `A-Za-z0-9-._`, escape the rest as `%XX`
/// (UTF-8 bytes). Mirror of the arbiter's decoder.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            out.push(b as char);
        } else {
            out.push('%');
            let hex = |n: u8| (if n < 10 { b'0' + n } else { b'A' + (n - 10) }) as char;
            out.push(hex(b >> 4));
            out.push(hex(b & 0xf));
        }
    }
    out
}

impl Host for crate::HostState {
    fn post(&mut self, id: u64, title: String, body: String) {
        let pid = std::process::id();
        let line = format!(
            "notify-post {pid} {id} {} {}\n",
            pct_encode(&title),
            pct_encode(&body),
        );
        match send_oneshot(&line) {
            Ok(()) => log::info!("notify-host: posted id={id} title={title:?}"),
            Err(e) => log::warn!("notify-host: post id={id} forward failed: {e:#} (arbiter down?)"),
        }
    }

    fn cancel(&mut self, id: u64) {
        let pid = std::process::id();
        if let Err(e) = send_oneshot(&format!("notify-cancel {pid} {id}\n")) {
            log::warn!("notify-host: cancel id={id} forward failed: {e:#}");
        }
    }
}
