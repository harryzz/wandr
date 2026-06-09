//! call-live — the call-engine capstone: a live call carrying real microphone
//! audio to a real speaker, over a real UDP socket, with real DTLS-SRTP keys.
//!
//! Two `PeerSession`s connect over real `wasi:sockets` UDP (ICE + DTLS-SRTP).
//! The device mic feeds peer A; the encrypted RTP crosses the socket; peer B
//! decodes it; the PCM plays out the speaker:
//!
//!   mic → A: Opus/RTP/SRTP(DTLS keys) ─real UDP→ B: SRTP/RTP/Opus → speaker
//!
//! This is `repros/call-udp-loopback` (the networked call) composed with
//! `repros/call-audio-wire` (real mic/AAudio) — every plane of the engine at
//! once. Record-then-play because one device can't hold input + output MMAP
//! simultaneously (a real call splits capture and playback across two phones).
//!
//!   wandr-host --run-once wandr.probe.calllive

wit_bindgen::generate!({
    world: "callaudio",
    path: "wit",
    generate_all,
});

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use wandr_call::signaling::Signaling;
use wandr_call::{PeerSession, Role, SAMPLE_RATE};

use crate::my::skiko_gfx::audio::{self, ChannelLayout, Format, TrackConfig};

/// 20 ms @ 48 kHz mono — the Opus frame + audio tick granularity.
const FRAME: usize = SAMPLE_RATE as usize / 1000 * 20;

fn pump(a: &mut PeerSession, asock: &UdpSocket, b: &mut PeerSession, bsock: &UdpSocket, buf: &mut [u8]) {
    let now = Instant::now();
    a.handle_timeout(now);
    b.handle_timeout(now);
    for (dest, data) in a.poll_transmit() {
        let _ = asock.send_to(&data, dest);
    }
    for (dest, data) in b.poll_transmit() {
        let _ = bsock.send_to(&data, dest);
    }
    while let Ok((n, src)) = asock.recv_from(buf) {
        let _ = a.handle_datagram(src, &buf[..n]);
    }
    while let Ok((n, src)) = bsock.recv_from(buf) {
        let _ = b.handle_datagram(src, &buf[..n]);
    }
}

fn main() {
    // Two peers, each on a real loopback UDP socket (single device; a real call
    // puts these on two phones over the LAN).
    let asock = UdpSocket::bind("127.0.0.1:0").expect("bind A");
    let bsock = UdpSocket::bind("127.0.0.1:0").expect("bind B");
    asock.set_nonblocking(true).unwrap();
    bsock.set_nonblocking(true).unwrap();
    let a_addr: SocketAddr = asock.local_addr().unwrap();
    let b_addr: SocketAddr = bsock.local_addr().unwrap();
    let mut a = PeerSession::new(Role::Offerer, a_addr).expect("A");
    let mut b = PeerSession::new(Role::Answerer, b_addr).expect("B");

    // Signaling (SDP offer/answer) — A is the caller, B the callee.
    let offer = a.local_signaling().to_sdp();
    let answer = b.local_signaling().to_sdp();
    a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
    b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();
    println!("[calllive] signaling exchanged; connecting over real UDP…");

    // Connect: ICE connectivity + DTLS-SRTP handshake over the sockets.
    let mut buf = [0u8; 2048];
    let mut connected = false;
    for _ in 0..1500 {
        pump(&mut a, &asock, &mut b, &bsock, &mut buf);
        if a.is_connected() && b.is_connected() {
            connected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    if !connected {
        println!("[calllive] FAILED to connect");
        std::process::exit(1);
    }
    println!("[calllive] CONNECTED — ICE + DTLS-SRTP over real UDP, fingerprint verified");

    // ---- Phase 1: mic → A ─(SRTP over UDP)→ B → buffer ----
    let cap_cfg = TrackConfig {
        sample_rate: SAMPLE_RATE,
        channel_layout: ChannelLayout::Mono,
        format: Format::PcmF32,
    };
    let cap = audio::open_capture(cap_cfg);
    if cap == 0 {
        println!("[calllive] open-capture failed");
        std::process::exit(1);
    }
    audio::start(cap);
    println!("[calllive] on the call — speak now (~3 s)… mic → A ─UDP→ B");

    let mut pending: Vec<f32> = Vec::new();
    let mut out: Vec<f32> = Vec::new();
    let mut sent = 0usize;
    let target = 150; // ~3 s
    let deadline = Instant::now() + Duration::from_secs(6);
    while sent < target && Instant::now() < deadline {
        let chunk = audio::read_pcm_f32(cap, FRAME as u32);
        if !chunk.is_empty() {
            pending.extend_from_slice(&chunk);
        }
        while pending.len() >= FRAME {
            let frame: Vec<f32> = pending.drain(..FRAME).collect();
            let _ = a.send_audio(&frame); // A: Opus → RTP → SRTP, queued for UDP
            sent += 1;
        }
        pump(&mut a, &asock, &mut b, &bsock, &mut buf);
        while let Some(pcm) = b.recv_audio() {
            out.extend_from_slice(&pcm); // B: decrypted + decoded over the wire
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // Flush any media still in flight across the socket.
    for _ in 0..50 {
        pump(&mut a, &asock, &mut b, &bsock, &mut buf);
        while let Some(pcm) = b.recv_audio() {
            out.extend_from_slice(&pcm);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    audio::close(cap);
    println!("[calllive] {sent} frames A→B over UDP; B decoded {} samples from the wire", out.len());
    if out.is_empty() {
        println!("[calllive] no audio crossed the wire — aborting");
        std::process::exit(1);
    }

    // ---- Phase 2: play B's wire-decoded audio out the speaker ----
    // Stereo (this device's MMAP output is stereo-only); amplify (mic near noise
    // floor); write-then-start (prime the DMA ring before start, else silence).
    let trk_cfg = TrackConfig {
        sample_rate: SAMPLE_RATE,
        channel_layout: ChannelLayout::Stereo,
        format: Format::PcmF32,
    };
    let trk = audio::create_track(trk_cfg);
    if trk == 0 {
        println!("[calllive] create-track failed");
        std::process::exit(1);
    }
    let peak = out.iter().fold(0f32, |m, &v| m.max(v.abs()));
    let gain = if peak > 1e-6 { (0.3 / peak).min(30.0) } else { 1.0 };
    let all: Vec<f32> =
        out.iter().flat_map(|&s| { let x = (s * gain).clamp(-1.0, 1.0); [x, x] }).collect();
    println!("[calllive] playing what B received ×{gain:.1} (peak {peak:.4}) — listen…");

    let mut i = 0;
    while i < all.len() {
        let end = (i + FRAME * 2).min(all.len());
        let wrote = audio::write_pcm_f32(trk, &all[i..end]) as usize;
        if wrote == 0 {
            break; // ring primed
        }
        i += wrote * 2;
    }
    audio::start(trk);
    let play = Instant::now() + Duration::from_secs(15);
    while i < all.len() && Instant::now() < play {
        let end = (i + FRAME * 2).min(all.len());
        let wrote = audio::write_pcm_f32(trk, &all[i..end]) as usize;
        if wrote == 0 {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        i += wrote * 2;
    }
    let drain = Instant::now() + Duration::from_secs(5);
    while audio::pending_frames(trk) > 0 && Instant::now() < drain {
        std::thread::sleep(Duration::from_millis(20));
    }
    audio::close(trk);

    println!("[calllive] DONE — live call: mic → ICE/DTLS-SRTP/UDP → speaker, on real hardware");
}
