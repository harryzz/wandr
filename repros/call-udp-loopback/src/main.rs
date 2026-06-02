//! Device check for wart-call's real UDP transport. Two `PeerSession`s, each on
//! a real loopback `UdpSocket` (→ wasi:sockets in the guest), exchange SDP and
//! drive a full call over the sockets: signaling → ICE → DTLS-SRTP → encrypted
//! Opus media. Proves the engine works over real UDP through wart-host on-device.
//!
//!   wart-host --run-once war.probe.calludp

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use wart_call::signaling::Signaling;
use wart_call::{PeerSession, Role, SAMPLE_RATE};

fn main() {
    let asock = UdpSocket::bind("127.0.0.1:0").expect("bind A");
    let bsock = UdpSocket::bind("127.0.0.1:0").expect("bind B");
    asock.set_nonblocking(true).unwrap();
    bsock.set_nonblocking(true).unwrap();
    let mut a = PeerSession::new(Role::Offerer, asock.local_addr().unwrap()).expect("A");
    let mut b = PeerSession::new(Role::Answerer, bsock.local_addr().unwrap()).expect("B");
    println!("[calludp] A={} B={}", asock.local_addr().unwrap(), bsock.local_addr().unwrap());

    // Signaling exchange (SDP offer/answer).
    let offer = a.local_signaling().to_sdp();
    let answer = b.local_signaling().to_sdp();
    a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
    b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();
    println!("[calludp] signaling exchanged (SDP offer/answer)");

    // Drive over the real sockets until connected.
    let mut buf = [0u8; 2048];
    let mut connected = false;
    for _ in 0..1500 {
        let now = Instant::now();
        a.handle_timeout(now);
        b.handle_timeout(now);
        for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
        for (dest, data) in b.poll_transmit() { let _ = bsock.send_to(&data, dest); }
        while let Ok((n, src)) = asock.recv_from(&mut buf) { let _ = a.handle_datagram(src, &buf[..n]); }
        while let Ok((n, src)) = bsock.recv_from(&mut buf) { let _ = b.handle_datagram(src, &buf[..n]); }
        if a.is_connected() && b.is_connected() { connected = true; break; }
        std::thread::sleep(Duration::from_millis(3));
    }
    if !connected {
        println!("[calludp] FAILED to connect over UDP");
        std::process::exit(1);
    }
    println!("[calludp] connected — ICE + DTLS-SRTP over real UDP, fingerprint verified");

    // A sends an Opus tone over SRTP; B decodes it.
    let frame: Vec<f32> = (0..a.frame_len())
        .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
        .collect();
    a.send_audio(&frame).unwrap();
    for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
    std::thread::sleep(Duration::from_millis(30));
    while let Ok((n, src)) = bsock.recv_from(&mut buf) { let _ = b.handle_datagram(src, &buf[..n]); }
    let got = b.recv_audio().expect("B received audio over UDP");
    let rms = (got.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / got.len() as f64).sqrt();
    println!("[calludp] B decoded {} samples over UDP (rms={rms:.4})", got.len());

    println!("CALL OVER REAL UDP OK — wart-call PeerSession works over wasi:sockets on wasm32-wasip2");
}
