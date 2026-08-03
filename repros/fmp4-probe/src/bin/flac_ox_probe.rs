//! Same streaming proof as `audio-decode-probe/flac_stream_probe` (raw FLAC over a
//! BYTE-RANGE reader: header-only startup + bounded seek + decode to PCM) — but through
//! the **oxideav** stack (`oxideav-flac` native container demuxer + decoder on the
//! oxideav `Demuxer`/`Decoder` traits) instead of symphonia. This is the "move audio
//! onto oxideav-* too" candidate: FLAC then rides the exact same `Demuxer` abstraction
//! as MP4/MKV/CMAF, and the same `Decoder` push/pull as AC-3.
//!
//! Metric = range requests (a read after a discontiguous seek ≈ one HTTP range req).
//! Run: cargo run --release --bin flac_ox_probe [file.flac]   (default: ../sine48.flac)

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oxideav_core::{
    ContainerRegistry, Decoder, Demuxer, Frame, NullCodecResolver, ReadSeek,
};

struct Counting {
    file: File,
    pos: u64,
    read: Arc<AtomicU64>,
    range_reqs: Arc<AtomicU64>,
    seek_pending: bool,
}
impl Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.seek_pending {
            self.range_reqs.fetch_add(1, Ordering::Relaxed);
            self.seek_pending = false;
        }
        let n = self.file.read(buf)?;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        self.pos += n as u64;
        Ok(n)
    }
}
impl Seek for Counting {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let p = self.file.seek(from)?;
        if p != self.pos {
            self.seek_pending = true;
        }
        self.pos = p;
        Ok(p)
    }
}

/// Pull one decoded audio frame's per-channel sample count (send one packet, receive
/// one frame — FLAC is 1 packet → 1 frame). Returns None on a non-audio / empty frame.
fn decode_samples(dec: &mut Box<dyn Decoder>, pkt: &oxideav_core::Packet) -> Option<u32> {
    dec.send_packet(pkt).ok()?;
    match dec.receive_frame() {
        Ok(Frame::Audio(af)) => Some(af.samples),
        _ => None,
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("{}/../sine48.flac", env!("CARGO_MANIFEST_DIR")));
    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("file: {path}\nsize: {} bytes ({:.1} KB)\n", len, len as f64 / 1e3);

    let read = Arc::new(AtomicU64::new(0));
    let reqs = Arc::new(AtomicU64::new(0));
    let input: Box<dyn ReadSeek> = Box::new(Counting {
        file: File::open(&path).expect("open flac"),
        pos: 0,
        read: read.clone(),
        range_reqs: reqs.clone(),
        seek_pending: false,
    });

    // oxideav container demuxer, same trait as MP4/MKV.
    let mut creg = ContainerRegistry::new();
    oxideav_flac::register_containers(&mut creg);
    let mut dmx: Box<dyn Demuxer> = match creg.open_demuxer("flac", input, &NullCodecResolver) {
        Ok(d) => d,
        Err(e) => {
            println!("open FAILED: {e:?}");
            return;
        }
    };

    let streams = dmx.streams().to_vec();
    let Some(a) = streams.iter().find(|s| {
        format!("{:?}", s.params.media_type).contains("Audio")
    }) else {
        println!("no audio stream");
        return;
    };
    let sidx = a.index;
    let sr = a.params.sample_rate.unwrap_or(0);
    let ch = a.params.channels.unwrap_or(0);
    let (num, den) = (a.time_base.num(), a.time_base.den());
    let dur_us = dmx.duration_micros().unwrap_or(0);
    println!(
        "== opened ==  codec={:?}  {sr} Hz  {ch}ch  ~{:.1}s  (oxideav Demuxer)",
        a.params.codec_id, dur_us as f64 / 1e6
    );

    let mut dec: Box<dyn Decoder> = match oxideav_flac::decoder::make_decoder(&a.params) {
        Ok(d) => d,
        Err(e) => {
            println!("make_decoder FAILED: {e:?}");
            return;
        }
    };

    // --- startup: reqs/bytes to first decoded PCM ---
    let mut first_pcm_bytes = 0u64;
    let mut frames = 0u64;
    for _ in 0..40 {
        let pkt = match dmx.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if pkt.stream_index != sidx {
            continue;
        }
        if let Some(s) = decode_samples(&mut dec, &pkt) {
            if first_pcm_bytes == 0 {
                first_pcm_bytes = read.load(Ordering::Relaxed);
            }
            frames += s as u64;
        }
    }
    println!(
        "== startup ==  first PCM after {} reqs / {} bytes ({:.1}% of file);  decoded {} samples/ch",
        reqs.load(Ordering::Relaxed),
        first_pcm_bytes,
        100.0 * first_pcm_bytes as f64 / len.max(1) as f64,
        frames,
    );

    // --- seek to the middle (oxideav seek_to; seektable used if present, else scan) ---
    let target_us = dur_us / 2;
    let target_pts = if num == 0 {
        0
    } else {
        (target_us as i128 * den as i128 / (num as i128 * 1_000_000)) as i64
    };
    reqs.store(0, Ordering::Relaxed);
    read.store(0, Ordering::Relaxed);
    match dmx.seek_to(sidx, target_pts) {
        Ok(landed_pts) => {
            let landed_us = if den == 0 {
                0
            } else {
                (landed_pts as i128 * num as i128 * 1_000_000 / den as i128) as i64
            };
            // decode one packet post-seek
            let mut ok = false;
            for _ in 0..8 {
                match dmx.next_packet() {
                    Ok(p) if p.stream_index == sidx => {
                        if decode_samples(&mut dec, &p).is_some() {
                            ok = true;
                            break;
                        }
                    }
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
            let seek_reqs = reqs.load(Ordering::Relaxed);
            let seek_bytes = read.load(Ordering::Relaxed);
            println!(
                "== seek → {:.1}s ==  landed {:.1}s (Δ{:.0}ms),  {} reqs / {} bytes ({:.1}% of file),  decode {}",
                target_us as f64 / 1e6,
                landed_us as f64 / 1e6,
                (target_us as i64 - landed_us).abs() as f64 / 1000.0,
                seek_reqs,
                seek_bytes,
                100.0 * seek_bytes as f64 / len.max(1) as f64,
                if ok { "ok" } else { "FAILED" },
            );
            println!("\n================ RESULT ================");
            let start_ok = first_pcm_bytes < len / 2;
            let seek_ok = ok && seek_bytes < len;
            if start_ok && seek_ok {
                println!("✅ oxideav-flac over a byte-range reader: header-only startup + bounded-read seek.");
                println!("   FLAC rides the SAME oxideav Demuxer/Decoder traits as MP4/MKV/AC-3 — one framework.");
            } else {
                println!("⚠️ start_ok={start_ok} seek_ok={seek_ok} — investigate.");
            }
        }
        Err(e) => println!("== seek FAILED: {e:?}"),
    }
}
