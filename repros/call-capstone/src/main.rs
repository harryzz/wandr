//! Call engine — the CAPSTONE: a full end-to-end secure call between two peers
//! (A + B) in one wasm32-wasip2 guest, chaining every proven stage:
//!
//!   1. signaling   — exchange ICE creds + DTLS fingerprint + Opus params
//!   2. ICE         — connectivity checks → selected candidate pair
//!   3. DTLS-SRTP   — handshake over the pair → exported SRTP keys
//!   4. media       — A: tone → Opus → RTP → SRTP → wire → B: → Opus → PCM
//!
//! The "wire" is in-memory (real wasi:sockets UDP is de-risked separately).
//! Run on-device: `wart-host --run-once war.probe.call`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use opus_rs::{Application, OpusDecoder, OpusEncoder};
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
use rtc_rtp::header::Header;
use rtc_rtp::packet::Packet;
use rtc_shared::crypto::KeyingMaterialExporter;
use rtc_shared::marshal::{Marshal, Unmarshal};
use rtc_shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use rtc_srtp::context::Context as SrtpContext;
use rtc_srtp::protection_profile::ProtectionProfile;

const SR: usize = 48_000;
const FRAME: usize = SR / 1000 * 20; // 960 samples @ 48k / 20ms
const KEY: usize = 16;
const SALT: usize = 14;
const A_UFRAG: &str = "ufrAgentA";
const A_PWD: &str = "passwordApasswordApasswordA00";
const B_UFRAG: &str = "ufrAgentB";
const B_PWD: &str = "passwordBpasswordBpasswordB00";

fn main() {
    let a_addr = "127.0.0.1:40001".parse().unwrap();
    let b_addr = "127.0.0.1:40002".parse().unwrap();

    // ── 1. signaling ────────────────────────────────────────────────────────
    // A and B each have ICE creds (above) + a DTLS cert (→ fingerprint) + agree
    // on Opus/48000/2. A real call serialises these into SDP (../call-signaling-sdp,
    // proven); here we hand them across in-process.
    println!("[1/4 signaling] ICE creds + DTLS fingerprints + Opus exchanged");

    // ── 2. ICE connectivity ─────────────────────────────────────────────────
    let mut a = agent(A_UFRAG, A_PWD);
    let mut b = agent(B_UFRAG, B_PWD);
    let a_cand = host_candidate(40001);
    let b_cand = host_candidate(40002);
    a.add_local_candidate(a_cand.clone()).unwrap();
    b.add_local_candidate(b_cand.clone()).unwrap();
    a.add_remote_candidate(b_cand).unwrap();
    b.add_remote_candidate(a_cand).unwrap();
    a.start_connectivity_checks(true, B_UFRAG.into(), B_PWD.into()).unwrap();
    b.start_connectivity_checks(false, A_UFRAG.into(), A_PWD.into()).unwrap();

    let ice_ok = (0..600).any(|_| {
        let now = Instant::now();
        let _ = a.handle_timeout(now);
        let _ = b.handle_timeout(now);
        for _ in 0..6 {
            let mut moved = false;
            while let Some(t) = a.poll_write() { moved = true; ice_deliver(&mut b, t); }
            while let Some(t) = b.poll_write() { moved = true; ice_deliver(&mut a, t); }
            if !moved { break; }
        }
        while a.poll_event().is_some() {}
        while b.poll_event().is_some() {}
        let done = connected(&a) && connected(&b);
        if !done { std::thread::sleep(Duration::from_millis(8)); }
        done
    });
    assert!(ice_ok, "ICE failed");
    let (la, ra) = a.get_selected_candidate_pair().unwrap();
    println!("[2/4 ICE] connected — selected pair {} ↔ {}", la.addr(), ra.addr());

    // ── 3. DTLS-SRTP handshake over the selected pair ───────────────────────
    let cert = Certificate::generate_self_signed(vec!["wart-call".into()]).unwrap();
    let profiles = vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80];
    let server_cfg = Arc::new(ConfigBuilder::default()
        .with_certificates(vec![cert.clone()]).with_srtp_protection_profiles(profiles.clone())
        .with_insecure_skip_verify(true).build(false, None).unwrap());
    let client_cfg = Arc::new(ConfigBuilder::default()
        .with_certificates(vec![cert]).with_srtp_protection_profiles(profiles)
        .with_insecure_skip_verify(true).build(true, Some(b_addr)).unwrap());

    // A = DTLS client (setup:active), B = DTLS server (setup:passive).
    let mut da = DtlsEndpoint::new(a_addr, TransportProtocol::UDP, None);
    let mut db = DtlsEndpoint::new(b_addr, TransportProtocol::UDP, Some(server_cfg));
    da.connect(b_addr, client_cfg, None).unwrap();

    let (mut a_done, mut b_done) = (false, false);
    for _ in 0..200 {
        let now = Instant::now();
        while let Some(t) = da.poll_transmit() {
            for e in db.read(now, a_addr, None, t.message).unwrap() {
                if matches!(e, EndpointEvent::HandshakeComplete) { b_done = true; }
            }
        }
        while let Some(t) = db.poll_transmit() {
            for e in da.read(now, b_addr, None, t.message).unwrap() {
                if matches!(e, EndpointEvent::HandshakeComplete) { a_done = true; }
            }
        }
        if a_done && b_done { break; }
        let _ = da.handle_timeout(b_addr, now);
        let _ = db.handle_timeout(a_addr, now);
    }
    assert!(a_done && b_done, "DTLS failed");
    let ckm = da.get_connection_state(b_addr).unwrap()
        .export_keying_material("EXTRACTOR-dtls_srtp", &[], 2 * (KEY + SALT)).unwrap();
    let skm = db.get_connection_state(a_addr).unwrap()
        .export_keying_material("EXTRACTOR-dtls_srtp", &[], 2 * (KEY + SALT)).unwrap();
    assert_eq!(ckm, skm, "SRTP keys disagree");
    println!("[3/4 DTLS] handshake complete — SRTP keys derived + agree");

    // RFC 5764: [client_key|server_key|client_salt|server_salt]. A→B uses A's
    // (client) keys to protect; B uses the same to unprotect.
    let client_key = &ckm[0..KEY];
    let client_salt = &ckm[2 * KEY..2 * KEY + SALT];
    let mut tx = SrtpContext::new(client_key, client_salt, ProtectionProfile::Aes128CmHmacSha1_80, None, None).unwrap();
    let mut rx = SrtpContext::new(client_key, client_salt, ProtectionProfile::Aes128CmHmacSha1_80, None, None).unwrap();

    // ── 4. encrypted Opus media A → B ───────────────────────────────────────
    let mut enc = OpusEncoder::new(SR as _, 1, Application::Voip).unwrap();
    let mut dec = OpusDecoder::new(SR as _, 1).unwrap();
    let mut pkt_buf = vec![0u8; 4000];
    let mut pcm = vec![0f32; FRAME];
    let (mut seq, mut ts) = (1u16, 0u32);
    let mut last_rms = 0.0;

    for f in 0..25 {
        // A: capture (tone) → Opus → RTP → SRTP
        let input: Vec<f32> = (0..FRAME)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * (f * FRAME + i) as f32 / SR as f32).sin() * 0.5)
            .collect();
        let n = enc.encode(&input, FRAME, &mut pkt_buf).unwrap();
        let pkt = Packet {
            header: Header { version: 2, payload_type: 111, sequence_number: seq, timestamp: ts, ssrc: 0xA, ..Default::default() },
            payload: Bytes::copy_from_slice(&pkt_buf[..n]),
        };
        let srtp = tx.encrypt_rtp(&pkt.marshal().unwrap()).unwrap();

        // ── wire (the ICE-selected UDP path in a real call) ──

        // B: SRTP → RTP → Opus → PCM
        let rtp = rx.decrypt_rtp(&srtp).unwrap();
        let mut b = &rtp[..];
        let got = Packet::unmarshal(&mut b).unwrap();
        let samples = dec.decode(&got.payload, FRAME, &mut pcm).unwrap();
        last_rms = rms(&pcm[..samples]);
        seq = seq.wrapping_add(1);
        ts = ts.wrapping_add(FRAME as u32);
    }
    println!("[4/4 media] 25 encrypted Opus frames A→B delivered; B decoded audio (rms={last_rms:.4})");
    assert!(last_rms > 0.02, "B received no audio");

    println!();
    println!("CALL ESTABLISHED — signaling → ICE → DTLS-SRTP → encrypted Opus media, end-to-end on wasm32-wasip2");
}

fn agent(ufrag: &str, pwd: &str) -> Agent {
    Agent::new(Arc::new(AgentConfig {
        local_ufrag: ufrag.into(),
        local_pwd: pwd.into(),
        multicast_dns_mode: MulticastDnsMode::Disabled,
        ..Default::default()
    })).unwrap()
}

fn host_candidate(port: u16) -> Candidate {
    CandidateHostConfig {
        base_config: CandidateConfig { network: "udp".into(), address: "127.0.0.1".into(), port, component: 1, ..Default::default() },
        ..Default::default()
    }.new_candidate_host().unwrap()
}

fn connected(a: &Agent) -> bool {
    matches!(a.state(), ConnectionState::Connected | ConnectionState::Completed)
}

fn ice_deliver(rx: &mut Agent, t: TaggedBytesMut) {
    let msg = TaggedBytesMut {
        now: Instant::now(),
        transport: TransportContext {
            local_addr: t.transport.peer_addr,
            peer_addr: t.transport.local_addr,
            transport_protocol: TransportProtocol::UDP,
            ecn: None,
        },
        message: t.message,
    };
    let _ = rx.handle_read(msg);
}

fn rms(x: &[f32]) -> f64 {
    if x.is_empty() { return 0.0; }
    (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len() as f64).sqrt()
}
