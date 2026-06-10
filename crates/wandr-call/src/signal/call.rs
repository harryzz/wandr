//! `SignalCall` — the Signal 1:1 call protocol driver (ringrtc V4, audio-only).
//!
//! Ties the Phase-1 `opaque` codec ([`super`]) and the Phase-2a X25519-DH keying
//! ([`crate::transport`]) into the offer→answer→connected→hangup state machine plus
//! ICE trickle and the audio pump — the unit the `wandr.signal` engine runs as a
//! persistent call loop.
//!
//! **Transport- and Signal-proto-agnostic**, like the rest of `wandr-call`: it deals
//! in `opaque` blobs + PCM + UDP datagrams and emits/consumes abstract
//! [`CallSignal`] events. The engine maps `CallSignal` ⇄ libsignal's `CallMessage`
//! and supplies the real `MessageSender` / `wasi:sockets` UDP socket / host `audio`
//! interface; nothing here depends on libsignal or a particular transport.
//!
//! ```ignore
//! // caller
//! let mut call = SignalCall::place(call_id, addr, my_identity, peer_identity)?;
//! for sig in call.poll_signals() { engine.send_call_message(peer, sig); } // Offer + Ice
//! // … on an inbound CallMessage: call.on_signal(sig)?;  (Answer, Ice, Hangup)
//! loop {                                  // the engine's call tick
//!     for (dst, dg) in call.poll_transmit() { udp.send_to(&dg, dst); }
//!     while let Ok((n, src)) = udp.recv_from(&mut buf) { call.handle_datagram(src, &buf[..n])?; }
//!     call.handle_timeout(now());
//!     for sig in call.poll_signals() { engine.send_call_message(peer, sig); } // trickled Ice
//!     if call.is_connected() { call.send_audio(&mic)?; while let Some(p) = call.recv_audio() { spk.play(&p); } }
//! }
//! ```

use std::net::SocketAddr;
use std::time::Instant;

use super::{
    decode_answer, decode_ice_candidate, decode_offer, encode_answer, encode_ice_candidate,
    encode_offer,
};
use crate::session::{PeerSession, Role};
use crate::signaling::Signaling;
use crate::Error;

/// The lifecycle of a 1:1 call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// Outgoing call placed; awaiting the answer.
    Outgoing,
    /// Incoming call received; awaiting the local user's accept/decline.
    Ringing,
    /// Answered on both ends; ICE/DH in progress.
    Connecting,
    /// Media is flowing.
    Connected,
    /// Hung up / declined / busy / remote-ended.
    Ended,
}

/// Why a call ended — mirrors ringrtc/Signal `Hangup.Type` (audio-only subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HangupKind {
    Normal,
    Accepted,
    Declined,
    Busy,
}

/// An abstract 1:1 signaling event. The engine maps these ⇄ libsignal `CallMessage`
/// (`Offer`/`Answer`/`IceUpdate`/`Hangup`/`Busy`), each carrying the `opaque` bytes
/// produced by the Phase-1 codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallSignal {
    Offer { call_id: u64, opaque: Vec<u8> },
    Answer { call_id: u64, opaque: Vec<u8> },
    /// One trickled ICE candidate.
    Ice { call_id: u64, opaque: Vec<u8> },
    Hangup { call_id: u64, kind: HangupKind },
    Busy { call_id: u64 },
}

impl CallSignal {
    pub fn call_id(&self) -> u64 {
        match *self {
            CallSignal::Offer { call_id, .. }
            | CallSignal::Answer { call_id, .. }
            | CallSignal::Ice { call_id, .. }
            | CallSignal::Hangup { call_id, .. }
            | CallSignal::Busy { call_id } => call_id,
        }
    }
}

/// One 1:1 audio call, bound to a single `call_id`. Owns a Signal-keyed
/// [`PeerSession`] and the call state machine.
pub struct SignalCall {
    call_id: u64,
    state: CallState,
    session: PeerSession,
    /// The decoded remote offer, stored on `incoming` until `accept` (declining a
    /// call should not start ICE/DH).
    pending_offer: Option<Signaling>,
    /// Signaling to send out, drained by [`Self::poll_signals`].
    out: Vec<CallSignal>,
}

impl SignalCall {
    /// Place an outgoing call. Queues the `Offer` + our trickled ICE candidate(s).
    /// `caller_identity` is our serialized ACI identity key, `callee_identity` the
    /// peer's (caller = offerer first — the order the DH-SRTP HKDF binds).
    pub fn place(
        call_id: u64,
        local_addr: SocketAddr,
        caller_identity: Vec<u8>,
        callee_identity: Vec<u8>,
        turn: Option<crate::turn::TurnConfig>,
    ) -> Result<Self, Error> {
        let session = PeerSession::new_signal(
            Role::Offerer, local_addr, caller_identity, callee_identity, turn,
        )?;
        let mut call = Self {
            call_id,
            state: CallState::Outgoing,
            session,
            pending_offer: None,
            out: Vec::new(),
        };
        let sig = call.session.local_signaling();
        call.out.push(CallSignal::Offer { call_id, opaque: encode_offer(&sig)? });
        call.queue_local_candidates(&sig);
        Ok(call)
    }

    /// Receive an incoming call (`offer_opaque` = the `CallMessage.Offer` blob).
    /// Enters [`CallState::Ringing`]; nothing connects until [`Self::accept`].
    /// Identity-key order matches `place` (caller = the remote offerer first).
    pub fn incoming(
        call_id: u64,
        local_addr: SocketAddr,
        caller_identity: Vec<u8>,
        callee_identity: Vec<u8>,
        offer_opaque: &[u8],
        turn: Option<crate::turn::TurnConfig>,
    ) -> Result<Self, Error> {
        let session = PeerSession::new_signal(
            Role::Answerer, local_addr, caller_identity, callee_identity, turn,
        )?;
        let offer = decode_offer(offer_opaque)?;
        Ok(Self {
            call_id,
            state: CallState::Ringing,
            session,
            pending_offer: Some(offer),
            out: Vec::new(),
        })
    }

    /// Inject the host AEAD backend for the SRTP GCM (feature `host-aead`). Call
    /// right after `place`/`incoming`, before media starts, so the per-packet GCM
    /// runs on the host's hardware AES via `wandr:crypto/aead`. See
    /// [`crate::PeerSession::set_aead_provider`].
    #[cfg(feature = "host-aead")]
    pub fn set_aead_provider(&mut self, provider: Box<dyn crate::AeadProvider>) {
        self.session.set_aead_provider(provider);
    }

    /// Accept a ringing call: apply the offer, queue the `Answer` + our trickled ICE
    /// candidate(s), and start connecting. No-op unless [`CallState::Ringing`].
    pub fn accept(&mut self) -> Result<(), Error> {
        if self.state != CallState::Ringing {
            return Ok(());
        }
        let Some(offer) = self.pending_offer.take() else { return Ok(()) };
        self.session.set_remote_signaling(&offer)?;
        let sig = self.session.local_signaling();
        self.out.push(CallSignal::Answer { call_id: self.call_id, opaque: encode_answer(&sig)? });
        self.queue_local_candidates(&sig);
        self.state = CallState::Connecting;
        Ok(())
    }

    /// Feed a received signaling event. Events for a different `call_id` are ignored
    /// (the engine routes by call id; this is defensive).
    pub fn on_signal(&mut self, sig: CallSignal) -> Result<(), Error> {
        if sig.call_id() != self.call_id {
            return Ok(());
        }
        match sig {
            // The answer to our offer: apply the peer's params + start connecting.
            CallSignal::Answer { opaque, .. } => {
                if self.state == CallState::Outgoing {
                    let answer = decode_answer(&opaque)?;
                    self.session.set_remote_signaling(&answer)?;
                    self.state = CallState::Connecting;
                }
            }
            // A trickled remote candidate.
            CallSignal::Ice { opaque, .. } => {
                if let Some(line) = decode_ice_candidate(&opaque)? {
                    self.session.add_remote_candidate(&line)?;
                }
            }
            // Remote hung up / declined, or is busy → end the call.
            CallSignal::Hangup { .. } | CallSignal::Busy { .. } => {
                self.state = CallState::Ended;
            }
            // A second Offer is glare — resolved by the engine (one active call
            // slot → it replies Busy). Ignore on an established call.
            CallSignal::Offer { .. } => {}
        }
        Ok(())
    }

    /// Hang up. Queues a `Hangup` and ends the call (no-op if already ended).
    pub fn hangup(&mut self) {
        if self.state != CallState::Ended {
            self.out.push(CallSignal::Hangup { call_id: self.call_id, kind: HangupKind::Normal });
            self.state = CallState::Ended;
        }
    }

    /// Drain signaling to send over the Signal `CallMessage` channel.
    pub fn poll_signals(&mut self) -> Vec<CallSignal> {
        std::mem::take(&mut self.out)
    }

    // ── UDP + audio pump (delegate to the PeerSession) ───────────────────────

    pub fn poll_transmit(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        if self.state == CallState::Ended {
            return Vec::new();
        }
        self.session.poll_transmit()
    }

    pub fn handle_datagram(&mut self, src: SocketAddr, data: &[u8]) -> Result<(), Error> {
        self.session.handle_datagram(src, data)
    }

    pub fn handle_timeout(&mut self, now: Instant) {
        self.session.handle_timeout(now);
        // Trickle any newly-gathered local candidate (the TURN relay candidate
        // arrives after allocation completes) as a fresh `CallSignal::Ice`.
        // NOT while `Ringing`: as the answerer the call is Ringing until accept(),
        // and ringrtc rejects an ice-update that arrives before our Answer (the
        // caller hangs up instantly). Hold candidates until accept() sends the
        // Answer and moves us to Connecting; they're drained on the next tick.
        // The offerer's Outgoing state already sent its Offer first, so it's fine.
        if !matches!(self.state, CallState::Ended | CallState::Ringing) {
            for line in self.session.take_new_local_candidates() {
                if let Ok(opaque) = encode_ice_candidate(&line) {
                    self.out.push(CallSignal::Ice { call_id: self.call_id, opaque });
                }
            }
        }
        if self.state == CallState::Connecting && self.session.is_connected() {
            self.state = CallState::Connected;
        }
        // Phase 5: ringrtc also hangs up in-band over RTP data (faster than the
        // signaling round trip) — honor it like a signaling Hangup.
        if self.session.take_peer_rtp_hangup() && self.state != CallState::Ended {
            self.state = CallState::Ended;
        }
    }

    /// PCM samples per 20 ms frame (mic frame size to feed [`Self::send_audio`]).
    pub fn frame_len(&self) -> usize {
        self.session.frame_len()
    }

    pub fn send_audio(&mut self, pcm: &[f32]) -> Result<(), Error> {
        self.session.send_audio(pcm)
    }

    pub fn recv_audio(&mut self) -> Option<Vec<f32>> {
        self.session.recv_audio()
    }

    // ── video track (task 93 Phase 3; delegates to the PeerSession) ─────────

    /// Send one encoded VP8 frame (90 kHz `timestamp` — the host encoder's
    /// `next-frame` output passes straight through).
    pub fn send_video(&mut self, frame: &[u8], timestamp: u32) -> Result<(), Error> {
        self.session.send_video(frame, timestamp)
    }

    /// Next whole inbound video frame — feed to the host decoder (`submit`).
    pub fn recv_video(&mut self) -> Option<crate::VideoFrame> {
        self.session.recv_video()
    }

    /// True once if the peer asked for a keyframe (RTCP PLI/FIR) — answer with
    /// the encoder's `request-keyframe`.
    pub fn take_keyframe_request(&mut self) -> bool {
        self.session.take_keyframe_request()
    }

    /// Ask the peer for a keyframe (our decoder lost sync).
    pub fn request_keyframe(&mut self) {
        self.session.request_keyframe()
    }

    /// The peer's latest REMB bandwidth estimate (bps) — feed to `set-bitrate`.
    pub fn peer_remb_bps(&self) -> Option<u32> {
        self.session.peer_remb_bps()
    }

    /// Video-plane counters: `((tx_frames, tx_pkts, rx_pkts, rx_frames,
    /// rx_broken), pli_tx, pli_rx, rtcp_err)`.
    pub fn video_diag(&self) -> (crate::video::VideoDiag, u64, u64, u64) {
        self.session.video_diag()
    }

    // ── Phase 5: in-call video state over the RTP-data channel ──────────────

    /// Flip our camera state (`senderStatus.video_enabled`) — the peer's
    /// ringrtc surfaces it as remote-video on/off. Resent ~1 Hz (accumulated).
    pub fn set_video_enabled(&mut self, enabled: bool) {
        self.session.set_video_enabled(self.call_id, enabled);
    }

    /// The peer toggled its camera: `Some(on)` once per change, clear-on-read.
    /// Open/close the remote-video decoder surface on this.
    pub fn take_peer_video_toggle(&mut self) -> Option<bool> {
        self.session.take_peer_video_toggle()
    }

    /// The peer's current camera state, if it ever reported one.
    pub fn peer_video_enabled(&self) -> Option<bool> {
        self.session.peer_video_enabled()
    }

    /// The send bitrate the peer asks of us (`receiverStatus.max_bitrate_bps`).
    pub fn peer_max_bitrate_bps(&self) -> Option<u64> {
        self.session.peer_max_bitrate_bps()
    }

    /// Outgoing CVO rotation (camera sensor orientation, degrees CW for the
    /// receiver) — set once when the encoder opens.
    pub fn set_video_rotation(&mut self, degrees: u32) {
        self.session.set_video_rotation(degrees);
    }

    /// True if we are the **answerer** (callee) — the side that must send
    /// ringrtc's `accepted` so the caller starts streaming media.
    pub fn is_answerer(&self) -> bool {
        matches!(self.session.role(), crate::session::Role::Answerer)
    }

    /// Send ringrtc's `accepted` control message (RTP-data PT 101 / SSRC 0xD),
    /// queued for the next `poll_transmit`. The answerer calls this once keyed and
    /// resends ~1 Hz (ringrtc does the same); the caller stays in
    /// "ConnectingBeforeAccepted" — no audio — until it arrives. No-op-errs until
    /// the SRTP keys exist.
    pub fn send_accepted(&mut self) -> Result<(), Error> {
        self.session.send_accepted(self.call_id)
    }

    pub fn state(&self) -> CallState {
        self.state
    }

    pub fn is_connected(&self) -> bool {
        self.session.is_connected()
    }

    /// Inbound-media diagnostics: `(srtp_seen, decode_ok, decode_err)`.
    pub fn media_diag(&self) -> (u64, u64, u64) {
        self.session.media_diag()
    }

    /// DIAG: inbound RTP stats `(seq_gaps, last_ts_step, last_payload_len)`.
    pub fn rtp_diag(&self) -> (u64, u32, usize) {
        self.session.rtp_diag()
    }

    /// DIAG: last decode failure `(opus error string, failing TOC, stereo-bit count)`.
    pub fn dec_err_diag(&self) -> (&'static str, u8, u64) {
        self.session.dec_err_diag()
    }

    /// DIAG: the peer's RTP `(payload_type, ssrc)` — what ringrtc actually sends.
    pub fn rtp_peer_ids(&self) -> (u8, u32) {
        self.session.rtp_peer_ids()
    }

    /// DIAG: one-line ICE/keying snapshot (role/state/selected pair/keyed).
    pub fn conn_debug(&self) -> String {
        self.session.conn_debug()
    }

    pub fn call_id(&self) -> u64 {
        self.call_id
    }

    /// Queue our local ICE candidate(s) as trickled `Ice` signals. wandr-call
    /// advertises a single host candidate (known at session creation).
    fn queue_local_candidates(&mut self, sig: &Signaling) {
        for c in &sig.candidates {
            if let Ok(opaque) = encode_ice_candidate(c) {
                self.out.push(CallSignal::Ice { call_id: self.call_id, opaque });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;
    use std::time::Duration;

    /// Two stand-in 33-byte serialized identity keys (0x05 ‖ 32) — caller, callee.
    fn identities() -> (Vec<u8>, Vec<u8>) {
        let caller: Vec<u8> = std::iter::once(0x05).chain(0..32).collect();
        let callee: Vec<u8> = std::iter::once(0x05).chain(32..64).collect();
        (caller, callee)
    }

    fn udp() -> UdpSocket {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.set_nonblocking(true).unwrap();
        s
    }

    /// Relay queued signals both ways (the in-process stand-in for the Signal
    /// `CallMessage` channel) and pump UDP both ways until both sides connect.
    fn drive(a: &mut SignalCall, asock: &UdpSocket, b: &mut SignalCall, bsock: &UdpSocket) {
        let mut buf = [0u8; 2048];
        for _ in 0..1200 {
            for sig in a.poll_signals() { b.on_signal(sig).unwrap(); }
            for sig in b.poll_signals() { a.on_signal(sig).unwrap(); }
            let now = Instant::now();
            a.handle_timeout(now);
            b.handle_timeout(now);
            for (dst, dg) in a.poll_transmit() { let _ = asock.send_to(&dg, dst); }
            for (dst, dg) in b.poll_transmit() { let _ = bsock.send_to(&dg, dst); }
            while let Ok((n, src)) = asock.recv_from(&mut buf) { a.handle_datagram(src, &buf[..n]).unwrap(); }
            while let Ok((n, src)) = bsock.recv_from(&mut buf) { b.handle_datagram(src, &buf[..n]).unwrap(); }
            if a.is_connected() && b.is_connected() { break; }
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    /// Full call: place → incoming → accept → ICE trickle → DH/GCM connect →
    /// two-way audio → hangup. The wandr↔wandr call minus the real Signal wire + mic.
    #[test]
    fn full_call_place_accept_connect_audio_hangup() {
        let (caller_id, callee_id) = identities();
        let call_id = 0x00C0FFEE;
        let (asock, bsock) = (udp(), udp());

        // Caller places — Offer + trickled Ice are queued.
        let mut a = SignalCall::place(call_id, asock.local_addr().unwrap(), caller_id.clone(), callee_id.clone(), None).unwrap();
        assert_eq!(a.state(), CallState::Outgoing);

        // The engine delivers the Offer to B; the trickled Ice follows.
        let a_signals = a.poll_signals();
        let offer = a_signals.iter().find_map(|s| match s {
            CallSignal::Offer { opaque, .. } => Some(opaque.clone()),
            _ => None,
        }).expect("place queues an Offer");
        let mut b = SignalCall::incoming(call_id, bsock.local_addr().unwrap(), caller_id, callee_id, &offer, None).unwrap();
        assert_eq!(b.state(), CallState::Ringing);
        b.accept().unwrap();
        assert_eq!(b.state(), CallState::Connecting);
        // A's already-queued trickled candidates (drained above) → B.
        for s in a_signals.into_iter().filter(|s| matches!(s, CallSignal::Ice { .. })) {
            b.on_signal(s).unwrap();
        }

        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected(), "Signal call failed to connect");
        assert_eq!(a.state(), CallState::Connected);
        assert_eq!(b.state(), CallState::Connected);

        // A sends a tone → B decrypts (GCM) + decodes non-silent audio.
        let frame: Vec<f32> = (0..a.frame_len())
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / crate::SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        a.send_audio(&frame).unwrap();
        let mut buf = [0u8; 2048];
        for (dst, dg) in a.poll_transmit() { let _ = asock.send_to(&dg, dst); }
        std::thread::sleep(Duration::from_millis(20));
        while let Ok((n, src)) = bsock.recv_from(&mut buf) { b.handle_datagram(src, &buf[..n]).unwrap(); }
        let got = b.recv_audio().expect("B received a call audio frame");
        let rms = (got.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / got.len() as f64).sqrt();
        assert!(rms > 0.0, "decoded audio present");

        // Hang up: A emits Hangup → B ends; A ends immediately.
        a.hangup();
        assert_eq!(a.state(), CallState::Ended);
        for s in a.poll_signals() { b.on_signal(s).unwrap(); }
        assert_eq!(b.state(), CallState::Ended);
    }

    /// A received `Busy` (the peer is in another call) ends the outgoing call —
    /// the receive side of the engine's one-active-call-slot glare policy.
    #[test]
    fn received_busy_ends_outgoing_call() {
        let (caller_id, callee_id) = identities();
        let call_id = 0x42;
        let sock = udp();
        let mut a = SignalCall::place(call_id, sock.local_addr().unwrap(), caller_id, callee_id, None).unwrap();
        assert_eq!(a.state(), CallState::Outgoing);
        a.on_signal(CallSignal::Busy { call_id }).unwrap();
        assert_eq!(a.state(), CallState::Ended);
        // Ended calls transmit nothing further.
        assert!(a.poll_transmit().is_empty());
    }

    /// Signals for a different call_id are ignored (engine routes by id).
    #[test]
    fn foreign_call_id_signal_ignored() {
        let (caller_id, callee_id) = identities();
        let mut a = SignalCall::place(1, udp().local_addr().unwrap(), caller_id, callee_id, None).unwrap();
        a.on_signal(CallSignal::Hangup { call_id: 999, kind: HangupKind::Normal }).unwrap();
        assert_eq!(a.state(), CallState::Outgoing, "a hangup for another call must not end this one");
    }
}
