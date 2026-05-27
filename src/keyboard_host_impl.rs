//! `my:skiko-gfx/keyboard` WIT impl — task 47 step 3b.
//!
//! Mirror of `ime_host_impl.rs` (step 2) for the OPPOSITE direction:
//! where `ime` is the host trait an EDITOR-bearing guest uses to
//! report focus changes, `keyboard` is what an IME-bearing guest
//! (e.g. `war.ime.keyboard`) uses to push keystrokes to the
//! currently-focused editor.
//!
//! `send_key_event` forwards each call to the arbiter's
//! `ime-send-key-event` socket cmd. The arbiter then looks up the
//! focused editor's pid and pushes a `key-event` line down its
//! per-host control socket (task 47 step 3a). The editor's
//! `dispatch_key_v2` dispatches it as a synthetic Compose
//! `KeyEvent` — same code path a hardware key press takes.

use std::io::Write;
use std::os::unix::net::UnixStream;

use crate::bindings::my::skiko_gfx::keyboard::Host;

const ARBITER_SOCK_PATH: &str = "/data/local/tmp/wart-arbiter.sock";

impl Host for crate::HostState {
    fn send_key_event(&mut self, code_point: u32, key_id: u32, action: u8) {
        let action_str = match action {
            0 => "down",
            1 => "up",
            other => {
                log::warn!(
                    "keyboard-host: send-key-event got bad action={other}, defaulting to down"
                );
                "down"
            }
        };
        let line = format!(
            "ime-send-key-event {code_point} {key_id} {action_str}\n"
        );
        if let Err(e) = send_oneshot(&line) {
            log::warn!(
                "keyboard-host: send-key-event forward failed: {e:#}. \
                 code_point={code_point} key_id={key_id} action={action_str}",
            );
            return;
        }
        log::debug!(
            "keyboard-host: forwarded ime-send-key-event {code_point} {key_id} {action_str}"
        );
    }
}

/// Same one-shot connect pattern as `ime_host_impl::send_oneshot`.
/// Open → write one line → half-close → drain reply → close.
fn send_oneshot(line: &str) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(ARBITER_SOCK_PATH)?;
    stream.write_all(line.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    use std::io::Read;
    let mut buf = [0u8; 256];
    let _ = stream.read(&mut buf);
    Ok(())
}
