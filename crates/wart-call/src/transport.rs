//! Transport — ICE connectivity + the DTLS-SRTP handshake, driven as one unit.
//!
//! Sequencing: run ICE checks; once a candidate pair is selected, run the DTLS
//! handshake over it; on completion, export the SRTP keys. Inbound datagrams are
//! demuxed by leading byte (STUN → ICE, DTLS → DTLS) — the same multiplexing a
//! real WebRTC socket does. SRTP media (byte ≥ 128) is handed back to the caller.
//!
//! Composes the device-verified repros/call-{ice-connect,dtls-handshake}.

use std::sync::Arc;
use std::time::Instant;

use sansio::Protocol;

use rtc_dtls::config::ConfigBuilder;
use rtc_dtls::crypto::Certificate;
use rtc_dtls::endpoint::{Endpoint as DtlsEndpoint, EndpointEvent};
use rtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use rtc_ice::agent::agent_config::AgentConfig;
use rtc_ice::agent::Agent;
use rtc_ice::candidate::candidate_host::CandidateHostConfig;
use rtc_ice::candidate::{Candidate, CandidateConfig};
use rtc_ice::mdns::MulticastDnsMode;
use rtc_ice::state::ConnectionState;
use rtc_shared::crypto::KeyingMaterialExporter;
use rtc_shared::{TaggedBytesMut, TransportContext, TransportProtocol};

use crate::media::SrtpKeys;
use crate::session::Role;
use crate::Error;

const KEY: usize = 16;
const SALT: usize = 14;
const EXPORT_LEN: usize = 2 * (KEY + SALT);

/// What a demuxed inbound datagram was.
pub(crate) enum Demux {
    /// Consumed by ICE or DTLS internally.
    Consumed,
    /// SRTP media — hand to the media session.
    Media(Vec<u8>),
}

pub(crate) struct Transport {
    role: Role,
    local: std::net::SocketAddr,
    remote: std::net::SocketAddr,
    ice: Agent,
    dtls: DtlsEndpoint,
    client_cfg: Option<Arc<rtc_dtls::config::HandshakeConfig>>,
    dtls_started: bool,
    dtls_done: bool,
    fingerprint: String,
    /// RFC-5764 keying material once DTLS completes.
    keys: Option<(SrtpKeys, SrtpKeys)>,
}

impl Transport {
    pub(crate) fn new(role: Role, ufrag: &str, pwd: &str) -> Result<Self, Error> {
        // One loopback port per side stands in for the UDP socket the guest binds.
        let (local, remote) = match role {
            Role::Offerer => ("127.0.0.1:40001", "127.0.0.1:40002"),
            Role::Answerer => ("127.0.0.1:40002", "127.0.0.1:40001"),
        };
        let local = local.parse().unwrap();
        let remote = remote.parse().unwrap();

        let ice = Agent::new(Arc::new(AgentConfig {
            local_ufrag: ufrag.to_owned(),
            local_pwd: pwd.to_owned(),
            multicast_dns_mode: MulticastDnsMode::Disabled,
            ..Default::default()
        }))
        .map_err(|_| Error::Ice("agent"))?;

        // One self-signed cert; the fingerprint goes in our SDP. insecure_skip_
        // verify = the WebRTC fingerprint trust model.
        let cert = Certificate::generate_self_signed(vec!["wart-call".to_owned()])
            .map_err(|_| Error::Dtls("cert"))?;
        let fingerprint = "sha-256 00".to_owned(); // real fp wiring: hash the cert DER (TODO)
        let profiles = vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80];

        // Offerer = DTLS server (passive), Answerer = DTLS client (active). (Either
        // role works; the SDP `setup` attribute decides. We fix it here.)
        let (server_cfg, client_cfg) = (
            Some(
                Arc::new(
                    ConfigBuilder::default()
                        .with_certificates(vec![cert.clone()])
                        .with_srtp_protection_profiles(profiles.clone())
                        .with_insecure_skip_verify(true)
                        .build(false, None)
                        .map_err(|_| Error::Dtls("server cfg"))?,
                ),
            ),
            Arc::new(
                ConfigBuilder::default()
                    .with_certificates(vec![cert])
                    .with_srtp_protection_profiles(profiles)
                    .with_insecure_skip_verify(true)
                    .build(true, Some(remote))
                    .map_err(|_| Error::Dtls("client cfg"))?,
            ),
        );

        let (dtls, client_cfg) = match role {
            // Server side: endpoint holds the server config, waits for ClientHello.
            Role::Offerer => (DtlsEndpoint::new(local, TransportProtocol::UDP, server_cfg), None),
            // Client side: endpoint connects once ICE is up.
            Role::Answerer => (DtlsEndpoint::new(local, TransportProtocol::UDP, None), Some(client_cfg)),
        };

        let mut t = Self {
            role, local, remote, ice, dtls, client_cfg,
            dtls_started: false, dtls_done: false, fingerprint, keys: None,
        };
        // Gather our single host candidate.
        t.ice.add_local_candidate(host_candidate(local.port())).map_err(|_| Error::Ice("local cand"))?;
        Ok(t)
    }

    pub(crate) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Apply the remote's signaling (creds) + start connectivity checks.
    pub(crate) fn set_remote(&mut self, ufrag: &str, pwd: &str) -> Result<(), Error> {
        self.ice.add_remote_candidate(host_candidate(self.remote.port())).map_err(|_| Error::Ice("remote cand"))?;
        let controlling = matches!(self.role, Role::Offerer);
        self.ice.start_connectivity_checks(controlling, ufrag.to_owned(), pwd.to_owned())
            .map_err(|_| Error::Ice("start checks"))?;
        Ok(())
    }

    pub(crate) fn handle_timeout(&mut self, now: Instant) {
        let _ = self.ice.handle_timeout(now);
        let _ = self.dtls.handle_timeout(self.remote, now);
        self.maybe_start_dtls();
    }

    /// Drain outgoing datagrams (ICE checks + DTLS handshake) for the wire.
    pub(crate) fn poll_transmit(&mut self) -> Vec<Vec<u8>> {
        self.maybe_start_dtls();
        let mut out = Vec::new();
        while let Some(t) = self.ice.poll_write() {
            out.push(t.message.to_vec());
        }
        while let Some(t) = self.dtls.poll_transmit() {
            out.push(t.message.to_vec());
        }
        out
    }

    /// Demux + dispatch one inbound datagram.
    pub(crate) fn handle_datagram(&mut self, data: &[u8]) -> Result<Demux, Error> {
        match data.first().copied() {
            // STUN (0..3) → ICE
            Some(0..=3) => {
                let msg = TaggedBytesMut {
                    now: Instant::now(),
                    transport: ctx(self.local, self.remote),
                    message: bytes::BytesMut::from(data),
                };
                let _ = self.ice.handle_read(msg);
                while self.ice.poll_event().is_some() {}
                Ok(Demux::Consumed)
            }
            // DTLS record (20..63) → DTLS
            Some(20..=63) => {
                let evs = self
                    .dtls
                    .read(Instant::now(), self.remote, None, bytes::BytesMut::from(data))
                    .map_err(|_| Error::Dtls("read"))?;
                if evs.iter().any(|e| matches!(e, EndpointEvent::HandshakeComplete)) {
                    self.on_dtls_done()?;
                }
                Ok(Demux::Consumed)
            }
            // SRTP/SRTCP (128..) → media
            Some(128..=255) => Ok(Demux::Media(data.to_vec())),
            _ => Ok(Demux::Consumed),
        }
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.ice_connected() && self.dtls_done
    }

    /// `(send_keys, recv_keys)` for the media session, once connected.
    pub(crate) fn take_keys(&mut self) -> Option<(SrtpKeys, SrtpKeys)> {
        self.keys.take()
    }

    // ── internals ───────────────────────────────────────────────────────────

    fn ice_connected(&self) -> bool {
        matches!(self.ice.state(), ConnectionState::Connected | ConnectionState::Completed)
    }

    /// Once ICE is up, the client side kicks off the DTLS handshake.
    fn maybe_start_dtls(&mut self) {
        if self.dtls_started || !self.ice_connected() {
            return;
        }
        if let Some(cfg) = self.client_cfg.take() {
            let _ = self.dtls.connect(self.remote, cfg, None);
        }
        self.dtls_started = true;
    }

    fn on_dtls_done(&mut self) -> Result<(), Error> {
        if self.dtls_done {
            return Ok(());
        }
        let state = self.dtls.get_connection_state(self.remote).ok_or(Error::Dtls("no state"))?;
        let km = state
            .export_keying_material("EXTRACTOR-dtls_srtp", &[], EXPORT_LEN)
            .map_err(|_| Error::Dtls("export keys"))?;
        // RFC 5764: [client_key|server_key|client_salt|server_salt]. The DTLS
        // client sends with the client keys; the server with the server keys.
        let client = SrtpKeys { key: arr16(&km[0..16]), salt: arr14(&km[32..46]) };
        let server = SrtpKeys { key: arr16(&km[16..32]), salt: arr14(&km[46..60]) };
        // Our send keys = our DTLS role's keys; recv = the peer's.
        self.keys = Some(match self.role {
            Role::Answerer => (client, server), // we're the DTLS client
            Role::Offerer => (server, client),  // we're the DTLS server
        });
        self.dtls_done = true;
        Ok(())
    }
}

fn host_candidate(port: u16) -> Candidate {
    CandidateHostConfig {
        base_config: CandidateConfig {
            network: "udp".to_owned(),
            address: "127.0.0.1".to_owned(),
            port,
            component: 1,
            ..Default::default()
        },
        ..Default::default()
    }
    .new_candidate_host()
    .expect("host candidate")
}

fn ctx(local: std::net::SocketAddr, remote: std::net::SocketAddr) -> TransportContext {
    TransportContext { local_addr: local, peer_addr: remote, transport_protocol: TransportProtocol::UDP, ecn: None }
}

fn arr16(s: &[u8]) -> [u8; 16] { let mut a = [0; 16]; a.copy_from_slice(s); a }
fn arr14(s: &[u8]) -> [u8; 14] { let mut a = [0; 14]; a.copy_from_slice(s); a }
