//! wandr.video.test — a wasi:cli command guest that drives `wandr:video` over the
//! WIT boundary (task 93 Phase 1 end-to-end test): host opens the camera + HW VP8
//! encoder, the guest pulls encoded frames (the Phase-3 RTP path's read side) and
//! pushes each one into a host HW VP8 decoder (the write side) — the spike's
//! loopback, now through the real interface. Exits cleanly so the host's ordered
//! camera/codec teardown runs (run twice back-to-back to prove cameraserver
//! doesn't wedge).

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
use std::time::{Duration, Instant};

const W: u32 = 640;
const H: u32 = 480;
const RUN_SECS: u64 = 5;

fn main() {
    println!("=== wandr.video.test — guest -> wandr:video host (camera -> HW VP8 -> guest -> HW decode) ===");

    let enc = match VideoEncoder::open(EncoderConfig {
        codec: Codec::Vp8,
        width: W,
        height: H,
        bitrate_bps: 1_000_000,
        framerate: 30,
        source_camera: true,
        // Back camera = the probe-proven baseline; the call self-view will use front.
        facing: CameraFacing::Back,
    }) {
        Ok(e) => {
            println!("encoder OPEN ✓ ({W}x{H} VP8, camera live)");
            e
        }
        Err(e) => {
            println!("FAIL: encoder open: {e:?} (stubs/sensormanager running? wandr-sensors stopped?)");
            return;
        }
    };

    let dec = match VideoDecoder::open(DecoderConfig {
        codec: Codec::Vp8,
        width: W,
        height: H,
        rect: VideoRect { x: 0, y: 0, width: W, height: H },
    }) {
        Ok(d) => {
            println!("decoder OPEN ✓ (decode-to-buffer)");
            d
        }
        Err(e) => {
            println!("FAIL: decoder open: {e:?}");
            return;
        }
    };

    let start = Instant::now();
    let mut pulled = 0u64;
    let mut bytes = 0u64;
    let mut keyframes = 0u64;
    let mut first_ms: i128 = -1;
    let mut submitted = 0u64;
    let mut queue_full = 0u64;
    let mut other_err = 0u64;
    let mut midrun_done = false;

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
        // Mid-run control smoke: congestion-adapt + PLI-style keyframe request.
        if !midrun_done && start.elapsed().as_millis() > (RUN_SECS as u128 * 500) {
            midrun_done = true;
            enc.set_bitrate(500_000);
            enc.request_keyframe();
            println!("   mid-run: set-bitrate(500k) + request-keyframe sent");
        }
    }
    let secs = start.elapsed().as_secs_f64();
    let decoded = dec.decoded_frames();

    println!("──────── RESULT ────────");
    println!("encoded frames : {pulled} in {secs:.1}s = {:.1} fps", pulled as f64 / secs);
    println!("avg frame bytes: {}", if pulled > 0 { bytes / pulled } else { 0 });
    println!("keyframes      : {keyframes}");
    println!("first-frame    : {first_ms} ms");
    println!("submitted      : {submitted} (queue-full {queue_full}, other-err {other_err})");
    println!("decoded frames : {decoded} = {:.1} fps  (ready={})", decoded as f64 / secs, dec.ready());

    let ok = pulled > 0 && decoded > 0 && other_err == 0;
    println!(
        "=== RESULT: {} — wandr:video WIT boundary {} ===",
        if ok { "PASS" } else { "FAIL" },
        if ok { "works end-to-end (camera->encode->guest->decode)" } else { "has failures" }
    );

    // Explicit drops so the host teardown logs line up with the exit.
    drop(dec);
    drop(enc);
    println!("resources dropped — host should log encoder/decoder teardown");
}
