use std::io::Cursor;
use std::time::Instant;
use libde265::{De265, Decoder as De265Decoder};
use mp4::Mp4Reader;
use wandr_video::{open_decoder, Chunk, Codec, DecoderParams};

const SC: [u8; 4] = [0, 0, 0, 1];

/// Read bbb-h265.mp4 → Vec of Annex-B access units (hvcC prefix at keyframes).
fn load_units(path: &str) -> (Vec<Vec<u8>>, u32, u32) {
    let bytes = std::fs::read(path).unwrap();
    let mut mp4 = Mp4Reader::read_header(Cursor::new(&bytes[..]), bytes.len() as u64).unwrap();
    let tid = mp4.tracks().values()
        .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H265))).unwrap().track_id();
    let (vw, vh) = (mp4.tracks()[&tid].width() as u32, mp4.tracks()[&tid].height() as u32);
    // hvcC VPS/SPS/PPS
    let p = bytes.windows(4).position(|w| w == b"hvcC").unwrap();
    let pl = &bytes[p + 4..]; let na = pl[22] as usize;
    let mut pre = Vec::new(); let mut i = 23;
    for _ in 0..na { let num = u16::from_be_bytes([pl[i+1], pl[i+2]]) as usize; i += 3;
        for _ in 0..num { let l = u16::from_be_bytes([pl[i], pl[i+1]]) as usize; i += 2;
            pre.extend_from_slice(&SC); pre.extend_from_slice(&pl[i..i+l]); i += l; } }
    let n = mp4.tracks()[&tid].sample_count();
    let mut units = Vec::with_capacity(n as usize);
    for sid in 1..=n {
        let s = mp4.read_sample(tid, sid).unwrap().unwrap();
        let mut au = Vec::new();
        if s.is_sync { au.extend_from_slice(&pre); }
        let mut j = 0;
        while j + 4 <= s.bytes.len() {
            let ln = u32::from_be_bytes([s.bytes[j], s.bytes[j+1], s.bytes[j+2], s.bytes[j+3]]) as usize;
            j += 4; au.extend_from_slice(&SC); au.extend_from_slice(&s.bytes[j..j+ln]); j += ln;
        }
        units.push(au);
    }
    (units, vw, vh)
}

fn bench_oxideav(units: &[Vec<u8>]) -> (usize, f64) {
    let mut dec = open_decoder(&DecoderParams { codec: Codec::H265, width: 0, height: 0 }).unwrap();
    let t = Instant::now();
    let mut frames = 0;
    for u in units {
        if dec.decode(Chunk::new(u, 0)).is_ok() {
            while dec.next_frame().is_some() { frames += 1; }
        }
    }
    dec.flush().ok();
    while dec.next_frame().is_some() { frames += 1; }
    (frames, t.elapsed().as_secs_f64())
}

fn bench_libde265(units: &[Vec<u8>], threads: i32) -> (usize, f64) {
    let de = De265::new().unwrap();
    let mut dec = De265Decoder::new(de.clone());
    if threads > 0 { dec.start_worker_threads(threads as u32).unwrap(); }
    let t = Instant::now();
    let mut frames = 0;
    for u in units {
        dec.push_data(u, 0, None).unwrap();
        dec.decode().unwrap();
        while dec.get_next_picture().is_some() { frames += 1; }
    }
    dec.flush_data().ok();
    loop {
        dec.decode().ok();
        if let Some(_img) = dec.get_next_picture() { frames += 1; } else { break; }
    }
    (frames, t.elapsed().as_secs_f64())
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or("../oxideav-spike/samples/bbb-h265.mp4".into());
    let (units, w, h) = load_units(&path);
    println!("HEVC decode benchmark — {w}x{h}, {} access units\n", units.len());

    let (f1, t1) = bench_oxideav(&units);
    println!("oxideav-h265 (pure-Rust, 1 thread):  {f1} frames in {t1:.2}s = {:.1} fps", f1 as f64 / t1);

    let (f2, t2) = bench_libde265(&units, 1);
    println!("libde265     (C/LGPL,   1 thread):   {f2} frames in {t2:.2}s = {:.1} fps", f2 as f64 / t2);

    let nt = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4);
    let (f3, t3) = bench_libde265(&units, nt);
    println!("libde265     (C/LGPL,   {nt} threads):  {f3} frames in {t3:.2}s = {:.1} fps", f3 as f64 / t3);

    println!("\nrealtime bar = 30 fps");
    println!("speedup libde265-1t vs oxideav: {:.1}x", (f2 as f64/t2)/(f1 as f64/t1));
}
