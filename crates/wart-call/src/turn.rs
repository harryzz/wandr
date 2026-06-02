//! TURN relay configuration — credentials + server addresses for allocating a relay
//! so a call connects across NAT when there's no direct path. Signal supplies these
//! per call (its calling-relays endpoint); the consuming engine parses Signal's
//! `turn:host:port?transport=udp` URLs into this struct and passes it to
//! [`crate::PeerSession::new_signal`]. `wart-call` stays transport-agnostic: it just
//! feeds these to `rtc_turn::ClientConfig`.
//!
//! Addresses must be **`ip:port`** (not hostnames): `wart-call` matches inbound
//! datagrams against `turn_serv_addr` to demux TURN traffic, and the guest's wasip2
//! runtime has no reliable DNS — the engine resolves to IPs (Signal's `urlsWithIps`).

/// Parsed TURN server credentials for one relay allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnConfig {
    /// `ip:port` of the STUN server (often the same host); empty to skip STUN.
    pub stun_serv_addr: String,
    /// `ip:port` of the TURN server.
    pub turn_serv_addr: String,
    pub username: String,
    pub password: String,
    /// TURN realm (Signal returns it as the server `hostname`).
    pub realm: String,
}
