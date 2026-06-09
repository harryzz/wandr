//! Browser interop harness — a tiny HTTP server (serves the test page + relays
//! SDP) with `wandr-call` as the answerer. Open the page in a real browser; the
//! browser offers a recvonly audio call, wandr-call answers, connects (ICE +
//! DTLS-SRTP over real UDP), and streams a 440 Hz Opus tone the browser plays.
//!
//!   cargo run     # then open http://<this-machine-ip>:8088

use std::io::Read;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tiny_http::{Header, Method, Response, Server};

use wandr_call::signaling::Signaling;
use wandr_call::{local_lan_ip, PeerSession, Role, SAMPLE_RATE};

const INDEX_HTML: &str = include_str!("../index.html");
const PORT: u16 = 8088;

fn main() -> Result<()> {
    let server = Server::http(("0.0.0.0", PORT)).map_err(|e| anyhow!("http: {e}"))?;
    let ip = local_lan_ip().unwrap_or(IpAddr::from([127, 0, 0, 1]));
    println!("▸ wandr-call browser harness up");
    println!("▸ open  http://{ip}:{PORT}  in a browser on the same network");
    println!("▸ (if it won't connect, disable Chrome mDNS: chrome://flags/#enable-webrtc-hide-local-ips-with-mdns → Disabled — see README)");

    for mut req in server.incoming_requests() {
        match (req.method(), req.url()) {
            (Method::Get, "/") => {
                let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
                let _ = req.respond(Response::from_string(INDEX_HTML).with_header(header));
            }
            (Method::Post, "/offer") => {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                match answer_offer(&body) {
                    Ok(answer) => {
                        println!("[browser] answered offer ({} → {} bytes); driving call…", body.len(), answer.len());
                        let _ = req.respond(Response::from_string(answer));
                    }
                    Err(e) => {
                        eprintln!("[browser] offer error: {e:#}");
                        let _ = req.respond(Response::from_string(format!("{e:#}")).with_status_code(500));
                    }
                }
            }
            _ => { let _ = req.respond(Response::empty(404)); }
        }
    }
    Ok(())
}

/// Build a wandr-call answer for the browser's offer + spawn the call-driving
/// thread (which streams a tone once connected). Returns the answer SDP.
fn answer_offer(offer_sdp: &str) -> Result<String> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_nonblocking(true)?;
    let ip = local_lan_ip().unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let wc_addr = SocketAddr::new(ip, sock.local_addr()?.port());

    let mut a = PeerSession::new(Role::Answerer, wc_addr)?;
    let remote = Signaling::from_sdp(offer_sdp)?;
    println!("[browser] offer: ufrag={} setup={} candidates={}", remote.ice_ufrag, remote.setup, remote.candidates.len());
    a.set_remote_signaling(&remote)?;
    let answer = a.local_signaling().to_sdp();

    std::thread::spawn(move || {
        let tone: Vec<f32> = (0..a.frame_len())
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.4)
            .collect();
        let mut buf = [0u8; 2048];
        let mut announced = false;
        loop {
            let now = Instant::now();
            a.handle_timeout(now);
            if a.is_connected() {
                if !announced {
                    println!("[browser] ✓ wandr-call CONNECTED to the browser — streaming Opus tone");
                    announced = true;
                }
                let _ = a.send_audio(&tone);
            }
            for (dest, data) in a.poll_transmit() {
                let _ = sock.send_to(&data, dest);
            }
            while let Ok((n, src)) = sock.recv_from(&mut buf) {
                let _ = a.handle_datagram(src, &buf[..n]);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    Ok(answer)
}
