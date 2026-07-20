//! Desktop camera+codec probe (task 108 / video roadmap): nokhwa capture → VP8
//! encode → VP8 decode → count. De-risks the pieces behind
//! runtime/wandr-host/src/video_desktop.rs's VideoEncoder/VideoDecoder.
//!
//! Task 117 ported this off ffmpeg onto `wandr-video` (statically-linked libvpx),
//! so it now exercises exactly the code the host runs — previously it tested a
//! parallel ffmpeg implementation that could drift from the host's.
//!
//! This is the camera → encode → DECODE path. The host's `--video-selfview-test`
//! covers camera → encode → PiP but never decodes, so this probe is the one that
//! proves the decoder against real camera frames rather than synthetic ones.

use std::time::Instant;

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};
use nokhwa::Camera;
use wandr_video::{open_decoder, open_encoder, Codec, DecoderParams, EncoderParams, Rgb24Frame};

const W: u32 = 640;
const H: u32 = 480;
const FPS: u32 = 30;

fn main() {
    // ── camera (nokhwa) ────────────────────────────────────────────────────
    let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
        CameraFormat::new(Resolution::new(W, H), FrameFormat::MJPEG, FPS),
    ));
    let mut camera = Camera::new(CameraIndex::Index(0), requested).expect("Camera::new");
    let fmt = camera.camera_format();
    println!(
        "camera: {}x{} @ {} fps {:?}",
        fmt.resolution().width_x, fmt.resolution().height_y, fmt.frame_rate(), fmt.format()
    );
    camera.open_stream().expect("open_stream");

    // ── VP8 encode + decode (libvpx, static) ───────────────────────────────
    // No scaler setup and no manual stride handling: the crate resizes (only when
    // the camera size differs from the encode size) and converts RGB→I420 straight
    // into libvpx's own planes.
    let mut encoder = open_encoder(&EncoderParams {
        codec: Codec::Vp8,
        width: W,
        height: H,
        bitrate_bps: 1_000_000,
        framerate: FPS,
    })
    .expect("open encoder");
    let mut decoder =
        open_decoder(&DecoderParams { codec: Codec::Vp8, width: W, height: H }).expect("open decoder");

    const N: u32 = 60;
    let start = Instant::now();
    let (mut captured, mut encoded, mut enc_bytes, mut keyframes, mut decoded) =
        (0u64, 0u64, 0u64, 0u64, 0u64);

    for i in 0..N {
        let buf = match camera.frame() {
            Ok(b) => b,
            Err(e) => { eprintln!("frame {i}: capture {e:?}"); continue; }
        };
        let img = match buf.decode_image::<RgbFormat>() {
            Ok(im) => im,
            Err(e) => { eprintln!("frame {i}: decode {e:?}"); continue; }
        };
        captured += 1;
        let (fw, fh) = (img.width(), img.height());

        if let Err(e) = encoder.encode(Rgb24Frame::new(img.as_raw(), fw, fh), i == 0) {
            eprintln!("frame {i}: encode {e:?}");
            continue;
        }
        while let Some(pkt) = encoder.next_packet() {
            encoded += 1;
            enc_bytes += pkt.data.len() as u64;
            if pkt.keyframe {
                keyframes += 1;
            }
            match decoder.decode(&pkt.data) {
                Ok(()) => while decoder.next_frame().is_some() { decoded += 1; },
                Err(e) => eprintln!("frame {i}: decode {e:?}"),
            }
        }
    }

    let secs = start.elapsed().as_secs_f64();
    println!(
        "captured {captured}, encoded {encoded} ({keyframes} kf, avg {} B/frame, {:.1} fps)",
        if encoded > 0 { enc_bytes / encoded } else { 0 },
        encoded as f64 / secs
    );
    println!("decoded  {decoded} frames");
    let ok = captured > 0 && encoded > 0 && decoded > 0;
    println!("RESULT: {}", if ok { "PASS — camera → VP8 → decode works" } else { "FAIL" });
    let _ = camera.stop_stream();
    std::process::exit(if ok { 0 } else { 1 });
}
