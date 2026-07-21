use std::fs::File;
use std::time::Instant;
use matroska_demuxer::{MatroskaFile, Frame, TrackType};

/// Demux the AV1 track from a .webm into per-frame OBU byte vectors.
fn load_av1(path: &str) -> Vec<Vec<u8>> {
    let mut mkv = MatroskaFile::open(File::open(path).unwrap()).unwrap();
    let track = mkv.tracks().iter()
        .find(|t| t.track_type() == TrackType::Video && t.codec_id().contains("AV1"))
        .expect("no AV1 track");
    let tid = track.track_number().get();
    let priv_ = track.codec_private().map(|p| p.to_vec()).unwrap_or_default();
    eprintln!("AV1 CodecPrivate: {} bytes", priv_.len());
    let mut frames = Vec::new();
    let mut f = Frame::default();
    let mut first = true;
    while mkv.next_frame(&mut f).unwrap() {
        if f.track == tid {
            let mut d = Vec::new();
            // AV1 CodecPrivate = AV1CodecConfigurationRecord; bytes[4..] is the
            // seq-header OBU(s). Prepend to the first frame.
            if first && priv_.len() > 4 { d.extend_from_slice(&priv_[4..]); first = false; }
            d.extend_from_slice(&f.data);
            frames.push(d);
        }
    }
    frames
}

fn bench_oxideav(frames: &[Vec<u8>]) -> (usize, f64, String) {
    let mut all = Vec::new();
    for fr in frames { all.extend_from_slice(fr); }
    let t = Instant::now();
    match oxideav_av1::decode_av1(&all) {
        Ok(v) => (v.len(), t.elapsed().as_secs_f64(), String::new()),
        Err(e) => (0, 0.0, format!("{e:?}")),
    }
}

fn bench_dav1d(frames: &[Vec<u8>]) -> (usize, f64) {
    let mut dec = dav1d::Decoder::new().unwrap();
    let t = Instant::now();
    let mut n = 0;
    for (i, fr) in frames.iter().enumerate() {
        // send_data wants an owned, Send buffer.
        loop {
            match dec.send_data(fr.clone(), Some(i as i64), None, None) {
                Ok(()) => break,
                Err(e) if e.is_again() => { while dec.get_picture().is_ok() { n += 1; } }
                Err(_) => break,
            }
        }
        while dec.get_picture().is_ok() { n += 1; }
    }
    dec.flush();
    while dec.get_picture().is_ok() { n += 1; }
    (n, t.elapsed().as_secs_f64())
}

/// Decode through the WIRED wandr-video backend — the exact path the host uses:
/// `open_decoder(Av1)` routes to `Dav1dBackend` via the registry, and each frame
/// comes back as tightly-packed I420 carrying its native PTS. Asserts on
/// dimensions and PTS so a broken repack or a dropped timestamp fails loudly.
/// `frame_pts` is the microsecond PTS we feed per frame (native pass-through).
fn verify_backend(frames: &[Vec<u8>], frame_pts: &[i64]) {
    use wandr_video::{open_decoder, Chunk, Codec, DecoderParams};
    let mut dec = open_decoder(&DecoderParams { codec: Codec::Av1, width: 0, height: 0 })
        .expect("open_decoder(Av1) — registry must route to dav1d");
    let mut out_pts = Vec::new();
    let mut dims = None;
    let mut push = |dec: &mut Box<dyn wandr_video::Decoder>| {
        while let Some(f) = dec.next_frame() {
            assert!(f.width > 0 && f.height > 0, "zero-sized AV1 frame");
            assert_eq!(f.y.len(), (f.width * f.height) as usize, "Y plane not tightly packed");
            dims.get_or_insert((f.width, f.height));
            out_pts.push(f.timestamp_us);
        }
    };
    for (fr, &pts) in frames.iter().zip(frame_pts) {
        dec.decode(Chunk::new(fr, pts)).expect("dav1d decode");
        push(&mut dec);
    }
    dec.flush().expect("flush");
    push(&mut dec);

    let (w, h) = dims.expect("no frames decoded through the backend");
    assert_eq!(out_pts.len(), frames.len(), "backend dropped frames: {} of {}", out_pts.len(), frames.len());
    // dav1d outputs display order; PTS must come back sorted and every value we fed
    // must be present (native pass-through, no FIFO desync).
    let mut sorted = frame_pts.to_vec();
    sorted.sort_unstable();
    assert_eq!(out_pts, sorted, "PTS not preserved / not in display order");
    println!("backend (open_decoder→dav1d): {} frames {w}x{h}, PTS preserved ✅", out_pts.len());
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or("../oxideav-spike/samples/bbb-av1.webm".into());
    let frames = load_av1(&path);
    println!("AV1 decode benchmark — {} frames from {path}\n", frames.len());

    let (f1, t1, err) = bench_oxideav(&frames);
    if f1 > 0 {
        println!("oxideav-av1 (pure-Rust): {f1} frames in {t1:.2}s = {:.1} fps", f1 as f64 / t1);
    } else {
        println!("oxideav-av1 (pure-Rust): FAILED on matroska AV1 framing ({err}) — needs oxideav's own demuxer");
    }

    let (f2, t2) = bench_dav1d(&frames);
    println!("dav1d       (C):         {f2} frames in {t2:.2}s = {:.1} fps", f2 as f64 / t2);

    // Prove the wired path, with a realistic 30fps PTS ladder in microseconds.
    let frame_pts: Vec<i64> = (0..frames.len() as i64).map(|i| i * 1_000_000 / 30).collect();
    verify_backend(&frames, &frame_pts);

    println!("\nrealtime bar = 30 fps");
    println!("dav1d is BSD-2 (permissive) — AV1 is licence-cleaner than HEVC's LGPL libde265.");
}
