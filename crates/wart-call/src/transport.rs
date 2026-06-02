//! Transport — ICE connectivity + the DTLS-SRTP handshake, driven as one unit
//! over **real datagrams**: `poll_transmit` yields `(dest, bytes)` to send on a
//! UDP socket; `handle_datagram(src, bytes)` feeds what the socket received.
//!
//! Proper ICE: add *all* of the peer's candidates, run connectivity checks until
//! a pair is **selected**, then run DTLS over the selected remote and export the
//! SRTP keys (verifying the peer's cert against its SDP fingerprint). Inbound
//! datagrams are demuxed by leading byte (STUN → ICE, DTLS → DTLS, SRTP → media).
//!
//! Composes the device-verified repros/call-{ice-connect,dtls-handshake}; UDP is
//! repros/wasi-udp-probe; interop with the webrtc-rs async stack is repros/call-interop.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use sansio::Protocol;

use rtc_dtls::config::{ClientAuthType, ConfigBuilder, HandshakeConfig};
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
use sha2::{Digest, Sha256};

use crate::media::SrtpKeys;
use crate::session::Role;
use crate::Error;

const KEY: usize = 16;
const SALT: usize = 14;
const EXPORT_LEN: usize = 2 * (KEY + SALT);

pub(crate) enum Demux {
    Consumed,
    Media(Vec<u8>),
}

pub(crate) struct Transport {
    role: Role,
    local: SocketAddr,
    /// The selected pair's remote address — set once ICE connects.
    remote: Option<SocketAddr>,
    ice: Agent,
    dtls: DtlsEndpoint,
    client_cfg: Option<Arc<HandshakeConfig>>,
    dtls_started: bool,
    dtls_done: bool,
    fingerprint: String,
    expected_remote_fp: Option<String>,
    keys: Option<(SrtpKeys, SrtpKeys)>,
}

impl Transport {
    /// `local` is our bound UDP socket address — advertised as our host candidate.
    pub(crate) fn new(role: Role, ufrag: &str, pwd: &str, local: SocketAddr) -> Result<Self, Error> {
        let ice = Agent::new(Arc::new(AgentConfig {
            local_ufrag: ufrag.to_owned(),
            local_pwd: pwd.to_owned(),
            multicast_dns_mode: MulticastDnsMode::Disabled,
            ..Default::default()
        }))
        .map_err(|_| Error::Ice("agent"))?;

        let cert = Certificate::generate_self_signed(vec!["wart-call".to_owned()])
            .map_err(|_| Error::Dtls("cert"))?;
        let der = cert.certificate.first().ok_or(Error::Dtls("cert der"))?;
        let fingerprint = fingerprint_sha256(der.as_ref());

        // Offerer = DTLS server (requests the client cert for the fingerprint
        // check). Answerer = DTLS client (connects to the selected remote later).
        let (dtls, client_cfg) = match role {
            Role::Offerer => {
                let server_cfg = Arc::new(
                    ConfigBuilder::default()
                        .with_certificates(vec![cert])
                        .with_srtp_protection_profiles(srtp_profiles())
                        .with_insecure_skip_verify(true)
                        .with_client_auth(ClientAuthType::RequireAnyClientCert)
                        .build(false, None)
                        .map_err(|_| Error::Dtls("server cfg"))?,
                );
                (DtlsEndpoint::new(local, TransportProtocol::UDP, Some(server_cfg)), None)
            }
            Role::Answerer => {
                let client_cfg = Arc::new(
                    ConfigBuilder::default()
                        .with_certificates(vec![cert])
                        .with_srtp_protection_profiles(srtp_profiles())
                        .with_insecure_skip_verify(true)
                        .build(true, None)
                        .map_err(|_| Error::Dtls("client cfg"))?,
                );
                (DtlsEndpoint::new(local, TransportProtocol::UDP, None), Some(client_cfg))
            }
        };

        let mut t = Self {
            role, local, remote: None, ice, dtls, client_cfg,
            dtls_started: false, dtls_done: false, fingerprint,
            expected_remote_fp: None, keys: None,
        };
        t.ice.add_local_candidate(host_candidate(local)).map_err(|_| Error::Ice("local cand"))?;
        Ok(t)
    }

    pub(crate) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub(crate) fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote
    }

    /// Apply the remote's signaling (creds + fingerprint) + ALL its candidates
    /// (those matching our address family) + start connectivity checks.
    pub(crate) fn set_remote(
        &mut self,
        ufrag: &str,
        pwd: &str,
        fingerprint: &str,
        remotes: &[SocketAddr],
    ) -> Result<(), Error> {
        if !fingerprint.is_empty() {
            self.expected_remote_fp = Some(normalize_fp(fingerprint));
        }
        let mut added = 0;
        for r in remotes.iter().filter(|r| r.is_ipv4() == self.local.is_ipv4()) {
            if self.ice.add_remote_candidate(host_candidate(*r)).is_ok() {
                added += 1;
            }
        }
        if added == 0 {
            return Err(Error::Ice("no reachable remote candidate"));
        }
        let controlling = matches!(self.role, Role::Offerer);
        self.ice
            .start_connectivity_checks(controlling, ufrag.to_owned(), pwd.to_owned())
            .map_err(|_| Error::Ice("start checks"))?;
        Ok(())
    }

    pub(crate) fn handle_timeout(&mut self, now: Instant) {
        let _ = self.ice.handle_timeout(now);
        if let Some(remote) = self.remote {
            let _ = self.dtls.handle_timeout(remote, now);
        }
        self.maybe_start_dtls();
    }

    pub(crate) fn poll_transmit(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        self.maybe_start_dtls();
        let mut out = Vec::new();
        while let Some(t) = self.ice.poll_write() {
            out.push((t.transport.peer_addr, t.message.to_vec()));
        }
        while let Some(t) = self.dtls.poll_transmit() {
            out.push((t.transport.peer_addr, t.message.to_vec()));
        }
        out
    }

    pub(crate) fn handle_datagram(&mut self, src: SocketAddr, data: &[u8]) -> Result<Demux, Error> {
        match data.first().copied() {
            Some(0..=3) => {
                let msg = TaggedBytesMut {
                    now: Instant::now(),
                    transport: ctx(self.local, src),
                    message: bytes::BytesMut::from(data),
                };
                let _ = self.ice.handle_read(msg);
                while self.ice.poll_event().is_some() {}
                Ok(Demux::Consumed)
            }
            Some(20..=63) => {
                let evs = self
                    .dtls
                    .read(Instant::now(), src, None, bytes::BytesMut::from(data))
                    .map_err(|_| Error::Dtls("read"))?;
                if evs.iter().any(|e| matches!(e, EndpointEvent::HandshakeComplete)) {
                    self.on_dtls_done(src)?;
                }
                Ok(Demux::Consumed)
            }
            Some(128..=255) => Ok(Demux::Media(data.to_vec())),
            _ => Ok(Demux::Consumed),
        }
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.ice_connected() && self.dtls_done
    }

    pub(crate) fn take_keys(&mut self) -> Option<(SrtpKeys, SrtpKeys)> {
        self.keys.take()
    }

    // ── internals ───────────────────────────────────────────────────────────

    fn ice_connected(&self) -> bool {
        matches!(self.ice.state(), ConnectionState::Connected | ConnectionState::Completed)
    }

    /// Once ICE selects a pair, latch the selected remote + (client side) start DTLS.
    fn maybe_start_dtls(&mut self) {
        if self.dtls_started || !self.ice_connected() {
            return;
        }
        let Some(remote) = self.ice.get_selected_candidate_pair().map(|(_, r)| r.addr()) else {
            return; // connected but pair not yet exposed — try next tick
        };
        self.remote = Some(remote);
        if let Some(cfg) = self.client_cfg.take() {
            let _ = self.dtls.connect(remote, cfg, None);
        }
        self.dtls_started = true;
    }

    fn on_dtls_done(&mut self, remote: SocketAddr) -> Result<(), Error> {
        if self.dtls_done {
            return Ok(());
        }
        let state = self.dtls.get_connection_state(remote).ok_or(Error::Dtls("no state"))?;
        if let Some(expected) = &self.expected_remote_fp {
            let peer_der = state.peer_certificates.first().ok_or(Error::Dtls("no peer cert"))?;
            let got = normalize_fp(&fingerprint_sha256(peer_der));
            if &got != expected {
                return Err(Error::Dtls("peer fingerprint mismatch — possible MITM"));
            }
        }
        let km = state
            .export_keying_material("EXTRACTOR-dtls_srtp", &[], EXPORT_LEN)
            .map_err(|_| Error::Dtls("export keys"))?;
        let client = SrtpKeys { key: arr16(&km[0..16]), salt: arr14(&km[32..46]) };
        let server = SrtpKeys { key: arr16(&km[16..32]), salt: arr14(&km[46..60]) };
        self.keys = Some(match self.role {
            Role::Answerer => (client, server),
            Role::Offerer => (server, client),
        });
        self.dtls_done = true;
        Ok(())
    }
}

fn srtp_profiles() -> Vec<SrtpProtectionProfile> {
    vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80]
}

fn host_candidate(addr: SocketAddr) -> Candidate {
    CandidateHostConfig {
        base_config: CandidateConfig {
            network: "udp".to_owned(),
            address: addr.ip().to_string(),
            port: addr.port(),
            component: 1,
            ..Default::default()
        },
        ..Default::default()
    }
    .new_candidate_host()
    .expect("host candidate")
}

fn ctx(local: SocketAddr, remote: SocketAddr) -> TransportContext {
    TransportContext { local_addr: local, peer_addr: remote, transport_protocol: TransportProtocol::UDP, ecn: None }
}

fn arr16(s: &[u8]) -> [u8; 16] { let mut a = [0; 16]; a.copy_from_slice(s); a }
fn arr14(s: &[u8]) -> [u8; 14] { let mut a = [0; 14]; a.copy_from_slice(s); a }

/// SDP `a=fingerprint` value over a cert's DER: `sha-256 AB:CD:…` (upper hex).
fn fingerprint_sha256(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let hex: Vec<String> = digest.iter().map(|b| format!("{b:02X}")).collect();
    format!("sha-256 {}", hex.join(":"))
}

fn normalize_fp(fp: &str) -> String {
    fp.to_ascii_uppercase().split_whitespace().collect::<Vec<_>>().join(" ")
}
