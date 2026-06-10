//! wandr.video.test — a wasi:cli command guest that drives `wandr:video` over the
//! WIT boundary.
//!
//! **Part 1 (task 93 Phase 1)**: host opens the camera + HW VP8 encoder, the guest
//! pulls encoded frames and pushes each into a host HW VP8 decoder — the spike's
//! loopback through the real interface. Resources are dropped at the end so Part 2's
//! re-open also proves clean teardown (no cameraserver wedge) within one run.
//!
//! **Part 2 (task 93 Phase 3)**: the same camera frames through the wandr-call
//! video track: encoder → `PeerSession` A `send_video` → VP8 RTP + SRTP over real
//! `wasi:sockets` UDP (loopback) → `PeerSession` B `recv_video` → reassembled
//! frames → HW decoder. Plus the RTCP control loop: B `request_keyframe` → PLI on
//! the wire → A `take_keyframe_request` → encoder `request-keyframe`, and A's ~1 Hz
//! video sender report landing as B's A/V-sync anchor.

wit_bindgen::generate!({
    world: "video-test",
    path: "wit",
    generate_all,
});

use crate::wandr::video::decoder::VideoDecoder;
use crate::wandr::video::encoder::VideoEncoder;
use crate::wandr::video::types::{
    CameraFacing, Codec, DecoderConfig, EncoderConfig, VideoError, VideoRect,
};
use std::net::UdpSocket;
use std::time::{Duration, Instant};
use wandr_call::signaling::Signaling;
use wandr_call::{PeerSession, Role};

const W: u32 = 640;
const H: u32 = 480;

/// `preview` = PiP self-view rect (Phase 4); `None` = encode-only.
fn open_encoder(preview: Option<VideoRect>) -> Option<VideoEncoder> {
    let has_preview = preview.is_some();
    match VideoEncoder::open(EncoderConfig {
        codec: Codec::Vp8,
        width: W,
        height: H,
        bitrate_bps: 1_000_000,
        framerate: 30,
        source_camera: true,
        // Front camera (faces up on a desk — the visual A/B for camera content;
        // the call self-view wants front anyway).
        facing: CameraFacing::Front,
        preview,
    }) {
        Ok(e) => {
            println!("encoder OPEN ✓ ({W}x{H} VP8, camera live{})",
                if has_preview { ", PiP preview on-screen" } else { "" });
            Some(e)
        }
        Err(e) => {
            println!("FAIL: encoder open: {e:?} (stubs/sensormanager running? wandr-sensors stopped?)");
            None
        }
    }
}

/// An empty `rect` = decode-to-buffer (Phase 1 diagnostics); a real one =
/// decode-to-surface — the host composites the video on the panel (Phase 4).
/// `rotation` = peer CVO degrees CW for upright display (Phase 5).
fn open_decoder(rect: VideoRect, rotation: u32) -> Option<VideoDecoder> {
    let to_surface = rect.width > 0 && rect.height > 0;
    match VideoDecoder::open(DecoderConfig { codec: Codec::Vp8, width: W, height: H, rect, rotation }) {
        Ok(d) => {
            println!("decoder OPEN ✓ ({}, rotation={rotation}°)",
                if to_surface { "decode-to-SURFACE, on-screen" } else { "decode-to-buffer" });
            Some(d)
        }
        Err(e) => {
            println!("FAIL: decoder open: {e:?}");
            None
        }
    }
}

/// Part 1 — direct WIT loopback (Phase 1 regression): encoder → guest → decoder.
/// Returns pass/fail. Drops its encoder+decoder on return (teardown exercise).
fn part1_direct_loopback() -> bool {
    const RUN_SECS: u64 = 3;
    println!("── Part 1: direct WIT loopback (camera → HW VP8 → guest → HW decode) ──");
    let Some(enc) = open_encoder(None) else { return false };
    // Empty rect → decode-to-buffer: keeps Part 1 the pure Phase-1 regression.
    let Some(dec) = open_decoder(VideoRect { x: 0, y: 0, width: 0, height: 0 }, 0) else { return false };

    let start = Instant::now();
    let (mut pulled, mut bytes, mut keyframes, mut submitted, mut queue_full, mut other_err) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut first_ms: i128 = -1;
    while start.elapsed().as_secs() < RUN_SECS {
        if let Some(frame) = enc.next_frame() {
            if first_ms < 0 {
                first_ms = start.elapsed().as_millis() as i128;
            }
            pulled += 1;
            bytes += frame.data.len() as u64;
            if frame.keyframe {
                keyframes += 1;
            }
            match dec.submit(&frame) {
                Ok(()) => submitted += 1,
                Err(VideoError::QueueFull) => queue_full += 1,
                Err(e) => {
                    other_err += 1;
                    println!("   submit error: {e:?}");
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let secs = start.elapsed().as_secs_f64();
    let decoded = dec.decoded_frames();
    println!("   encoded {pulled} ({:.1} fps, avg {} B, {keyframes} kf, first {first_ms} ms)",
        pulled as f64 / secs, if pulled > 0 { bytes / pulled } else { 0 });
    println!("   submitted {submitted} (queue-full {queue_full}, err {other_err}) → decoded {decoded}");
    let ok = pulled > 0 && decoded > 0 && other_err == 0;
    println!("   Part 1: {}", if ok { "PASS" } else { "FAIL" });
    ok
    // enc + dec drop here — ordered host teardown; Part 2 re-opens the camera.
}

/// Part 2 — the wandr-call video track (Phase 3): camera → encoder → PeerSession A
/// → SRTP/UDP → PeerSession B → reassembled frames → decoder; PLI both ways.
fn part2_engine_pipeline() -> bool {
    const RUN_SECS: u64 = 10;
    println!("── Part 2: wandr-call video track (encoder → SRTP/UDP → decoder), ON-SCREEN ──");
    // Phase 4 visual: remote video decode-to-surface in a 2×-scaled rect
    // (proves the buffer→rect scaling matrix) + the camera PiP self-view
    // bottom-right. Test rects in taimen panel pixels (this is a headless
    // diagnostic — the surfaces are top-level; a real call app's rects are
    // its own surface coordinates).
    let Some(enc) = open_encoder(Some(VideoRect { x: 1020, y: 1840, width: 360, height: 270 }))
    else { return false };
    // Phase 5: the decoder opens LAZILY on the first received keyframe, with
    // that frame's CVO rotation — the exact pattern a real call uses (the
    // peer's rotation is unknown until its video arrives).
    let mut dec: Option<VideoDecoder> = None;
    let dec_rect = VideoRect { x: 80, y: 700, width: 1280, height: 960 };

    // Two engine peers over real wasi:sockets UDP on loopback (the call-live pattern).
    let asock = UdpSocket::bind("127.0.0.1:0").expect("bind A");
    let bsock = UdpSocket::bind("127.0.0.1:0").expect("bind B");
    asock.set_nonblocking(true).unwrap();
    bsock.set_nonblocking(true).unwrap();
    let mut a = PeerSession::new(Role::Offerer, asock.local_addr().unwrap()).expect("session A");
    let mut b = PeerSession::new(Role::Answerer, bsock.local_addr().unwrap()).expect("session B");
    let offer = a.local_signaling().to_sdp();
    let answer = b.local_signaling().to_sdp();
    a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
    b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();

    let mut buf = [0u8; 2048];
    let mut pump = |a: &mut PeerSession, asock: &UdpSocket, b: &mut PeerSession, bsock: &UdpSocket| {
        for (dst, dg) in a.poll_transmit() {
            let _ = asock.send_to(&dg, dst);
        }
        for (dst, dg) in b.poll_transmit() {
            let _ = bsock.send_to(&dg, dst);
        }
        while let Ok((n, src)) = asock.recv_from(&mut buf) {
            let _ = a.handle_datagram(src, &buf[..n]);
        }
        while let Ok((n, src)) = bsock.recv_from(&mut buf) {
            let _ = b.handle_datagram(src, &buf[..n]);
        }
    };

    let start = Instant::now();
    while !(a.is_connected() && b.is_connected()) {
        if start.elapsed().as_secs() > 10 {
            println!("FAIL: engine peers did not connect (ICE/DTLS)");
            return false;
        }
        let now = Instant::now();
        a.handle_timeout(now);
        b.handle_timeout(now);
        pump(&mut a, &asock, &mut b, &bsock);
        std::thread::sleep(Duration::from_millis(3));
    }
    println!("   engine peers CONNECTED (ICE + DTLS-SRTP) ✓");

    // Phase 5: A's outgoing CVO = the camera's display rotation; B's frames
    // carry it and the decoder is opened with it → upright on-screen video.
    let cam_rotation = enc.display_rotation();
    a.set_video_rotation(cam_rotation);
    println!("   camera display-rotation {cam_rotation}° → outgoing CVO");

    let start = Instant::now();
    let (mut sent, mut rx_frames, mut decoded_fed, mut queue_full, mut other_err) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut pli_requested = false;
    let mut pli_answered = false;
    while start.elapsed().as_secs() < RUN_SECS {
        // Camera → encoder → engine A (the Phase-5 Signal send path).
        if let Some(frame) = enc.next_frame() {
            if a.send_video(&frame.data, frame.timestamp).is_ok() {
                sent += 1;
            }
        }
        pump(&mut a, &asock, &mut b, &bsock);
        // Engine B → decoder (the receive path). Decoder opens on the first
        // KEYFRAME, with that frame's CVO rotation (the real call pattern).
        while let Some(vf) = b.recv_video() {
            rx_frames += 1;
            if dec.is_none() {
                if !vf.keyframe {
                    b.request_keyframe(); // mid-stream join — need a sync point
                    continue;
                }
                dec = open_decoder(dec_rect, vf.rotation);
                if dec.is_none() {
                    return false;
                }
            }
            let f = crate::wandr::video::types::EncodedFrame {
                data: vf.data,
                timestamp: vf.timestamp,
                keyframe: vf.keyframe,
            };
            match dec.as_ref().unwrap().submit(&f) {
                Ok(()) => decoded_fed += 1,
                Err(VideoError::QueueFull) => queue_full += 1,
                Err(e) => {
                    other_err += 1;
                    println!("   submit error: {e:?}");
                }
            }
        }
        // RTCP control loop: B asks for a keyframe mid-run → PLI crosses the
        // wire → A surfaces it → encoder request-keyframe (the PLI handler).
        if !pli_requested && start.elapsed().as_millis() > (RUN_SECS as u128 * 500) {
            pli_requested = true;
            b.request_keyframe();
            // Phase 4 surface-control smoke, mid-run on live video: move +
            // shrink the remote rect and nudge the PiP — visibly jumps on
            // the panel and proves set-rect/set-preview-rect on a hot codec.
            if let Some(d) = &dec {
                d.set_rect(VideoRect { x: 240, y: 950, width: 960, height: 720 });
            }
            enc.set_preview_rect(VideoRect { x: 60, y: 1840, width: 360, height: 270 });
            println!("   mid-run: B sent PLI + remote rect moved + PiP moved");
        }
        if a.take_keyframe_request() {
            enc.request_keyframe();
            pli_answered = true;
        }
        let now = Instant::now();
        a.handle_timeout(now);
        b.handle_timeout(now);
        std::thread::sleep(Duration::from_millis(5));
    }
    let secs = start.elapsed().as_secs_f64();
    let decoded = dec.as_ref().map(|d| d.decoded_frames()).unwrap_or(0);
    let ((_, tx_pkts, rx_pkts, _, rx_broken), _, _, rtcp_err) = b.video_diag();

    println!("──────── Part 2 RESULT ────────");
    println!("frames cam→A   : {sent} in {secs:.1}s = {:.1} fps", sent as f64 / secs);
    println!("A tx RTP pkts  : {} (fragmented + SRTP)", { let ((_, tx, ..), ..) = a.video_diag(); tx });
    println!("B rx RTP pkts  : {rx_pkts}, frames reassembled {rx_frames}, broken {rx_broken}");
    println!("decoder fed    : {decoded_fed} (queue-full {queue_full}, err {other_err}) → decoded {decoded} = {:.1} fps", decoded as f64 / secs);
    println!("PLI round trip : requested={pli_requested} answered={pli_answered} (rtcp_err {rtcp_err})");
    println!("B peer SR      : {:?} (A/V-sync anchor from A)", b.peer_sender_report());
    let _ = tx_pkts;

    let ok = sent > 0 && rx_frames > 0 && decoded > 0 && other_err == 0 && pli_answered;
    println!("   Part 2: {}", if ok { "PASS" } else { "FAIL" });

    // Visual-inspection hold: keep the live video + PiP on the panel (camera
    // still streaming, decoder still draining via submit-free renders) so a
    // screenshot/dumpsys can be taken deterministically after the result.
    println!("   holding surfaces 30s for visual inspection …");
    let hold = Instant::now();
    while hold.elapsed().as_secs() < 30 {
        if let Some(frame) = enc.next_frame() {
            let _ = a.send_video(&frame.data, frame.timestamp);
        }
        pump(&mut a, &asock, &mut b, &bsock);
        while let Some(vf) = b.recv_video() {
            let f = crate::wandr::video::types::EncodedFrame {
                data: vf.data,
                timestamp: vf.timestamp,
                keyframe: vf.keyframe,
            };
            if let Some(d) = &dec {
                let _ = d.submit(&f);
            }
        }
        let now = Instant::now();
        a.handle_timeout(now);
        b.handle_timeout(now);
        std::thread::sleep(Duration::from_millis(5));
    }
    ok
}

fn main() {
    println!("=== wandr.video.test — wandr:video WIT + wandr-call video track ===");
    let p1 = part1_direct_loopback();
    let p2 = part2_engine_pipeline();
    println!(
        "=== RESULT: {} — Phase 1 {} | Phase 3 {} ===",
        if p1 && p2 { "ALL PASS" } else { "FAILURES" },
        if p1 { "ok" } else { "FAIL" },
        if p2 { "ok" } else { "FAIL" },
    );
}
