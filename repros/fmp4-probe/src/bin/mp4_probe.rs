//! Prove `oxideav_mp4::demux::open` on a PROGRESSIVE MP4 is HTTP-Range-friendly: it
//! parses ftyp + moov and SKIPS mdat by seeking (never reads the media body), so over
//! a Range reader it costs only a handful of range requests regardless of file size —
//! even when `moov` sits AFTER `mdat` (non-faststart), the case that would read the
//! whole file if open() streamed mdat. This is what lets the wandr engine retire the
//! `mp4` crate and demux progressive MP4 over HTTP the same way it does CMAF.
//!
//! Metric = range requests (a read after a discontiguous seek), same as mkv_probe.
//! Make the inputs:  ffmpeg -i in.mkv -c copy -movflags +faststart prog_faststart.mp4
//!                   ffmpeg -i in.mkv -c copy                        prog_moovend.mp4
//! Run:  cargo run --bin mp4_probe [file.mp4 ...]   (defaults: both prog_*.mp4)

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oxideav_core::{Demuxer, NullCodecResolver, ReadSeek};

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

fn probe(path: &str) {
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => { println!("  {path}: cannot stat: {e}"); return; }
    };
    let read = Arc::new(AtomicU64::new(0));
    let reqs = Arc::new(AtomicU64::new(0));
    let reader = Counting {
        file: File::open(path).expect("open mp4"),
        pos: 0,
        read: read.clone(),
        range_reqs: reqs.clone(),
        seek_pending: false,
    };
    let input: Box<dyn ReadSeek> = Box::new(reader);
    let mut dmx: Box<dyn Demuxer> = match oxideav_mp4::demux::open(input, &NullCodecResolver) {
        Ok(d) => d,
        Err(e) => { println!("  {path}: open FAILED: {e:?}"); return; }
    };
    let open_reqs = reqs.load(Ordering::Relaxed);
    let open_bytes = read.load(Ordering::Relaxed);
    let mut pkts = 0;
    for _ in 0..5 {
        if dmx.next_packet().is_ok() { pkts += 1; }
    }
    let v = dmx.streams().iter().filter(|s| format!("{:?}", s.params.media_type).contains("Video")).count();
    let a = dmx.streams().iter().filter(|s| format!("{:?}", s.params.media_type).contains("Audio")).count();
    let verdict = if open_bytes < size / 2 { "✅ mdat skipped (no whole-file read)" } else { "⚠️ read >50% — mdat NOT skipped" };
    println!(
        "  {:<22} {:>4} range-reqs, {:>9} bytes ({:>4.1}%) at open  [{v}v/{a}a, +{pkts} pkts]  {verdict}",
        path.rsplit('/').next().unwrap_or(path),
        open_reqs, open_bytes, 100.0 * open_bytes as f64 / size.max(1) as f64,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let files: Vec<String> = if args.is_empty() {
        let base = env!("CARGO_MANIFEST_DIR");
        vec![format!("{base}/../prog_faststart.mp4"), format!("{base}/../prog_moovend.mp4")]
    } else {
        args
    };
    println!("== oxideav_mp4::demux::open — range requests on a PROGRESSIVE MP4 ==");
    for f in &files {
        probe(f);
    }
    println!("(moov-at-end is the telling case: if open streamed mdat it would read ~100%.)");
}
