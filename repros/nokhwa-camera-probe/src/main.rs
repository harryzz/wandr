//! Desktop camera+codec probe (task 108 / video roadmap): nokhwa capture →
//! ffmpeg VP8 encode (libvpx) → ffmpeg VP8 decode → count. De-risks the pieces
//! that fill runtime/wandr-host/src/video.rs's desktop VideoEncoder/VideoDecoder
//! BEFORE touching the host build. See project_desktop_audio_cpal (cpal analog).

use ffmpeg_next as ffmpeg;
use ffmpeg::format::Pixel;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg::util::frame::video::Video as VideoFrame;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};
use nokhwa::Camera;
use std::time::Instant;

const W: u32 = 640;
const H: u32 = 480;
const FPS: u32 = 30;

/// Copy tightly-packed RGB24 (nokhwa) into an ffmpeg RGB24 frame, respecting its
/// (aligned) row stride.
fn fill_rgb(frame: &mut VideoFrame, rgb: &[u8]) {
    let w = W as usize;
    let h = H as usize;
    let stride = frame.stride(0);
    let data = frame.data_mut(0);
    for y in 0..h {
        let s = &rgb[y * w * 3..y * w * 3 + w * 3];
        data[y * stride..y * stride + w * 3].copy_from_slice(s);
    }
}

fn main() {
    ffmpeg::init().expect("ffmpeg init");

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

    // ── VP8 encoder (libvpx) ───────────────────────────────────────────────
    let enc_codec = ffmpeg::encoder::find_by_name("libvpx").expect("libvpx encoder");
    let mut enc_ctx = ffmpeg::codec::context::Context::new_with_codec(enc_codec)
        .encoder()
        .video()
        .expect("encoder.video");
    enc_ctx.set_width(W);
    enc_ctx.set_height(H);
    enc_ctx.set_format(Pixel::YUV420P);
    enc_ctx.set_time_base((1, FPS as i32));
    enc_ctx.set_bit_rate(1_000_000);
    // Realtime, low-latency VP8 (what a call wants): deadline=realtime, no lag.
    let mut opts = ffmpeg::Dictionary::new();
    opts.set("deadline", "realtime");
    opts.set("lag-in-frames", "0");
    let mut encoder = enc_ctx.open_with(opts).expect("encoder open");

    // ── VP8 decoder ────────────────────────────────────────────────────────
    let dec_codec = ffmpeg::decoder::find(ffmpeg::codec::Id::VP8).expect("VP8 decoder");
    let mut decoder = ffmpeg::codec::context::Context::new_with_codec(dec_codec)
        .decoder()
        .video()
        .expect("decoder.video");

    // ── RGB24 → YUV420P scaler ─────────────────────────────────────────────
    let mut scaler = Scaler::get(
        Pixel::RGB24, W, H,
        Pixel::YUV420P, W, H,
        Flags::BILINEAR,
    ).expect("scaler");

    let mut rgb_frame = VideoFrame::new(Pixel::RGB24, W, H);

    const N: u32 = 60;
    let start = Instant::now();
    let (mut captured, mut encoded, mut enc_bytes, mut keyframes, mut decoded) = (0u64, 0u64, 0u64, 0u64, 0u64);

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
        fill_rgb(&mut rgb_frame, img.as_raw());

        let mut yuv = VideoFrame::empty();
        scaler.run(&rgb_frame, &mut yuv).expect("scale");
        yuv.set_pts(Some(i as i64));

        encoder.send_frame(&yuv).expect("send_frame");
        let mut pkt = ffmpeg::Packet::empty();
        while encoder.receive_packet(&mut pkt).is_ok() {
            encoded += 1;
            enc_bytes += pkt.size() as u64;
            if pkt.is_key() { keyframes += 1; }
            decoder.send_packet(&pkt).expect("send_packet");
            let mut df = VideoFrame::empty();
            while decoder.receive_frame(&mut df).is_ok() { decoded += 1; }
        }
    }
    // Flush encoder → decoder.
    encoder.send_eof().ok();
    let mut pkt = ffmpeg::Packet::empty();
    while encoder.receive_packet(&mut pkt).is_ok() {
        encoded += 1; enc_bytes += pkt.size() as u64;
        decoder.send_packet(&pkt).ok();
        let mut df = VideoFrame::empty();
        while decoder.receive_frame(&mut df).is_ok() { decoded += 1; }
    }
    decoder.send_eof().ok();
    let mut df = VideoFrame::empty();
    while decoder.receive_frame(&mut df).is_ok() { decoded += 1; }

    let secs = start.elapsed().as_secs_f64();
    println!("captured {captured}, encoded {encoded} ({keyframes} kf, avg {} B/frame, {:.1} fps)",
        if encoded > 0 { enc_bytes / encoded } else { 0 }, encoded as f64 / secs);
    println!("decoded  {decoded} frames");
    let ok = captured > 0 && encoded > 0 && decoded > 0;
    println!("RESULT: {}", if ok { "PASS — camera → VP8 → decode works" } else { "FAIL" });
    let _ = camera.stop_stream();
    std::process::exit(if ok { 0 } else { 1 });
}
