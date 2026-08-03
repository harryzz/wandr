//! Streaming proof for the audio-only engine profile: does Symphonia demux + decode +
//! SEEK a raw FLAC over a BYTE-RANGE (seekable) reader without slurping the whole file?
//! This is the "raw FLAC over HTTP" question — the same range-request metric the mkv/mp4
//! probes use (a read after a discontiguous seek ≈ one HTTP range request).
//!
//! Proves, on a synthetic seektable-less FLAC (the common case for server raw streams):
//!   * startup reads only headers + the first frames (not the whole file),
//!   * decode yields interleaved PCM (what wasi:audio wants),
//!   * `format.seek(Time)` lands near the target reading only a bounded tail (Symphonia
//!     bisects frames when there is no SEEKTABLE — same shape as our MKV cue-less seek).
//!
//! Native (like the container probes) so we can count I/O. Run:
//!   cargo run --release --bin flac_stream_probe [file.flac]   (default: ../sine48.flac)

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

struct Counting {
    file: File,
    len: u64,
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
// Symphonia needs is_seekable + byte_len to enable random-access seeking — exactly
// what an HTTP-Range reader advertises (Accept-Ranges + Content-Length).
impl MediaSource for Counting {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!("{}/../sine48.flac", env!("CARGO_MANIFEST_DIR"))
    });
    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("file: {path}\nsize: {} bytes ({:.1} KB)\n", len, len as f64 / 1e3);

    let read = Arc::new(AtomicU64::new(0));
    let reqs = Arc::new(AtomicU64::new(0));
    let src = Counting {
        file: File::open(&path).expect("open flac"),
        len,
        pos: 0,
        read: read.clone(),
        range_reqs: reqs.clone(),
        seek_pending: false,
    };

    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("flac");
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .expect("probe/format failed");
    let mut format = probed.format;
    let track = format.default_track().expect("no default track").clone();
    let track_id = track.id;
    let sr = track.codec_params.sample_rate.unwrap_or(0);
    let ch = track.codec_params.channels.map(|c| c.count()).unwrap_or(0);
    let dur_ts = track.codec_params.n_frames.unwrap_or(0);
    let dur_s = if sr > 0 { dur_ts as f64 / sr as f64 } else { 0.0 };
    println!("== opened ==  codec={:?}  {sr} Hz  {ch}ch  ~{dur_s:.1}s", track.codec_params.codec);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("make decoder");

    // --- startup: bytes/requests to the FIRST decoded PCM ---
    let mut first_pcm_bytes = 0u64;
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut frames_before_seek = 0u64;
    for _ in 0..40 {
        let pkt = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if pkt.track_id() != track_id {
            continue;
        }
        if let Ok(decoded) = decoder.decode(&pkt) {
            if sbuf.is_none() {
                first_pcm_bytes = read.load(Ordering::Relaxed);
                sbuf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec()));
            }
            let b = sbuf.as_mut().unwrap();
            b.copy_interleaved_ref(decoded);
            frames_before_seek += (b.samples().len() / ch.max(1)) as u64;
        }
    }
    println!(
        "== startup ==  first PCM after {} reqs / {} bytes ({:.1}% of file);  decoded {} frames",
        reqs.load(Ordering::Relaxed),
        first_pcm_bytes,
        100.0 * first_pcm_bytes as f64 / len.max(1) as f64,
        frames_before_seek,
    );

    // --- seek to the middle (seektable-less → Symphonia bisects frames) ---
    let target_s = dur_s / 2.0;
    reqs.store(0, Ordering::Relaxed);
    read.store(0, Ordering::Relaxed);
    match format.seek(
        SeekMode::Accurate,
        SeekTo::Time { time: Time::from(target_s), track_id: Some(track_id) },
    ) {
        Ok(seeked) => {
            let landed_s = seeked.actual_ts as f64 / sr.max(1) as f64;
            // Decode one packet post-seek to confirm the stream is live there.
            let mut ok = false;
            for _ in 0..8 {
                match format.next_packet() {
                    Ok(p) if p.track_id() == track_id => {
                        if decoder.decode(&p).is_ok() {
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
                target_s, landed_s, (target_s - landed_s).abs() * 1000.0,
                seek_reqs, seek_bytes, 100.0 * seek_bytes as f64 / len.max(1) as f64,
                if ok { "ok" } else { "FAILED" },
            );
            println!("\n================ RESULT ================");
            let start_ok = first_pcm_bytes < len / 2;
            let seek_ok = ok && seek_bytes < len;
            if start_ok && seek_ok {
                println!("✅ raw FLAC over a byte-range reader: header-only startup + bounded-read seek.");
                println!("   Symphonia (default features, royalty-free) = the audio-only engine's demux+decode.");
            } else {
                println!("⚠️ start_ok={start_ok} seek_ok={seek_ok} — investigate.");
            }
        }
        Err(e) => println!("== seek FAILED: {e}"),
    }
}
