//! Cross-implementation interop: does wart-call speak real WebRTC?
//!
//! The webrtc-rs async `webrtc` crate (a separate codebase — its own
//! webrtc-ice/webrtc-dtls) is the offerer; wart-call is the answerer. They
//! connect over real localhost UDP. If they reach `Connected`, wart-call's
//! SDP/ICE/DTLS interoperate with an independent WebRTC stack — the scriptable
//! proxy for a browser.

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;

use wart_call::signaling::Signaling;
use wart_call::{PeerSession, Role};

#[tokio::main]
async fn main() -> Result<()> {
    // ── the independent peer: a full async RTCPeerConnection (the offerer) ──
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();
    let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    // One audio m-line (so the pc negotiates audio, matching wart-call).
    pc.add_transceiver_from_kind(
        RTPCodecType::Audio,
        Some(RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Sendrecv, send_encodings: vec![] }),
    )
    .await?;

    let pc_connected = Arc::new(AtomicBool::new(false));
    {
        let f = pc_connected.clone();
        pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
            println!("[interop] webrtc-rs pc state → {s}");
            if s == RTCPeerConnectionState::Connected {
                f.store(true, Ordering::SeqCst);
            }
            Box::pin(async {})
        }));
    }

    // Offer, then wait for non-trickle ICE gathering so candidates are in the SDP.
    let offer = pc.create_offer(None).await?;
    let mut gather = pc.gathering_complete_promise().await;
    pc.set_local_description(offer).await?;
    let _ = gather.recv().await;
    let offer_sdp = pc.local_description().await.expect("local desc").sdp;
    println!("[interop] webrtc-rs offer ({} bytes) generated + candidates gathered", offer_sdp.len());

    // ── wart-call: parse the real offer, answer ──
    // Advertise the LAN IP (the pc excludes loopback from its host candidates).
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_nonblocking(true)?;
    let ip = wart_call::local_lan_ip().unwrap_or(std::net::IpAddr::from([127, 0, 0, 1]));
    let wc_addr = SocketAddr::new(ip, sock.local_addr()?.port());
    println!("[interop] wart-call advertises {wc_addr}");
    let mut a = PeerSession::new(Role::Answerer, wc_addr)?;

    let remote_sig = Signaling::from_sdp(&offer_sdp)?;
    println!(
        "[interop] wart-call parsed offer: ufrag={} setup={} cand={:?}",
        remote_sig.ice_ufrag, remote_sig.setup, remote_sig.candidates.first()
    );
    a.set_remote_signaling(&remote_sig)?;
    let answer_sdp = a.local_signaling().to_sdp();

    pc.set_remote_description(RTCSessionDescription::answer(answer_sdp)?).await?;
    println!("[interop] wart-call answer accepted by webrtc-rs pc");

    // ── drive wart-call (sync) in a thread over its real UDP socket ──
    let wc_connected = Arc::new(AtomicBool::new(false));
    {
        let flag = wc_connected.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            for _ in 0..4000 {
                let now = Instant::now();
                a.handle_timeout(now);
                for (dest, data) in a.poll_transmit() {
                    let _ = sock.send_to(&data, dest);
                }
                while let Ok((n, src)) = sock.recv_from(&mut buf) {
                    let _ = a.handle_datagram(src, &buf[..n]);
                }
                if a.is_connected() {
                    flag.store(true, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(3));
            }
        });
    }

    // ── wait for both to connect (or time out) ──
    for _ in 0..200 {
        if pc_connected.load(Ordering::SeqCst) && wc_connected.load(Ordering::SeqCst) {
            println!();
            println!("INTEROP OK — wart-call connected to the webrtc-rs async stack (ICE + DTLS-SRTP over UDP)");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!();
    println!(
        "INTEROP INCOMPLETE — pc_connected={} wart_call_connected={}",
        pc_connected.load(Ordering::SeqCst),
        wc_connected.load(Ordering::SeqCst)
    );
    std::process::exit(1);
}
