//! Decode a real H.264 MP4 through wandr-video's openh264 backend and prove the
//! full file-playback path (task 117 M2 step 2b).
//!
//! Pipeline:
//!   mp4 demux
//!   → avcC-to-Annex-B  (the `h264_mp4toannexb` conversion: length-prefixed NALs
//!                        → start-code NALs, SPS/PPS prepended at each keyframe)
//!   → wandr_video decode
//!   → reorder buffer   (decode order → presentation order, for B-frames)
//!
//! This answers what the in-crate round-trip test cannot: does the backend decode
//! a REAL file — real SPS/PPS, real GOPs, B-frames — and does the container PTS
//! come out usable? Findings (bbb-h264.mp4): yes, 300/300, but openh264 emits in
//! DECODE order so presentation needs a small reorder buffer. It also surfaced a
//! real backend bug — openh264's default flush-after-every-decode overflows the
//! reorder buffer on the 2nd GOP; the backend now opens with `Flush::NoFlush`.

use std::io::{BufReader, Write};

use mp4::Mp4Reader;
use wandr_video::{open_decoder, Chunk, Codec, DecoderParams};

const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Convert one MP4 sample (length-prefixed NAL units) to Annex-B, prepending
/// SPS+PPS when `keyframe` (a decoder started mid-stream needs them each IDR).
fn to_annex_b(sample: &[u8], sps: &[u8], pps: &[u8], keyframe: bool, len_size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(sample.len() + 64);
    if keyframe {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(sps);
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(pps);
    }
    let mut i = 0;
    while i + len_size <= sample.len() {
        let mut n = 0usize; // big-endian NAL length
        for b in &sample[i..i + len_size] {
            n = (n << 8) | *b as usize;
        }
        i += len_size;
        if i + n > sample.len() {
            break;
        }
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(&sample[i..i + n]);
        i += n;
    }
    out
}

/// A bounded reorder buffer: hold up to `depth` frames, then always emit the
/// smallest PTS. Turns decode order into presentation order for a B-frame stream
/// — presentation policy that belongs in the player, not the codec.
fn reorder(decode_order: &[i64], depth: usize) -> Vec<i64> {
    let mut win: Vec<i64> = Vec::new();
    let mut out = Vec::with_capacity(decode_order.len());
    for &pts in decode_order {
        win.push(pts);
        if win.len() > depth {
            let (mi, _) = win.iter().enumerate().min_by_key(|(_, p)| **p).unwrap();
            out.push(win.remove(mi));
        }
    }
    win.sort_unstable();
    out.extend(win);
    out
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../oxideav-spike/samples/bbb-h264.mp4".to_string());
    eprintln!("h264-mp4-decode: {path}");

    let f = std::fs::File::open(&path).expect("open mp4");
    let size = f.metadata().unwrap().len();
    let mut mp4 = Mp4Reader::read_header(BufReader::new(f), size).expect("parse mp4");

    let (track_id, timescale, sps, pps) = {
        let track = mp4
            .tracks()
            .values()
            .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H264)))
            .expect("no H.264 track");
        (
            track.track_id(),
            track.timescale() as i64,
            track.sequence_parameter_set().expect("sps").to_vec(),
            track.picture_parameter_set().expect("pps").to_vec(),
        )
    };
    let n = mp4.tracks()[&track_id].sample_count();
    eprintln!("track {track_id}: {n} samples, timescale {timescale}");

    let mut dec = open_decoder(&DecoderParams { codec: Codec::H264, width: 0, height: 0 })
        .expect("open h264 decoder");

    // Feed in FILE order (= decode order). True presentation time is
    // start_time + rendering_offset (DTS + the ctts CTS-DTS delta).
    let ts_to_us = |t: i64| t * 1_000_000 / timescale;
    let (mut fed, mut errs, mut any_bframe) = (0u64, 0u64, false);
    let mut out_pts: Vec<i64> = Vec::new();
    let mut dims = (0u32, 0u32);

    for sid in 1..=n {
        let s = mp4.read_sample(track_id, sid).expect("read").expect("sample");
        any_bframe |= s.rendering_offset != 0;
        let pts_us = ts_to_us(s.start_time as i64 + s.rendering_offset as i64);
        let annexb = to_annex_b(&s.bytes, &sps, &pps, s.is_sync, 4);
        fed += 1;
        match dec.decode(Chunk::new(&annexb, pts_us)) {
            Ok(()) => {
                while let Some(fr) = dec.next_frame() {
                    dims = (fr.width, fr.height);
                    out_pts.push(fr.timestamp_us);
                }
            }
            Err(_) => errs += 1,
        }
    }
    dec.flush().ok();
    while let Some(fr) = dec.next_frame() {
        out_pts.push(fr.timestamp_us);
    }

    let decoded = out_pts.len();
    let raw_monotonic = out_pts.windows(2).all(|w| w[1] >= w[0]);
    let reordered = reorder(&out_pts, 4);
    let reorder_monotonic = reordered.windows(2).all(|w| w[1] >= w[0]);

    println!("──────── RESULT ────────");
    println!("fed {fed} samples, {errs} decode errors, decoded {decoded} frames at {}x{}", dims.0, dims.1);
    println!("container has B-frames (ctts != 0): {any_bframe}");
    println!("decoder output monotonic (raw): {raw_monotonic}   after depth-4 reorder: {reorder_monotonic}");

    let ok = errs == 0
        && decoded == fed as usize
        && any_bframe
        && !raw_monotonic
        && reorder_monotonic
        && reordered.len() == out_pts.len();
    println!(
        "VERDICT: {}",
        if ok {
            "real MP4 decodes fully; decode order needs a reorder buffer; depth-4 makes presentation monotonic with no loss."
        } else {
            "FAIL — see numbers above."
        }
    );
    let _ = std::io::stdout().flush();
    std::process::exit(if ok { 0 } else { 1 });
}
