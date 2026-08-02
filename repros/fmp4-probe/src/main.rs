//! fMP4 / CMAF demux verification for task 119 Part B (B1).
//!
//! Empirically checks that `oxideav-mp4` 0.0.9 correctly demuxes REAL DASH/CMAF
//! segments (Tears of Steel, lowest video rep + audio rep) — because the `mp4`
//! 0.14 crate's `read_sample()` is BROKEN for fragments (constant sample offset,
//! see reference_mp4_014_fragmented_read_sample_broken). "Compiles for wasm" was
//! already shown; this proves it actually DEMUXES: sane streams (codec + config),
//! packets with monotonic PTS, a leading video keyframe, and non-empty extradata
//! (avcC/esds) for the decoder.
//!
//! Segments were fetched to `samples/` with curl (see the shell history); a CMAF
//! stream = init segment (ftyp+moov) followed by media segments (styp+moof+mdat),
//! and concatenating them yields a valid fragmented file. Run: `cargo run`.

use std::io::Cursor;

use oxideav_container::Demuxer;
use oxideav_core::{NullCodecResolver, ReadSeek};

fn read(p: &str) -> Vec<u8> {
    let full = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), p);
    std::fs::read(&full).unwrap_or_else(|e| panic!("read {full}: {e}"))
}

/// Concatenate init + media segment into one contiguous fragmented-MP4 stream,
/// open the oxideav-mp4 demuxer over it, and report streams + the first packets.
/// Returns (stream_count, first_video_or_audio_packet_seen, keyframe_first,
/// pts_monotonic, extradata_present, total_packets).
fn probe(label: &str, init: &[u8], segs: &[&[u8]]) -> bool {
    println!("\n===== {label} =====");
    let mut buf = Vec::new();
    buf.extend_from_slice(init);
    for s in segs {
        buf.extend_from_slice(s);
    }
    println!("concat init({}) + {} segs = {} bytes", init.len(), segs.len(), buf.len());

    let input: Box<dyn ReadSeek> = Box::new(Cursor::new(buf));
    let mut dmx = match oxideav_mp4::demux::open(input, &NullCodecResolver) {
        Ok(d) => d,
        Err(e) => {
            println!("FAIL: open: {e:?}");
            return false;
        }
    };
    println!("format: {}", dmx.format_name());
    if let Some(us) = dmx.duration_micros() {
        println!("duration: {:.3}s", us as f64 / 1e6);
    }

    // ---- streams ----
    let mut extradata_ok = false;
    let streams: Vec<_> = dmx.streams().to_vec();
    if streams.is_empty() {
        println!("FAIL: no streams");
        return false;
    }
    for s in &streams {
        let p = &s.params;
        println!(
            "stream #{}: {:?} codec={:?} {}x{} sr={:?} ch={:?} extradata={}B tb={:?}",
            s.index, p.media_type, p.codec_id,
            p.width.unwrap_or(0), p.height.unwrap_or(0),
            p.sample_rate, p.channels, p.extradata.len(), s.time_base,
        );
        if !p.extradata.is_empty() {
            extradata_ok = true;
            let n = p.extradata.len().min(16);
            println!("   extradata[0..{n}] = {:02x?}", &p.extradata[..n]);
        }
    }

    // Timescale (ticks per second) for tick→µs conversion in the span check.
    let den = streams[0].time_base.den().max(1);

    // ---- packets ----
    let mut count = 0usize;
    let mut first_keyframe = None;
    let mut first_key_pts: Option<i64> = None;
    let mut last_pts: Option<i64> = None;
    let mut monotonic = true;
    loop {
        match dmx.next_packet() {
            Ok(pkt) => {
                if count < 8 {
                    let n = pkt.data.len().min(12);
                    println!(
                        "pkt #{count}: stream={} pts={:?} dts={:?} key={} {}B  head={:02x?}",
                        pkt.stream_index, pkt.pts, pkt.dts, pkt.flags.keyframe, pkt.data.len(), &pkt.data[..n],
                    );
                }
                if first_keyframe.is_none() {
                    first_keyframe = Some(pkt.flags.keyframe);
                    first_key_pts = pkt.pts;
                }
                if let (Some(prev), Some(now)) = (last_pts, pkt.pts) {
                    if now < prev {
                        monotonic = false;
                    }
                }
                if pkt.pts.is_some() {
                    last_pts = pkt.pts;
                }
                count += 1;
                if count > 5000 {
                    break; // safety
                }
            }
            Err(e) => {
                println!("(stop after {count} packets: {e:?})");
                break;
            }
        }
    }

    let key_ok = first_keyframe == Some(true);
    let pts_ok = last_pts.is_some() && monotonic;
    // Continuity: the last PTS must reach past the FIRST segment (~4 s) — proving the
    // demuxer read through the concatenated later segments, not stopping at seg 0.
    let span_ticks = last_pts.zip(first_key_pts).map(|(l, f)| l - f).unwrap_or(0);
    let span_us = span_ticks * 1_000_000 / den;
    let multi_ok = segs.len() < 2 || span_us > 5_000_000; // >5 s ⇒ crossed ≥2 segments
    println!(
        "-> packets={count} first_keyframe={key_ok} pts_monotonic={pts_ok} extradata={extradata_ok} span={:.1}s multi_ok={multi_ok}",
        span_us as f64 / 1e6,
    );
    let pass = count >= 2 && pts_ok && extradata_ok && key_ok && multi_ok;
    println!("{}", if pass { "PASS" } else { "FAIL" });
    pass
}

fn main() {
    println!("oxideav-mp4 {} — CMAF MULTI-segment demux verification (Tears of Steel)", env!("CARGO_PKG_VERSION"));
    let vi = read("samples/video_init.dash");
    let v0 = read("samples/video_seg0.dash");
    let v1 = read("samples/video_seg1.dash");
    let v2 = read("samples/video_seg2.dash");
    let ai = read("samples/audio_init.dash");
    let a0 = read("samples/audio_seg0.dash");
    let a1 = read("samples/audio_seg1.dash");
    let a2 = read("samples/audio_seg2.dash");

    // Exactly what open_fmp4 does: concat init + N media segments, one demuxer each.
    let v = probe("VIDEO (avc1, 224x100) × 3 segs", &vi, &[&v0, &v1, &v2]);
    let a = probe("AUDIO (mp4a AAC-LC, 48k stereo) × 3 segs", &ai, &[&a0, &a1, &a2]);

    println!("\n================ RESULT ================");
    println!("video: {}", if v { "PASS" } else { "FAIL" });
    println!("audio: {}", if a { "PASS" } else { "FAIL" });
    if v && a {
        println!("ALL PASS — oxideav-mp4 demuxes real CMAF correctly; wire Demux::Fmp4.");
        std::process::exit(0);
    } else {
        println!("FAILURE — do NOT build Demux::Fmp4 on this until resolved.");
        std::process::exit(1);
    }
}
