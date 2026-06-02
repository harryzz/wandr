//! Transport — ICE connectivity + key agreement, driven as one unit over **real
//! datagrams**: `poll_transmit` yields `(dest, bytes)` to send on a UDP socket;
//! `handle_datagram(src, bytes)` feeds what the socket received. Inbound datagrams
//! are demuxed by leading byte (STUN → ICE, DTLS → DTLS, SRTP → media).
//!
//! Two **keying modes** share the ICE + demux machinery:
//!
//! - [`Keying::Dtls`] — WebRTC-native (the default). Proper ICE: add *all* of the
//!   peer's candidates, run connectivity checks until a pair is selected, then run
//!   DTLS over the selected remote and export the SRTP keys
//!   (AES-128-CM/HMAC-SHA1-80), verifying the peer's cert against its SDP
//!   fingerprint. Composes repros/call-{ice-connect,dtls-handshake}.
//! - [`Keying::Signal`] (`feature = "signal"`) — ringrtc's V4 1:1 keying. **No
//!   DTLS.** An X25519 public key is exchanged in *signaling* (not on the wire),
//!   and SRTP keys (AEAD-AES-256-GCM) are derived via Diffie-Hellman + HKDF as soon
//!   as ICE selects a pair. See [`crate::signal`].

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
use rtc_srtp::protection_profile::ProtectionProfile;
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

/// How the SRTP keys are agreed — the only thing that differs between the
/// WebRTC-native and Signal call paths. ICE + media are shared.
enum Keying {
    Dtls(DtlsKeying),
    #[cfg(feature = "signal")]
    Signal(SignalKeying),
}

struct DtlsKeying {
    dtls: DtlsEndpoint,
    /// Client (answerer) config, consumed when we start the handshake.
    client_cfg: Option<Arc<HandshakeConfig>>,
    started: bool,
    fingerprint: String,
    expected_remote_fp: Option<String>,
}

/// Signal's X25519-DH keying (ringrtc V4). The secret is ephemeral per call; the
/// `*_identity` keys are the two ACI identity public keys (caller = offerer first)
/// that bind the SRTP keys to the authenticated Signal session.
#[cfg(feature = "signal")]
struct SignalKeying {
    secret: x25519_dalek::StaticSecret,
    public: [u8; 32],
    remote_public: Option<[u8; 32]>,
    caller_identity: Vec<u8>,
    callee_identity: Vec<u8>,
}

#[cfg(feature = "signal")]
impl SignalKeying {
    /// Derive the SRTP send/recv key pair from the DH shared secret, exactly as
    /// ringrtc's `Connection::negotiate_srtp_keys` does (so a real Signal client
    /// derives the same keys). Returns `(send, recv)` for our [`Role`].
    fn derive(&self, role: Role) -> Result<(SrtpKeys, SrtpKeys), Error> {
        use hkdf::Hkdf;
        use x25519_dalek::PublicKey;

        let remote = self.remote_public.ok_or(Error::Dh("no remote public key yet"))?;
        let shared = self.secret.diffie_hellman(&PublicKey::from(remote));
        if !shared.was_contributory() {
            return Err(Error::Dh("non-contributory DH shared secret (rejected)"));
        }
        // HKDF-SHA256, salt = 32 zero bytes, info = label || caller_id || callee_id.
        let mut info = Vec::with_capacity(48 + self.caller_identity.len() + self.callee_identity.len());
        info.extend_from_slice(b"Signal_Calling_20200807_SignallingDH_SRTPKey_KDF");
        info.extend_from_slice(&self.caller_identity);
        info.extend_from_slice(&self.callee_identity);

        // AEAD_AES_256_GCM: 32 B key + 12 B salt per direction; offer then answer.
        let hk = Hkdf::<Sha256>::new(Some(&[0u8; 32]), shared.as_bytes());
        let mut okm = [0u8; 2 * (32 + 12)];
        hk.expand(&info, &mut okm).map_err(|_| Error::Dh("hkdf expand"))?;
        let offer = SrtpKeys { key: okm[0..32].to_vec(), salt: okm[32..44].to_vec() };
        let answer = SrtpKeys { key: okm[44..76].to_vec(), salt: okm[76..88].to_vec() };
        Ok(match role {
            // The offerer (caller) sends with offer_*; the answerer with answer_*.
            Role::Offerer => (offer, answer),
            Role::Answerer => (answer, offer),
        })
    }
}

pub(crate) struct Transport {
    role: Role,
    local: SocketAddr,
    /// The selected pair's remote address — set once ICE connects.
    remote: Option<SocketAddr>,
    ice: Agent,
    keying: Keying,
    /// (send, recv) SRTP keys — set once keying completes.
    keys: Option<(SrtpKeys, SrtpKeys)>,
    /// Keying complete (DTLS handshake done, or Signal DH derived).
    done: bool,
}

impl Transport {
    /// WebRTC-native (DTLS-SRTP). `local` is our bound UDP socket address —
    /// advertised as our host candidate.
    pub(crate) fn new(role: Role, ufrag: &str, pwd: &str, local: SocketAddr) -> Result<Self, Error> {
        let ice = make_ice(ufrag, pwd, local)?;

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

        Ok(Self {
            role, local, remote: None, ice,
            keying: Keying::Dtls(DtlsKeying {
                dtls, client_cfg, started: false, fingerprint, expected_remote_fp: None,
            }),
            keys: None, done: false,
        })
    }

    /// Signal DH (ringrtc V4). Generates an ephemeral X25519 keypair; `*_identity`
    /// are the caller's then callee's ACI identity public keys (serialized).
    #[cfg(feature = "signal")]
    pub(crate) fn new_signal(
        role: Role,
        ufrag: &str,
        pwd: &str,
        local: SocketAddr,
        caller_identity: Vec<u8>,
        callee_identity: Vec<u8>,
    ) -> Result<Self, Error> {
        use rand_core::OsRng;
        use x25519_dalek::{PublicKey, StaticSecret};

        let ice = make_ice(ufrag, pwd, local)?;
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret).to_bytes();
        Ok(Self {
            role, local, remote: None, ice,
            keying: Keying::Signal(SignalKeying {
                secret, public, remote_public: None, caller_identity, callee_identity,
            }),
            keys: None, done: false,
        })
    }

    /// The DTLS cert fingerprint to advertise in signaling (empty on the Signal
    /// path, which carries an X25519 [`Self::signal_public_key`] instead).
    pub(crate) fn fingerprint(&self) -> &str {
        match &self.keying {
            Keying::Dtls(d) => &d.fingerprint,
            #[cfg(feature = "signal")]
            Keying::Signal(_) => "",
        }
    }

    /// Our X25519 public key to advertise in signaling (Signal path only).
    #[cfg(feature = "signal")]
    pub(crate) fn signal_public_key(&self) -> Option<[u8; 32]> {
        match &self.keying {
            Keying::Signal(s) => Some(s.public),
            _ => None,
        }
    }

    /// The SRTP suite this keying produces — both `SrtpKeys` are sized for it.
    pub(crate) fn srtp_profile(&self) -> ProtectionProfile {
        match &self.keying {
            #[cfg(feature = "signal")]
            Keying::Signal(_) => ProtectionProfile::AeadAes256Gcm,
            _ => ProtectionProfile::Aes128CmHmacSha1_80,
        }
    }

    pub(crate) fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote
    }

    /// Apply the remote's signaling — keying material (DTLS fingerprint, or the
    /// Signal X25519 `remote_public`) + ALL its candidates (matching our address
    /// family) — and start connectivity checks.
    pub(crate) fn set_remote(
        &mut self,
        ufrag: &str,
        pwd: &str,
        fingerprint: &str,
        remote_public: Option<&[u8]>,
        remotes: &[SocketAddr],
    ) -> Result<(), Error> {
        #[cfg(not(feature = "signal"))]
        let _ = remote_public;
        match &mut self.keying {
            Keying::Dtls(d) => {
                if !fingerprint.is_empty() {
                    d.expected_remote_fp = Some(normalize_fp(fingerprint));
                }
            }
            #[cfg(feature = "signal")]
            Keying::Signal(s) => {
                let rp = remote_public.ok_or(Error::Dh("signaling missing remote public key"))?;
                if rp.len() != 32 {
                    return Err(Error::Dh("remote public key must be 32 bytes"));
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(rp);
                s.remote_public = Some(arr);
            }
        }
        // Add whatever candidates came with the signaling. Trickle (Signal) sends
        // none here — they arrive later via `add_remote_candidate` — so an empty
        // set is fine; the bundled (DTLS/SDP) path supplies them up front.
        for r in remotes.iter().filter(|r| r.is_ipv4() == self.local.is_ipv4()) {
            let _ = self.ice.add_remote_candidate(host_candidate(*r));
        }
        let controlling = matches!(self.role, Role::Offerer);
        self.ice
            .start_connectivity_checks(controlling, ufrag.to_owned(), pwd.to_owned())
            .map_err(|_| Error::Ice("start checks"))?;
        Ok(())
    }

    /// Add one trickled remote ICE candidate after `set_remote` started checks
    /// (the Signal path delivers candidates as separate `IceUpdate`s).
    pub(crate) fn add_remote_candidate(&mut self, addr: SocketAddr) -> Result<(), Error> {
        if addr.is_ipv4() != self.local.is_ipv4() {
            return Ok(()); // address-family mismatch — not reachable from our socket
        }
        self.ice
            .add_remote_candidate(host_candidate(addr))
            .map(|_| ())
            .map_err(|_| Error::Ice("add remote candidate"))
    }

    pub(crate) fn handle_timeout(&mut self, now: Instant) {
        let _ = self.ice.handle_timeout(now);
        let remote = self.remote;
        if let Keying::Dtls(d) = &mut self.keying {
            if let Some(remote) = remote {
                let _ = d.dtls.handle_timeout(remote, now);
            }
        }
        self.maybe_advance();
    }

    pub(crate) fn poll_transmit(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        self.maybe_advance();
        let mut out = Vec::new();
        while let Some(t) = self.ice.poll_write() {
            out.push((t.transport.peer_addr, t.message.to_vec()));
        }
        if let Keying::Dtls(d) = &mut self.keying {
            while let Some(t) = d.dtls.poll_transmit() {
                out.push((t.transport.peer_addr, t.message.to_vec()));
            }
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
            // DTLS records — only meaningful on the DTLS keying path; the Signal
            // path never speaks DTLS, so any such datagram is ignored.
            Some(20..=63) => {
                let mut complete = false;
                if let Keying::Dtls(d) = &mut self.keying {
                    let evs = d
                        .dtls
                        .read(Instant::now(), src, None, bytes::BytesMut::from(data))
                        .map_err(|_| Error::Dtls("read"))?;
                    complete = evs.iter().any(|e| matches!(e, EndpointEvent::HandshakeComplete));
                }
                if complete {
                    self.on_dtls_done(src)?;
                }
                Ok(Demux::Consumed)
            }
            Some(128..=255) => Ok(Demux::Media(data.to_vec())),
            _ => Ok(Demux::Consumed),
        }
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.ice_connected() && self.done
    }

    pub(crate) fn take_keys(&mut self) -> Option<(SrtpKeys, SrtpKeys)> {
        self.keys.take()
    }

    // ── internals ───────────────────────────────────────────────────────────

    fn ice_connected(&self) -> bool {
        matches!(self.ice.state(), ConnectionState::Connected | ConnectionState::Completed)
    }

    /// Once ICE selects a pair, latch the selected remote and advance keying:
    /// DTLS = start the client handshake; Signal = derive the SRTP keys via DH.
    fn maybe_advance(&mut self) {
        if self.done || !self.ice_connected() {
            return;
        }
        let Some(remote) = self.ice.get_selected_candidate_pair().map(|(_, r)| r.addr()) else {
            return; // connected but pair not yet exposed — try next tick
        };
        self.remote = Some(remote);
        let role = self.role;
        match &mut self.keying {
            Keying::Dtls(d) => {
                if !d.started {
                    if let Some(cfg) = d.client_cfg.take() {
                        let _ = d.dtls.connect(remote, cfg, None);
                    }
                    d.started = true;
                }
                // DTLS completion arrives as a datagram → on_dtls_done sets `done`.
            }
            #[cfg(feature = "signal")]
            Keying::Signal(s) => {
                // No handshake: keys are ready the moment the remote public key is
                // known and ICE has a pair. (Err = remote pubkey not set yet; retry.)
                if let Ok(keys) = s.derive(role) {
                    self.keys = Some(keys);
                    self.done = true;
                }
            }
        }
    }

    fn on_dtls_done(&mut self, remote: SocketAddr) -> Result<(), Error> {
        if self.done {
            return Ok(());
        }
        let keys = {
            let Keying::Dtls(d) = &self.keying else { return Ok(()) };
            let state = d.dtls.get_connection_state(remote).ok_or(Error::Dtls("no state"))?;
            if let Some(expected) = &d.expected_remote_fp {
                let peer_der = state.peer_certificates.first().ok_or(Error::Dtls("no peer cert"))?;
                let got = normalize_fp(&fingerprint_sha256(peer_der));
                if &got != expected {
                    return Err(Error::Dtls("peer fingerprint mismatch — possible MITM"));
                }
            }
            let km = state
                .export_keying_material("EXTRACTOR-dtls_srtp", &[], EXPORT_LEN)
                .map_err(|_| Error::Dtls("export keys"))?;
            let client = SrtpKeys { key: km[0..16].to_vec(), salt: km[32..46].to_vec() };
            let server = SrtpKeys { key: km[16..32].to_vec(), salt: km[46..60].to_vec() };
            match self.role {
                Role::Answerer => (client, server),
                Role::Offerer => (server, client),
            }
        };
        self.keys = Some(keys);
        self.done = true;
        Ok(())
    }
}

/// Build the ICE agent (mDNS off) and add our host candidate.
fn make_ice(ufrag: &str, pwd: &str, local: SocketAddr) -> Result<Agent, Error> {
    let mut ice = Agent::new(Arc::new(AgentConfig {
        local_ufrag: ufrag.to_owned(),
        local_pwd: pwd.to_owned(),
        multicast_dns_mode: MulticastDnsMode::Disabled,
        ..Default::default()
    }))
    .map_err(|_| Error::Ice("agent"))?;
    ice.add_local_candidate(host_candidate(local)).map_err(|_| Error::Ice("local cand"))?;
    Ok(ice)
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

/// SDP `a=fingerprint` value over a cert's DER: `sha-256 AB:CD:…` (upper hex).
fn fingerprint_sha256(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let hex: Vec<String> = digest.iter().map(|b| format!("{b:02X}")).collect();
    format!("sha-256 {}", hex.join(":"))
}

fn normalize_fp(fp: &str) -> String {
    fp.to_ascii_uppercase().split_whitespace().collect::<Vec<_>>().join(" ")
}
