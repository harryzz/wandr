//! [`PeerSession`] — the call engine API. Composes signaling + transport + media
//! into an event-loop-driveable session: the guest feeds it inbound datagrams +
//! time, drains outbound datagrams, exchanges SDP out-of-band, and pumps PCM in
//! and out.
//!
//! ```ignore
//! let mut s = PeerSession::new(Role::Offerer)?;
//! let offer = s.local_signaling().to_sdp();        // → send over signaling channel
//! s.set_remote_signaling(&Signaling::from_sdp(&answer)?)?;   // ← peer's answer
//! loop {
//!     for dg in s.poll_transmit() { udp.send(dg); }          // → UDP socket
//!     while let Some(dg) = udp.recv() { s.handle_datagram(&dg)?; }
//!     s.handle_timeout(now());
//!     if s.is_connected() {
//!         s.send_audio(&mic_frame)?;                          // mic → wire
//!         while let Some(pcm) = s.recv_audio() { speaker.play(&pcm); }  // wire → speaker
//!     }
//! }
//! ```

use std::time::Instant;

use crate::media::MediaSession;
use crate::signaling::Signaling;
use crate::transport::{Demux, Transport};
use crate::{Error, OPUS_PAYLOAD_TYPE, SAMPLE_RATE};

/// Which end of the call this is. Offerer sends the SDP offer; Answerer answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Offerer,
    Answerer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    New,
    Connecting,
    Connected,
}

pub struct PeerSession {
    role: Role,
    ufrag: String,
    pwd: String,
    transport: Transport,
    media: Option<MediaSession>,
    /// SRTP media datagrams queued for the wire (drained by `poll_transmit`).
    media_out: Vec<Vec<u8>>,
    /// Decoded PCM frames from inbound media (drained by `recv_audio`).
    audio_in: Vec<Vec<f32>>,
}

impl PeerSession {
    pub fn new(role: Role) -> Result<Self, Error> {
        // Per-role ICE creds (exchanged via signaling). A production session
        // generates these randomly; fixed-per-role is fine for one call pair.
        let (ufrag, pwd) = match role {
            Role::Offerer => ("ufrAgentA", "passwordApasswordApasswordA00"),
            Role::Answerer => ("ufrAgentB", "passwordBpasswordBpasswordB00"),
        };
        let transport = Transport::new(role, ufrag, pwd)?;
        Ok(Self {
            role,
            ufrag: ufrag.to_owned(),
            pwd: pwd.to_owned(),
            transport,
            media: None,
            media_out: Vec::new(),
            audio_in: Vec::new(),
        })
    }

    /// Our signaling params — marshal to SDP and send over the signaling channel.
    pub fn local_signaling(&self) -> Signaling {
        Signaling {
            ice_ufrag: self.ufrag.clone(),
            ice_pwd: self.pwd.clone(),
            fingerprint: self.transport.fingerprint().to_owned(),
            setup: match self.role {
                Role::Offerer => "actpass".to_owned(),
                Role::Answerer => "active".to_owned(),
            },
            candidates: vec!["1 1 udp 2130706431 127.0.0.1 0 typ host".to_owned()],
        }
    }

    /// Apply the peer's signaling (from their SDP) → start ICE connectivity.
    pub fn set_remote_signaling(&mut self, remote: &Signaling) -> Result<(), Error> {
        self.transport.set_remote(&remote.ice_ufrag, &remote.ice_pwd)
    }

    pub fn state(&self) -> SessionState {
        if self.transport.is_connected() {
            SessionState::Connected
        } else {
            SessionState::Connecting
        }
    }

    pub fn is_connected(&self) -> bool {
        self.transport.is_connected()
    }

    /// Drain outbound datagrams (ICE/DTLS handshake + SRTP media) for the wire.
    pub fn poll_transmit(&mut self) -> Vec<Vec<u8>> {
        let mut out = self.transport.poll_transmit();
        out.append(&mut self.media_out);
        out
    }

    /// Feed one inbound datagram; demuxed to ICE/DTLS/media. Decoded PCM lands in
    /// `recv_audio`.
    pub fn handle_datagram(&mut self, data: &[u8]) -> Result<(), Error> {
        if let Demux::Media(srtp) = self.transport.handle_datagram(data)? {
            self.ensure_media()?;
            if let Some(m) = &mut self.media {
                if let Ok(pcm) = m.recv(&srtp) {
                    self.audio_in.push(pcm);
                }
            }
        }
        Ok(())
    }

    pub fn handle_timeout(&mut self, now: Instant) {
        self.transport.handle_timeout(now);
        let _ = self.ensure_media();
    }

    /// PCM frame length (samples) the codec expects per 20 ms tick.
    pub fn frame_len(&self) -> usize {
        SAMPLE_RATE as usize / 1000 * 20
    }

    /// Encode + protect one PCM frame → queued for the next `poll_transmit`.
    pub fn send_audio(&mut self, pcm: &[f32]) -> Result<(), Error> {
        self.ensure_media()?;
        let m = self.media.as_mut().ok_or(Error::NotConnected)?;
        let dg = m.send(pcm)?;
        self.media_out.push(dg);
        Ok(())
    }

    /// Next decoded PCM frame from the peer, if any.
    pub fn recv_audio(&mut self) -> Option<Vec<f32>> {
        if self.audio_in.is_empty() { None } else { Some(self.audio_in.remove(0)) }
    }

    /// Build the media session once the DTLS keys are available.
    fn ensure_media(&mut self) -> Result<(), Error> {
        if self.media.is_some() {
            return Ok(());
        }
        if let Some((send, recv)) = self.transport.take_keys() {
            let ssrc = if self.role == Role::Offerer { 0xA } else { 0xB };
            self.media = Some(MediaSession::new(SAMPLE_RATE, 1, OPUS_PAYLOAD_TYPE, ssrc, &send, &recv)?);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Reproduce the capstone through the library API: two PeerSessions exchange
    /// signaling, connect (ICE → DTLS-SRTP), and pass an encrypted Opus frame.
    #[test]
    fn two_peers_connect_and_exchange_audio() {
        let mut a = PeerSession::new(Role::Offerer).unwrap();
        let mut b = PeerSession::new(Role::Answerer).unwrap();

        // Signaling: exchange offer/answer (via SDP round-trip, exercising it).
        let offer = a.local_signaling().to_sdp();
        let answer = b.local_signaling().to_sdp();
        a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();

        // Drive both until connected (in-memory wire).
        let mut connected = false;
        for _ in 0..800 {
            let now = Instant::now();
            a.handle_timeout(now);
            b.handle_timeout(now);
            for dg in a.poll_transmit() { b.handle_datagram(&dg).unwrap(); }
            for dg in b.poll_transmit() { a.handle_datagram(&dg).unwrap(); }
            if a.is_connected() && b.is_connected() { connected = true; break; }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(connected, "PeerSessions failed to connect");

        // Media: A sends a tone frame → B decodes non-silent audio.
        let frame: Vec<f32> = (0..a.frame_len())
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        a.send_audio(&frame).unwrap();
        for dg in a.poll_transmit() { b.handle_datagram(&dg).unwrap(); }
        let got = b.recv_audio().expect("B received an audio frame");
        let rms = (got.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / got.len() as f64).sqrt();
        assert!(rms > 0.0, "decoded audio present");
    }
}
