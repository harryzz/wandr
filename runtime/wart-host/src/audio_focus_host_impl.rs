//! `war:audio-focus/focus` host impl (wart-arbiter-audio, M2).
//!
//! A guest calls `request(kind)` / `abandon()`; the host forwards to the
//! arbiter's `audio-focus-request <pid> <kind>` / `audio-focus-abandon <pid>`
//! socket commands. Unlike the fire-and-forget alarm/notify forwards, `request`
//! reads the arbiter's reply (`OK granted …` / `OK delayed …` / `ERR …`) and
//! maps it to a `focus-result`. The arbiter, on an owner change, delivers
//! `on-focus-changed <change>` back to this host's control socket → the
//! standalone loop calls the guest's `focus-handler.on-focus-changed` export.
//!
//! Owner identity: the host self-reports its pid (`std::process::id()` — the
//! zygote-forked child the arbiter registered), resolved to the app-id arbiter-
//! side. Mirrors `alarm_host_impl` / `notify_host_impl`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use crate::audio_focus_host_bindings::war::audio_focus::focus::{
    FocusKind, FocusResult, Host,
};

const ARBITER_SOCK_PATH: &str = "/data/local/tmp/wart-arbiter.sock";

impl FocusKind {
    fn as_wire(self) -> &'static str {
        match self {
            FocusKind::Gain                 => "gain",
            FocusKind::GainTransient        => "gain-transient",
            FocusKind::GainTransientMayDuck => "gain-transient-may-duck",
        }
    }
}

/// Fire-and-forget one line to the arbiter (abandon).
fn send_oneshot(line: &str) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(ARBITER_SOCK_PATH)?;
    stream.write_all(line.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

/// Send one line and read the arbiter's single-line reply (`OK …` / `ERR …`).
fn send_and_read(line: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(ARBITER_SOCK_PATH)?;
    stream.write_all(line.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

impl Host for crate::HostState {
    fn request(&mut self, kind: FocusKind) -> FocusResult {
        let pid = std::process::id();
        let line = format!("audio-focus-request {pid} {}\n", kind.as_wire());
        match send_and_read(&line) {
            Ok(reply) => {
                let r = reply.trim();
                log::info!("audio-focus-host: request kind={} → {r}", kind.as_wire());
                // Reply is `OK granted …` / `OK delayed …` / `ERR …`.
                if r.starts_with("OK granted") {
                    FocusResult::Granted
                } else if r.starts_with("OK delayed") {
                    FocusResult::Delayed
                } else {
                    FocusResult::Failed
                }
            }
            Err(e) => {
                log::warn!("audio-focus-host: request forward failed: {e:#} (arbiter down?)");
                FocusResult::Failed
            }
        }
    }

    fn abandon(&mut self) {
        let pid = std::process::id();
        if let Err(e) = send_oneshot(&format!("audio-focus-abandon {pid}\n")) {
            log::warn!("audio-focus-host: abandon forward failed: {e:#}");
        }
    }
}
