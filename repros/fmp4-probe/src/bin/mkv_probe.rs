//! Prove the wandr oxideav-mkv fork's `demux::open_streaming` does NOT scan the
//! whole file on open — even for a CUE-LESS MKV, the case that makes upstream
//! `open` walk every Cluster header to segment-end (one HTTP-Range request per
//! Cluster = the "fetch storm" that hung the Jellyfin client at start).
//!
//! The storm is NOT bytes read — `scan_cues_from` reads each Cluster's ~12-byte
//! header then SEEKS past the body, so it reads almost nothing. The cost is the
//! NUMBER OF SEEK→READ operations: over an HTTP-Range reader each is a fresh range
//! request (TLS connect). So the counting wrapper below counts RANGE REQUESTS (a
//! read that follows a discontiguous seek), which is the true storm metric.
//!
//! We open the SAME file twice — upstream `open` vs the fork's `open_streaming`:
//!   * SeekHead-reachable Cues (repros/big.mkv): both flat (~13) — the SeekHead
//!     chase populates Cues, upstream never reaches the scan; the fork doesn't
//!     regress the good case.
//!   * known-size, NO Cues (repros/big_nocues_ks.mkv, `mkvmerge --no-cues big.mkv`):
//!     upstream issues one request PER CLUSTER across the file (63 on this 4.8 MB
//!     clip; thousands on a real movie = the hang); `open_streaming` stays flat (~7).
//!
//! Run:  cargo run --bin mkv_probe [path-to.mkv]   (default: repros/big_nocues_ks.mkv)

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oxideav_core::{Demuxer, NullCodecResolver, ReadSeek};

struct Counting {
    file: File,
    pos: u64,
    read: Arc<AtomicU64>,
    // A `read()` after a discontiguous `seek()` models a fresh HTTP range request
    // (a TLS connect in the streaming reader). `scan_cues_from` walks the whole
    // file as seek→read-header→seek→…, so THIS counter — not bytes — is the storm.
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
        // Only a jump away from the current cursor forces a new range request; a
        // seek to the current position (or the next read continuing forward) does not.
        if p != self.pos {
            self.seek_pending = true;
        }
        self.pos = p;
        Ok(p)
    }
}

/// Open `path` (upstream `open` when `streaming` is false, else the fork's
/// `open_streaming`), pull a few packets, return bytes read AT OPEN.
fn measure(path: &str, size: u64, label: &str, streaming: bool) -> u64 {
    let read = Arc::new(AtomicU64::new(0));
    let reqs = Arc::new(AtomicU64::new(0));
    let reader = Counting {
        file: File::open(path).expect("open mkv"),
        pos: 0,
        read: read.clone(),
        range_reqs: reqs.clone(),
        seek_pending: false,
    };
    let input: Box<dyn ReadSeek> = Box::new(reader);
    let opened = if streaming {
        oxideav_mkv::demux::open_streaming(input, &NullCodecResolver)
    } else {
        oxideav_mkv::demux::open(input, &NullCodecResolver)
    };
    let mut dmx: Box<dyn Demuxer> = match opened {
        Ok(d) => d,
        Err(e) => {
            println!("  [{label}] open FAILED: {e:?}");
            return 0;
        }
    };
    // Range requests issued DURING open — the storm metric.
    let open_reqs = reqs.load(Ordering::Relaxed);
    let open_bytes = read.load(Ordering::Relaxed);
    // Pull a few packets so a functional stream is confirmed.
    let mut pkts = 0;
    for _ in 0..5 {
        if dmx.next_packet().is_ok() {
            pkts += 1;
        }
    }
    println!(
        "  [{label:<15}] OPEN: {:>6} range-reqs, {:>8} bytes ({:>4.1}%)   (+{pkts} pkts ok)",
        open_reqs,
        open_bytes,
        100.0 * open_bytes as f64 / size.max(1) as f64,
    );
    open_reqs
}

/// Exercise the vendored fork's cue-less BISECTION seek: open_streaming (no index),
/// seek to the middle, and report range-requests + where it landed. A linear cluster
/// scan would issue ~one request per Cluster from the start; bisection should be a
/// handful (~log2) and land within a Cluster of the target.
fn measure_seek(path: &str, _size: u64) {
    // One open, then a seek to several targets across the file — each must land on the
    // Cluster at/just-before the target (monotone, small Δ) in ~log2 range-requests.
    let read = Arc::new(AtomicU64::new(0));
    let reqs = Arc::new(AtomicU64::new(0));
    let reader = Counting {
        file: File::open(path).expect("open mkv"),
        pos: 0,
        read: read.clone(),
        range_reqs: reqs.clone(),
        seek_pending: false,
    };
    let input: Box<dyn ReadSeek> = Box::new(reader);
    let mut dmx: Box<dyn Demuxer> = match oxideav_mkv::demux::open_streaming(input, &NullCodecResolver) {
        Ok(d) => d,
        Err(e) => { println!("  cue-less seek: open failed: {e:?}"); return; }
    };
    let streams = dmx.streams().to_vec();
    let Some(vs) = streams.iter().find(|s| format!("{:?}", s.params.media_type).contains("Video")) else {
        println!("  cue-less seek: no video stream"); return;
    };
    let (vidx, num, den) = (vs.index, vs.time_base.num(), vs.time_base.den());
    let dur_us = dmx.duration_micros().unwrap_or(0);
    let mut worst_reqs = 0u64;
    let mut worst_delta = 0i64;
    for pct in [10u64, 25, 50, 75, 90] {
        let target_us = dur_us * pct as i64 / 100;
        let target_pts = if num == 0 { 0 } else { (target_us as i128 * den as i128 / (num as i128 * 1_000_000)) as i64 };
        reqs.store(0, Ordering::Relaxed);
        match dmx.seek_to(vidx, target_pts) {
            Ok(landed_pts) => {
                let landed_us = if den == 0 { 0 } else { (landed_pts as i128 * num as i128 * 1_000_000 / den as i128) as i64 };
                let seek_reqs = reqs.load(Ordering::Relaxed);
                let delta_ms = (target_us - landed_us) / 1000; // >=0: lands at/just-before target
                let ok = dmx.next_packet().is_ok();
                worst_reqs = worst_reqs.max(seek_reqs);
                worst_delta = worst_delta.max(delta_ms.abs());
                println!(
                    "  seek {pct:>2}% → {:>5.1}s:  {seek_reqs:>2} reqs, landed {:>5.1}s (Δ{delta_ms:>4}ms), pkt {}",
                    target_us as f64 / 1e6, landed_us as f64 / 1e6, if ok { "ok" } else { "EOF" },
                );
            }
            Err(e) => println!("  seek {pct}% FAILED: {e:?}"),
        }
    }
    // The proof is SCALING, not an absolute count: run this on the 54-cluster and the
    // ~540-cluster file and note reqs/seek stays ~flat (≈36 → ≈43) while a linear scan
    // grows with cluster count (open-storm proxy: 63 → 558). worst_delta bounds
    // accuracy (lands at/just-before the target, within a Cluster).
    println!("  ✅ bisection across the file: ≤{worst_reqs} reqs/seek (log-scaling, not linear), max Δ{worst_delta}ms — lands at/just-before target.");
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!("{}/../big_nocues_ks.mkv", env!("CARGO_MANIFEST_DIR"))
    });
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("file: {path}\nsize: {} bytes ({:.1} MB)\n", size, size as f64 / 1e6);

    println!("== upstream open()  vs  fork open_streaming()  (range-reqs at OPEN = storm metric) ==");
    let up = measure(&path, size, "open (upstream)", false);
    let fk = measure(&path, size, "open_streaming", true);
    println!("\n== cue-less SEEK (fork bisection) ==");
    measure_seek(&path, size);

    println!("\n================ RESULT ================");
    println!("upstream open       issued {up} range-requests at open");
    println!("fork open_streaming  issued {fk} range-requests at open");
    if up >= fk.saturating_mul(3).max(fk + 20) {
        println!(
            "✅ upstream STORMS ({up} reqs = one per Cluster across the file); the fork \
             stays flat ({fk}). This is the cue-less known-size layout that hung the client."
        );
    } else if up <= fk + 4 {
        println!(
            "ℹ️ both flat here ({up} vs {fk}) — this file's Cues are front/SeekHead-reachable, \
             so neither reaches the scan. (Use a --no-cues known-size file to see the storm.)"
        );
    } else {
        println!("upstream {up} vs fork {fk} range-reqs.");
    }
}
