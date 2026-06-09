//! Call engine — Stage 1: the media plane, assembled in a wasm32-wasip2 guest.
//!
//! Composes the three de-risked pieces (Opus codec, SRTP, RTP) into the full
//! send→wire→receive media pipeline a WebRTC call runs:
//!
//!   PCM ─Opus enc→ payload ─RTP packetize→ pkt ─SRTP protect→ srtp ──┐
//!                                                                      │ (wire)
//!   PCM ←Opus dec─ payload ←RTP depacketize─ pkt ←SRTP unprotect─ srtp┘
//!
//! Input is a synthetic 440 Hz tone (a pure pipeline test; live mic↔speaker is
//! blocked by this device's input+output MMAP limit). A fixed SRTP master key +
//! loopback context stand in for the keys the DTLS handshake derives later.
//! Run on-device: `wandr-host --run-once wandr.probe.callmedia`.

use std::time::Instant;

use bytes::Bytes;
use opus_rs::{Application, OpusDecoder, OpusEncoder};
use rtc_rtp::header::Header;
use rtc_rtp::packet::Packet;
use rtc_shared::marshal::{Marshal, Unmarshal};
use rtc_srtp::context::Context;
use rtc_srtp::protection_profile::ProtectionProfile;

const SR: usize = 48_000;
const CH: usize = 1;
const FRAME: usize = SR / 1000 * 20; // 20 ms = 960 samples @ 48k
const OPUS_PT: u8 = 111; // dynamic payload type for Opus (WebRTC convention)
const SSRC: u32 = 0xDEAD_BEEF;

// Fixed SRTP_AES128_CM_HMAC_SHA1_80 key material (16-byte key + 14-byte salt).
// In a real call these come from the DTLS-SRTP key export; here both directions
// share them (loopback).
const MASTER_KEY: [u8; 16] = [
    0x0d, 0xcd, 0x21, 0x3e, 0x4c, 0xbc, 0xf2, 0x8f, 0x01, 0x7f, 0x69, 0x94, 0x40, 0x1e, 0x28, 0x89,
];
const MASTER_SALT: [u8; 14] = [
    0x62, 0x77, 0x60, 0x38, 0xc0, 0x6d, 0xc9, 0x41, 0x9f, 0x6d, 0xd9, 0x43, 0x3e, 0x7c,
];

fn main() {
    let mut enc = OpusEncoder::new(SR as _, CH as _, Application::Voip).expect("enc");
    let mut dec = OpusDecoder::new(SR as _, CH as _).expect("dec");

    // Separate send / receive SRTP contexts (same keys) — SRTP is directional.
    let profile = ProtectionProfile::Aes128CmHmacSha1_80;
    let mut send_ctx = Context::new(&MASTER_KEY, &MASTER_SALT, profile, None, None).expect("send ctx");
    let mut recv_ctx = Context::new(&MASTER_KEY, &MASTER_SALT, profile, None, None).expect("recv ctx");

    const FRAMES: usize = 50; // 1 s of audio
    let in_rms = rms(&tone(0));
    let (mut seq, mut ts) = (1u16, 0u32);
    let mut pkt_buf = vec![0u8; 4000];
    let mut pcm = vec![0f32; FRAME];
    let (mut opus_sz, mut srtp_sz, mut last_out_rms) = (0usize, 0usize, 0.0);
    let mut pipe_ns = 0u128;

    for f in 0..FRAMES {
        let input = tone(f);
        let t0 = Instant::now();

        // 1. Opus encode → payload
        let n = enc.encode(&input, FRAME, &mut pkt_buf).expect("encode");
        let payload = Bytes::copy_from_slice(&pkt_buf[..n]);

        // 2. RTP packetize
        let pkt = Packet {
            header: Header {
                version: 2,
                payload_type: OPUS_PT,
                sequence_number: seq,
                timestamp: ts,
                ssrc: SSRC,
                marker: f == 0,
                ..Default::default()
            },
            payload: payload.clone(),
        };
        let rtp_raw = pkt.marshal().expect("rtp marshal");

        // 3. SRTP protect → wire
        let wire = send_ctx.encrypt_rtp(&rtp_raw).expect("srtp encrypt");

        // ───── wire (in a real call: → UDP → ICE → remote peer) ─────

        // 4. SRTP unprotect
        let rtp_back = recv_ctx.decrypt_rtp(&wire).expect("srtp decrypt");

        // 5. RTP depacketize
        let mut b = &rtp_back[..];
        let pkt2 = Packet::unmarshal(&mut b).expect("rtp unmarshal");
        assert_eq!(pkt2.header.payload_type, OPUS_PT, "payload type survived");
        assert_eq!(pkt2.header.ssrc, SSRC, "ssrc survived");

        // 6. Opus decode
        let samples = dec.decode(&pkt2.payload, FRAME, &mut pcm).expect("decode");
        pipe_ns += t0.elapsed().as_nanos();

        assert_eq!(samples, FRAME, "frame size survived");
        (opus_sz, srtp_sz, last_out_rms) = (n, wire.len(), rms(&pcm[..samples]));
        seq = seq.wrapping_add(1);
        ts = ts.wrapping_add(FRAME as u32);
    }

    let pipe_ms = pipe_ns as f64 / FRAMES as f64 / 1e6;
    println!(
        "[media] opus={opus_sz}B → srtp={srtp_sz}B (+{} auth/hdr overhead)",
        srtp_sz as i64 - opus_sz as i64
    );
    println!("[media] {FRAMES} frames; in_rms={in_rms:.4} out_rms={last_out_rms:.4}");
    println!(
        "[media] full pipeline (opus+rtp+srtp, both ways) avg = {pipe_ms:.3}ms / 20ms frame ({:.0}x real-time)",
        20.0 / pipe_ms
    );

    assert!(srtp_sz > opus_sz, "SRTP must add header + auth tag");
    assert!(last_out_rms > 0.02, "no audio survived the pipeline");
    println!("MEDIA PLANE OK — PCM→Opus→RTP→SRTP→(wire)→SRTP→RTP→Opus→PCM assembled + runs on wasip2");
}

/// 440 Hz sine for frame index `f` (continuous phase).
fn tone(f: usize) -> Vec<f32> {
    (0..FRAME)
        .map(|i| {
            let n = (f * FRAME + i) as f32;
            (2.0 * std::f32::consts::PI * 440.0 * n / SR as f32).sin() * 0.5
        })
        .collect()
}

fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len() as f64).sqrt()
}
