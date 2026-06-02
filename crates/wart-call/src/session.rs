//! [`PeerSession`] — the call engine API. Composes signaling + transport + media
//! into an event-loop-driveable session over real UDP datagrams.
//!
//! ```ignore
//! let sock = UdpSocket::bind("0.0.0.0:0")?;        // wasi:sockets in the guest
//! sock.set_nonblocking(true)?;
//! let mut s = PeerSession::new(Role::Offerer, sock.local_addr()?)?;
//! let offer = s.local_signaling().to_sdp();        // → send over signaling channel
//! s.set_remote_signaling(&Signaling::from_sdp(&answer)?)?;   // ← peer's answer
//! loop {
//!     for (dest, data) in s.poll_transmit() { sock.send_to(&data, dest); }
//!     while let Ok((n, src)) = sock.recv_from(&mut buf) { s.handle_datagram(src, &buf[..n])?; }
//!     s.handle_timeout(now());
//!     if s.is_connected() {
//!         s.send_audio(&mic_frame)?;                          // mic → wire
//!         while let Some(pcm) = s.recv_audio() { speaker.play(&pcm); }  // wire → speaker
//!     }
//! }
//! ```

use std::net::SocketAddr;
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
    local_addr: SocketAddr,
    ufrag: String,
    pwd: String,
    transport: Transport,
    media: Option<MediaSession>,
    /// The offer's media direction (answerer mirrors its reverse).
    remote_direction: Option<String>,
    /// SRTP media datagrams queued for the wire (drained by `poll_transmit`).
    media_out: Vec<Vec<u8>>,
    /// Decoded PCM frames from inbound media (drained by `recv_audio`).
    audio_in: Vec<Vec<f32>>,
}

impl PeerSession {
    /// `local_addr` is the bound UDP socket address — advertised as our host
    /// candidate and used as the source address of our datagrams.
    pub fn new(role: Role, local_addr: SocketAddr) -> Result<Self, Error> {
        // Per-role ICE creds (exchanged via signaling). Production generates these
        // randomly; fixed-per-role is fine for one call pair.
        let (ufrag, pwd) = match role {
            Role::Offerer => ("ufrAgentA", "passwordApasswordApasswordA00"),
            Role::Answerer => ("ufrAgentB", "passwordBpasswordBpasswordB00"),
        };
        let transport = Transport::new(role, ufrag, pwd, local_addr)?;
        Ok(Self {
            role,
            local_addr,
            ufrag: ufrag.to_owned(),
            pwd: pwd.to_owned(),
            transport,
            media: None,
            remote_direction: None,
            media_out: Vec::new(),
            audio_in: Vec::new(),
        })
    }

    /// A Signal call session (ringrtc V4 keying): no DTLS — SRTP keys come from an
    /// X25519-DH exchange whose public key rides in signaling ([`Signaling::public_key`]).
    /// `caller_identity`/`callee_identity` are the two ACI identity public keys
    /// (serialized, caller = offerer first) that authenticate the derived keys.
    /// Pair the signaling with the [`crate::signal`] `opaque` codec.
    #[cfg(feature = "signal")]
    pub fn new_signal(
        role: Role,
        local_addr: SocketAddr,
        caller_identity: Vec<u8>,
        callee_identity: Vec<u8>,
    ) -> Result<Self, Error> {
        let (ufrag, pwd) = match role {
            Role::Offerer => ("ufrAgentA", "passwordApasswordApasswordA00"),
            Role::Answerer => ("ufrAgentB", "passwordBpasswordBpasswordB00"),
        };
        let transport =
            Transport::new_signal(role, ufrag, pwd, local_addr, caller_identity, callee_identity)?;
        Ok(Self {
            role,
            local_addr,
            ufrag: ufrag.to_owned(),
            pwd: pwd.to_owned(),
            transport,
            media: None,
            remote_direction: None,
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
            direction: match self.role {
                // Offer sendrecv; an answer mirrors the offer's direction.
                Role::Offerer => "sendrecv".to_owned(),
                Role::Answerer => {
                    let offer = self.remote_direction.as_deref().unwrap_or("sendrecv");
                    crate::signaling::reverse_direction(offer).to_owned()
                }
            },
            candidates: vec![candidate_string(self.local_addr)],
            // WebRTC-native path keys via DTLS (None); the Signal path advertises
            // its ephemeral X25519 public key here for the peer's DH.
            #[cfg(feature = "signal")]
            public_key: self.transport.signal_public_key().map(|k| k.to_vec()),
            #[cfg(not(feature = "signal"))]
            public_key: None,
        }
    }

    /// Apply the peer's signaling (from their SDP) → start ICE connectivity. The
    /// peer's cert fingerprint is checked against the handshake cert on connect.
    pub fn set_remote_signaling(&mut self, remote: &Signaling) -> Result<(), Error> {
        let remotes: Vec<SocketAddr> =
            remote.candidates.iter().filter_map(|c| parse_candidate_addr(c)).collect();
        if remotes.is_empty() {
            return Err(Error::Ice("no usable remote candidate"));
        }
        self.remote_direction = Some(remote.direction.clone());
        self.transport.set_remote(
            &remote.ice_ufrag,
            &remote.ice_pwd,
            &remote.fingerprint,
            remote.public_key.as_deref(),
            &remotes,
        )
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

    /// Drain outbound datagrams (ICE/DTLS handshake + SRTP media): `(dest, bytes)`.
    pub fn poll_transmit(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        let mut out = self.transport.poll_transmit();
        if let Some(dest) = self.transport.remote_addr() {
            for dg in self.media_out.drain(..) {
                out.push((dest, dg));
            }
        }
        out
    }

    /// Feed one inbound datagram from `src`; demuxed to ICE/DTLS/media. Decoded
    /// PCM lands in `recv_audio`.
    pub fn handle_datagram(&mut self, src: SocketAddr, data: &[u8]) -> Result<(), Error> {
        if let Demux::Media(srtp) = self.transport.handle_datagram(src, data)? {
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

    fn ensure_media(&mut self) -> Result<(), Error> {
        if self.media.is_some() {
            return Ok(());
        }
        if let Some((send, recv)) = self.transport.take_keys() {
            let ssrc = if self.role == Role::Offerer { 0xA } else { 0xB };
            let profile = self.transport.srtp_profile();
            self.media =
                Some(MediaSession::new(SAMPLE_RATE, 1, OPUS_PAYLOAD_TYPE, ssrc, profile, &send, &recv)?);
        }
        Ok(())
    }
}

/// Build a standard SDP host-candidate string for an address.
fn candidate_string(addr: SocketAddr) -> String {
    format!("1 1 udp 2130706431 {} {} typ host", addr.ip(), addr.port())
}

/// Parse the connection address from an SDP candidate string.
fn parse_candidate_addr(cand: &str) -> Option<SocketAddr> {
    // foundation component transport priority <ip> <port> typ host …
    let f: Vec<&str> = cand.split_whitespace().collect();
    if f.len() >= 6 {
        let ip: std::net::IpAddr = f[4].parse().ok()?;
        let port: u16 = f[5].parse().ok()?;
        return Some(SocketAddr::new(ip, port));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;
    use std::time::Duration;

    /// Bind a peer to a real loopback UDP socket + a PeerSession on its address.
    fn peer(role: Role) -> (PeerSession, UdpSocket) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_nonblocking(true).unwrap();
        let s = PeerSession::new(role, sock.local_addr().unwrap()).unwrap();
        (s, sock)
    }

    /// Drive two sessions over their real UDP sockets until both connect (or give
    /// up). Returns whether a fingerprint mismatch was raised.
    fn drive(
        a: &mut PeerSession, asock: &UdpSocket,
        b: &mut PeerSession, bsock: &UdpSocket,
    ) -> bool {
        let mut buf = [0u8; 2048];
        let mut mismatch = false;
        for _ in 0..1200 {
            let now = Instant::now();
            a.handle_timeout(now);
            b.handle_timeout(now);
            for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
            for (dest, data) in b.poll_transmit() { let _ = bsock.send_to(&data, dest); }
            while let Ok((n, src)) = asock.recv_from(&mut buf) {
                if matches!(a.handle_datagram(src, &buf[..n]), Err(Error::Dtls(_))) { mismatch = true; }
            }
            while let Ok((n, src)) = bsock.recv_from(&mut buf) {
                let _ = b.handle_datagram(src, &buf[..n]);
            }
            if (a.is_connected() && b.is_connected()) || mismatch { break; }
            std::thread::sleep(Duration::from_millis(3));
        }
        mismatch
    }

    /// Full call over REAL loopback UDP sockets: signaling → ICE → DTLS-SRTP →
    /// encrypted Opus media. This is repros/call-capstone over wasi:sockets-shaped
    /// transport (std::net::UdpSocket on host; wasi:sockets in the guest).
    #[test]
    fn two_peers_connect_over_real_udp() {
        let (mut a, asock) = peer(Role::Offerer);
        let (mut b, bsock) = peer(Role::Answerer);

        let offer = a.local_signaling().to_sdp();
        let answer = b.local_signaling().to_sdp();
        a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();

        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected(), "failed to connect over real UDP");

        // A sends a tone → B decodes non-silent audio.
        let frame: Vec<f32> = (0..a.frame_len())
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        a.send_audio(&frame).unwrap();
        let mut buf = [0u8; 2048];
        for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
        std::thread::sleep(Duration::from_millis(20));
        while let Ok((n, src)) = bsock.recv_from(&mut buf) { b.handle_datagram(src, &buf[..n]).unwrap(); }
        let got = b.recv_audio().expect("B received an audio frame over UDP");
        let rms = (got.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / got.len() as f64).sqrt();
        assert!(rms > 0.0, "decoded audio present");
    }

    /// A tampered fingerprint (MITM) is rejected over real UDP — no connection.
    #[test]
    fn mismatched_fingerprint_rejected_over_udp() {
        let (mut a, asock) = peer(Role::Offerer);
        let (mut b, bsock) = peer(Role::Answerer);

        let offer = a.local_signaling().to_sdp();
        let mut b_sig = Signaling::from_sdp(&b.local_signaling().to_sdp()).unwrap();
        b_sig.fingerprint = format!("sha-256 {}", vec!["DE"; 32].join(":")); // bogus
        a.set_remote_signaling(&b_sig).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();

        let mismatch = drive(&mut a, &asock, &mut b, &bsock);
        assert!(mismatch, "A must raise a fingerprint mismatch");
        assert!(!a.is_connected(), "A must not connect on fingerprint mismatch");
    }

    /// Signal-mode call (ringrtc V4): NO DTLS — keys come from X25519-DH over the
    /// public keys exchanged in signaling. Two wart peers connect over real UDP and
    /// exchange AEAD-AES-256-GCM-protected Opus. The analog of
    /// `two_peers_connect_over_real_udp`, proving the DH/GCM keying path end-to-end
    /// (the two-wart half of Phase 2; real-Signal interop is Phase 3).
    #[cfg(feature = "signal")]
    #[test]
    fn two_peers_connect_signal_dh_over_real_udp() {
        // Both peers must agree on the same (caller, callee) identity keys — caller
        // = offerer. Stand-in 33-byte serialized identity pubkeys (0x05 ‖ 32).
        let caller_id: Vec<u8> = std::iter::once(0x05).chain(0..32).collect();
        let callee_id: Vec<u8> = std::iter::once(0x05).chain(32..64).collect();

        let bind = |role| {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.set_nonblocking(true).unwrap();
            let s = PeerSession::new_signal(
                role, sock.local_addr().unwrap(), caller_id.clone(), callee_id.clone(),
            )
            .unwrap();
            (s, sock)
        };
        let (mut a, asock) = bind(Role::Offerer);
        let (mut b, bsock) = bind(Role::Answerer);

        // Signaling rides the `opaque` codec, not SDP — exchange Signaling directly
        // (it carries each peer's X25519 public_key + ICE creds/candidates).
        let offer = a.local_signaling();
        let answer = b.local_signaling();
        assert_eq!(offer.public_key.as_ref().map(|k| k.len()), Some(32), "offer carries X25519 key");
        assert!(offer.fingerprint.is_empty(), "Signal path has no DTLS fingerprint");
        a.set_remote_signaling(&answer).unwrap();
        b.set_remote_signaling(&offer).unwrap();

        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected(), "Signal-DH peers failed to connect");

        // A sends a tone → B decrypts (GCM) + decodes non-silent audio.
        let frame: Vec<f32> = (0..a.frame_len())
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        a.send_audio(&frame).unwrap();
        let mut buf = [0u8; 2048];
        for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
        std::thread::sleep(Duration::from_millis(20));
        while let Ok((n, src)) = bsock.recv_from(&mut buf) { b.handle_datagram(src, &buf[..n]).unwrap(); }
        let got = b.recv_audio().expect("B received a GCM-protected audio frame");
        let rms = (got.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / got.len() as f64).sqrt();
        assert!(rms > 0.0, "decoded audio present");
    }

    /// Mismatched identity keys (the HKDF `info`) → divergent SRTP keys → B cannot
    /// decrypt A's media. Guards that the identity binding actually feeds keying.
    #[cfg(feature = "signal")]
    #[test]
    fn signal_dh_identity_mismatch_breaks_media() {
        let bind = |role, caller: Vec<u8>, callee: Vec<u8>| {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.set_nonblocking(true).unwrap();
            let s = PeerSession::new_signal(role, sock.local_addr().unwrap(), caller, callee).unwrap();
            (s, sock)
        };
        let (mut a, asock) = bind(Role::Offerer, vec![0x05; 33], vec![0xAA; 33]);
        // B uses a different callee identity → different HKDF info → different keys.
        let (mut b, bsock) = bind(Role::Answerer, vec![0x05; 33], vec![0xBB; 33]);

        a.set_remote_signaling(&b.local_signaling()).unwrap();
        b.set_remote_signaling(&a.local_signaling()).unwrap();
        drive(&mut a, &asock, &mut b, &bsock);
        // ICE/DH still "connect" (DH succeeds; only the derived keys differ).
        assert!(a.is_connected() && b.is_connected());

        let frame = vec![0.3f32; a.frame_len()];
        a.send_audio(&frame).unwrap();
        let mut buf = [0u8; 2048];
        for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
        std::thread::sleep(Duration::from_millis(20));
        // B's SRTP unprotect fails (wrong key) → the datagram errors, no audio.
        let mut got_audio = false;
        while let Ok((n, src)) = bsock.recv_from(&mut buf) {
            let _ = b.handle_datagram(src, &buf[..n]);
            if b.recv_audio().is_some() { got_audio = true; }
        }
        assert!(!got_audio, "mismatched identity keys must not yield decryptable media");
    }
}
