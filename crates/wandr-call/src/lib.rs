//! # wandr-call — the wandr call engine
//!
//! A guest-side Rust library that establishes a secure real-time audio call from
//! a `wasm32-wasip2` component. WebRTC is the first backend:
//!
//! - **signaling** ([`signaling`]) — SDP offer/answer carrying ICE creds, the
//!   DTLS fingerprint, and the Opus rtpmap.
//! - **transport** ([`transport`]) — ICE connectivity + the DTLS-SRTP handshake
//!   that derives the SRTP keys.
//! - **media** ([`media`]) — Opus + RTP + SRTP: PCM ⇄ encrypted RTP datagrams.
//! - **session** ([`session`]) — [`PeerSession`], the API that composes them.
//!
//! The media core (RTP/SRTP/Opus) and the ICE transport are protocol-agnostic,
//! so a SIP or Jingle backend can reuse them later.
//!
//! ## WIT-agnostic
//! wandr-call deals only in **PCM f32 frames** and **opaque datagrams** — no host
//! WIT. The consuming guest wires the PCM ends to the host audio interface
//! (capture/playback) and the datagram ends to a UDP socket (`wasi:sockets`),
//! and carries the SDP over its own signaling channel.
//!
//! Every layer is device-verified individually + composed (repros/call-*; the
//! end-to-end call is repros/call-capstone, "CALL ESTABLISHED" on a Pixel 2 XL).

pub mod media;
pub mod session;
#[cfg(feature = "signal")]
pub mod signal;
pub mod signaling;
pub mod transport;
#[cfg(feature = "signal")]
pub mod turn;
pub mod video;

pub use media::{MediaSession, SrtpKeys};
pub use session::{PeerSession, Role, SessionState};
pub use video::{VideoFrame, VP8_PAYLOAD_TYPE};

/// AEAD backend injection (feature `host-aead`). A guest implements these to run the
/// SRTP per-packet AES-GCM on the host's hardware AES (`wandr:crypto/aead`) and hands
/// the provider to [`PeerSession::set_aead_provider`]. Re-exported from `rtc-srtp` so
/// the guest needn't depend on `rtc-srtp` directly (wandr-call stays the only seam).
#[cfg(feature = "host-aead")]
pub use rtc_srtp::{AeadCtx, AeadProvider};

/// Errors from any stage of the call engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Opus encode/decode or setup.
    Codec(&'static str),
    /// SRTP context / protect / unprotect.
    Srtp(&'static str),
    /// RTP marshal / unmarshal.
    Rtp(&'static str),
    /// SDP build / parse / missing field.
    Sdp(&'static str),
    /// ICE agent / candidate / connectivity.
    Ice(&'static str),
    /// DTLS handshake / key export.
    Dtls(&'static str),
    /// Signal X25519-DH key agreement (the `signal` feature keying path).
    Dh(&'static str),
    /// TURN relay client / allocation (the `signal` feature relay path).
    Turn(&'static str),
    /// Signal `opaque` protobuf encode/decode (the `signal` feature codec).
    Proto(&'static str),
    /// Used before the session is connected.
    NotConnected,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Codec(s) => write!(f, "codec: {s}"),
            Error::Srtp(s) => write!(f, "srtp: {s}"),
            Error::Rtp(s) => write!(f, "rtp: {s}"),
            Error::Sdp(s) => write!(f, "sdp: {s}"),
            Error::Ice(s) => write!(f, "ice: {s}"),
            Error::Dtls(s) => write!(f, "dtls: {s}"),
            Error::Dh(s) => write!(f, "dh: {s}"),
            Error::Turn(s) => write!(f, "turn: {s}"),
            Error::Proto(s) => write!(f, "proto: {s}"),
            Error::NotConnected => write!(f, "session not connected yet"),
        }
    }
}

impl std::error::Error for Error {}

/// Opus dynamic payload type. **102** — the value ringrtc/Signal uses for Opus
/// (captured on the wire from a real Signal peer: peer_pt=102 ssrc=0x7d2). A
/// real ringrtc client accepts our unsignaled audio SSRC only if the PT matches
/// a registered receive codec; PT 111 (the browser/Chrome default) is NOT in
/// ringrtc's receive map, so it silently dropped our outbound stream. The SDP
/// (`to_sdp`) advertises this same PT, and libwebrtc honors any dynamic PT, so
/// the WebRTC-native path stays compatible too.
pub const OPUS_PAYLOAD_TYPE: u8 = 102;
/// The audio device sample rate wandr uses end to end.
pub const SAMPLE_RATE: u32 = 48_000;

/// Discover the primary local (LAN) IP — the address a peer on the same network
/// reaches us at — to advertise as our host ICE candidate.
///
/// Uses the standard trick: `connect` a UDP socket to a public IP and read its
/// `local_addr`. No packets are sent; the OS just picks the egress interface, so
/// it needs no network round-trip (works behind a firewall). Returns `None` if
/// there's no route (offline). Pair the returned IP with the bound socket's port:
/// `PeerSession::new(role, SocketAddr::new(local_lan_ip()?, sock.local_addr()?.port()))`.
pub fn local_lan_ip() -> Option<std::net::IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip())
}
