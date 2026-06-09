//! Call engine — Stage 2: the DTLS-SRTP handshake, in a wasm32-wasip2 guest.
//!
//! Two sans-IO `rtc-dtls` Endpoints (client + server) complete a real DTLS
//! handshake over a loopback "wire" (we just hand each side's transmits to the
//! other — Stage 2b replaces this with ICE + wasi-sockets UDP). On completion
//! both sides export the SRTP keying material (RFC 5764) — the REAL keys that
//! replace Stage 1's fixed key — and we prove the two independently-derived key
//! sets AGREE and actually work with SRTP.
//!
//! Run on-device: `wandr-host --run-once wandr.probe.dtls`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use rtc_dtls::config::ConfigBuilder;
use rtc_dtls::crypto::Certificate;
use rtc_dtls::endpoint::{Endpoint, EndpointEvent};
use rtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use rtc_shared::crypto::KeyingMaterialExporter; // trait → State::export_keying_material
use rtc_shared::TransportProtocol;

const KEY: usize = 16; // AES-128
const SALT: usize = 14; // CM salt

fn main() {
    let client_addr: SocketAddr = "127.0.0.1:5001".parse().unwrap();
    let server_addr: SocketAddr = "127.0.0.1:5002".parse().unwrap();

    // One self-signed cert, shared (loopback). insecure_skip_verify = the WebRTC
    // fingerprint trust model (peer cert checked out-of-band via the SDP, not a CA).
    let cert = Certificate::generate_self_signed(vec!["wandr-call".to_owned()])
        .expect("generate cert");
    let profiles = vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80];

    let server_cfg = Arc::new(
        ConfigBuilder::default()
            .with_certificates(vec![cert.clone()])
            .with_srtp_protection_profiles(profiles.clone())
            .with_insecure_skip_verify(true)
            .build(false, None)
            .expect("server config"),
    );
    let client_cfg = Arc::new(
        ConfigBuilder::default()
            .with_certificates(vec![cert])
            .with_srtp_protection_profiles(profiles)
            .with_insecure_skip_verify(true)
            .build(true, Some(server_addr))
            .expect("client config"),
    );

    let mut client = Endpoint::new(client_addr, TransportProtocol::UDP, None);
    let mut server = Endpoint::new(server_addr, TransportProtocol::UDP, Some(server_cfg));

    // Client initiates → queues ClientHello.
    client.connect(server_addr, client_cfg, None).expect("connect");

    // Pump the handshake: hand each side's transmits to the other until both
    // report HandshakeComplete. No loss on loopback, so a bounded loop with a
    // timer nudge converges.
    let (mut client_done, mut server_done) = (false, false);
    let mut flights = 0;
    for round in 0..200 {
        let now = Instant::now();
        let mut moved = false;

        while let Some(t) = client.poll_transmit() {
            moved = true;
            flights += 1;
            for ev in server.read(now, client_addr, None, t.message).expect("server read") {
                if matches!(ev, EndpointEvent::HandshakeComplete) { server_done = true; }
            }
        }
        while let Some(t) = server.poll_transmit() {
            moved = true;
            flights += 1;
            for ev in client.read(now, server_addr, None, t.message).expect("client read") {
                if matches!(ev, EndpointEvent::HandshakeComplete) { client_done = true; }
            }
        }

        if client_done && server_done {
            println!("[dtls] handshake complete in {round} rounds, {flights} datagrams");
            break;
        }
        if !moved {
            // No packets in flight + not done → nudge the retransmit timers.
            let _ = client.handle_timeout(server_addr, now);
            let _ = server.handle_timeout(client_addr, now);
        }
    }
    assert!(client_done && server_done, "DTLS handshake did not complete");

    // Export SRTP keying material on BOTH sides (RFC 5764).
    let cs = client.get_connection_state(server_addr).expect("client state");
    let ss = server.get_connection_state(client_addr).expect("server state");
    let profile = cs.srtp_protection_profile();
    println!("[dtls] negotiated SRTP profile = {profile:?}");

    let len = 2 * (KEY + SALT);
    let ckm = cs.export_keying_material("EXTRACTOR-dtls_srtp", &[], len).expect("client export");
    let skm = ss.export_keying_material("EXTRACTOR-dtls_srtp", &[], len).expect("server export");
    println!("[dtls] exported {len} bytes keying material on each side");

    // Both sides MUST derive identical keying material from the shared master secret.
    assert_eq!(ckm, skm, "client and server derived DIFFERENT SRTP keys!");
    println!("[dtls] client and server keying material AGREE ✓");

    // RFC 5764 layout: [client_key][server_key][client_salt][server_salt].
    let client_key = &ckm[0..KEY];
    let server_key = &ckm[KEY..2 * KEY];
    let client_salt = &ckm[2 * KEY..2 * KEY + SALT];
    let server_salt = &ckm[2 * KEY + SALT..2 * KEY + 2 * SALT];
    println!(
        "[dtls] derived SRTP keys: client_key={} server_key={} (fingerprint-distinct)",
        hex8(client_key), hex8(server_key),
    );

    // Prove the derived keys actually work with SRTP: client protects with its
    // send keys, server unprotects with the same (the client→server direction).
    use rtc_srtp::context::Context;
    use rtc_srtp::protection_profile::ProtectionProfile;
    let mut tx = Context::new(client_key, client_salt, ProtectionProfile::Aes128CmHmacSha1_80, None, None)
        .expect("srtp tx");
    let mut rx = Context::new(client_key, client_salt, ProtectionProfile::Aes128CmHmacSha1_80, None, None)
        .expect("srtp rx");
    let _ = (server_key, server_salt); // the reverse direction uses these

    // A minimal RTP packet (12-byte header, SSRC, + payload), protect→unprotect.
    let mut rtp = vec![0x80, 111, 0x00, 0x01, 0, 0, 0, 0, 0xDE, 0xAD, 0xBE, 0xEF];
    rtp.extend_from_slice(b"opus-payload-stand-in");
    let protected = tx.encrypt_rtp(&rtp).expect("srtp encrypt with DTLS keys");
    let recovered = rx.decrypt_rtp(&protected).expect("srtp decrypt with DTLS keys");
    assert_eq!(&recovered[..], &rtp[..], "SRTP round-trip with DTLS-derived keys failed");
    println!("[dtls] SRTP protect/unprotect with the DTLS-derived keys: OK ({}→{} bytes)", rtp.len(), protected.len());

    println!("DTLS-SRTP OK — handshake + agreeing key export + usable SRTP keys on wasm32-wasip2");
}

fn hex8(b: &[u8]) -> String {
    b.iter().take(4).map(|x| format!("{x:02x}")).collect::<String>() + ".."
}
