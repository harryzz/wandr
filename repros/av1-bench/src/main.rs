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

    println!("\nrealtime bar = 30 fps");
    println!("dav1d is BSD-2 (permissive) — AV1 is licence-cleaner than HEVC's LGPL libde265.");
}
