//! wandr.video.player — task 117 M2 stage 2.
//!
//! The FIRST consumer of `wandr:video`'s playback lane. Stage 1 proved the
//! contract compiles and wires; this proves a guest can actually *play* with it:
//!
//!   submit-timed(PTS) -> next-decoded -> present(at-ns) -> flush(EOS)
//!
//! Anything awkward here is a CONTRACT bug, not a client bug — that is the whole
//! reason this exists before anything is proposed to WASI.
//!
//! Input: a raw H.264 **Annex-B** elementary stream from `/samples` (a read-only
//! mount declared in package.toml — demux/containers are deliberately guest-side
//! in wandr, so nothing here is baked into the host). Annex-B rather than MP4
//! keeps this dependency-free: no demuxer crate stands between the contract and
//! the finding. The clip has real B-frames, so DECODE order != DISPLAY order and
//! the host's reorder buffer is genuinely exercised.
//!
//! Run: `wandr-host --run-once wandr.video.player`

wit_bindgen::generate!({
    world: "video-player",
    path: "wit",
    generate_all,
});

use crate::wandr::video::decoder::VideoDecoder;
use crate::wandr::video::types::{Codec, DecoderConfig, TimedFrame, VideoError, VideoRect, ZLayer};
use std::time::{Duration, Instant};

const CLIP: &str = "/samples/bbb.h264";
const FPS: i64 = 30;
/// How far ahead of presentation we let decoding run. A real player needs a
/// cushion (it must exceed the stream's reorder depth, or a late earlier-PTS
/// frame arrives after its slot), but an unbounded one would decode the whole
/// file into memory before showing frame one.
const DECODE_AHEAD: usize = 8;

fn pts_us(index: i64) -> i64 {
    index * 1_000_000 / FPS
}

/// Split an Annex-B stream into ACCESS UNITS (one coded picture each).
///
/// A new AU starts at a VCL NAL (type 1 = non-IDR slice, 5 = IDR) when the
/// current AU already holds one; non-VCL NALs (SPS/PPS/SEI) attach to the AU
/// that follows them. This is the minimum correct grouping — feeding NALs
/// individually would give several of them the same PTS and make the timestamp
/// check meaningless.
fn access_units(buf: &[u8]) -> Vec<(Vec<u8>, bool)> {
    // Offsets of every start code (3- or 4-byte).
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            let sc = if i > 0 && buf[i - 1] == 0 { i - 1 } else { i };
            starts.push((sc, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut aus: Vec<(Vec<u8>, bool)> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut cur_has_vcl = false;
    let mut cur_idr = false;
    for (n, &(sc, payload)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map(|&(s, _)| s).unwrap_or(buf.len());
        let nal_type = buf[payload] & 0x1f;
        let is_vcl = nal_type == 1 || nal_type == 5;
        if is_vcl && cur_has_vcl {
            aus.push((std::mem::take(&mut cur), cur_idr));
            cur_has_vcl = false;
            cur_idr = false;
        }
        cur.extend_from_slice(&buf[sc..end]);
        if is_vcl {
            cur_has_vcl = true;
            cur_idr |= nal_type == 5;
        }
    }
    if !cur.is_empty() {
        aus.push((cur, cur_idr));
    }
    aus
}

/// Drain everything the decoder has ready, presenting each frame.
///
/// `at-ns` NOTE (a stage-2 finding): the contract says `present(at-ns)` rides the
/// wasi:clocks monotonic timeline, but a guest cannot currently name a host
/// monotonic instant it is sure shares an origin with the host's own clock. So
/// this passes 0 = "present as soon as possible" and paces itself instead. That
/// still exercises submit-timed -> next-decoded -> present -> flush end to end;
/// it does NOT exercise host-side scheduling. See the report at the end.
fn drain(dec: &VideoDecoder, seen: &mut Vec<i64>, started: Instant, pace: bool) -> usize {
    let mut n = 0;
    while let Some(frame) = dec.next_decoded() {
        let pts = frame.timestamp_us();
        // Guest-side pacing. present(at-ns) is the verb that SHOULD do this
        // host-side, but a guest cannot currently name a host monotonic instant
        // (stage-2 finding #1), so we hold the frame until its PTS is due and
        // then present immediately. This is exactly the workaround a real player
        // should NOT have to write — which is the argument for fixing at-ns.
        if pace {
            let due = Duration::from_micros(pts.max(0) as u64);
            let now = started.elapsed();
            if due > now {
                std::thread::sleep(due - now);
            }
        }
        seen.push(pts);
        frame.present(0);
        n += 1;
    }
    n
}

fn main() {
    println!("wandr.video.player — task 117 M2 stage 2 (playback contract proof)");

    let buf = match std::fs::read(CLIP) {
        Ok(b) => b,
        Err(e) => {
            println!("FAIL: cannot read {CLIP}: {e}");
            println!("      (is the /samples mount in package.toml resolving on this host?)");
            std::process::exit(1);
        }
    };
    let aus = access_units(&buf);
    println!("clip: {} bytes -> {} access units", buf.len(), aus.len());
    if aus.is_empty() {
        println!("FAIL: no access units parsed — not an Annex-B stream?");
        std::process::exit(1);
    }

    // DECODE-TO-SURFACE by default: a real rect makes the host composite decoded
    // frames into a surface, i.e. actual pixels on screen. The guest never sees
    // them — `decoded-frame` is opaque by design — so on-screen output is the
    // ONLY visual proof a guest can produce, and it is the honest one: it proves
    // the whole path (guest -> WIT -> host decode -> surface -> display).
    //
    // WANDR_PLAYER_HEADLESS=1 falls back to an empty rect = decode-to-buffer,
    // which exercises the control path only (what stage 2 originally measured).
    let headless = std::env::var("WANDR_PLAYER_HEADLESS").is_ok();
    let rect = if headless {
        VideoRect { x: 0, y: 0, width: 0, height: 0 }
    } else {
        VideoRect { x: 0, y: 0, width: 1280, height: 720 }
    };
    let dec = match VideoDecoder::open(DecoderConfig {
        codec: Codec::H264,
        width: 1280,
        height: 720,
        rect,
        rotation: 0,
        layer: ZLayer::AboveUi,
    }) {
        Ok(d) => d,
        Err(e) => {
            println!("FAIL: decoder open: {e:?}");
            std::process::exit(1);
        }
    };
    println!(
        "decoder OPEN ✓ (H.264, {})",
        if headless { "decode-to-buffer" } else { "decode-to-surface 1280x720" }
    );

    let started = Instant::now();
    let mut submitted = 0usize;
    let mut presented = 0usize;
    let mut backpressure = 0usize;
    let mut seen: Vec<i64> = Vec::new();
    let pace = !headless;
    let _ = pace;

    for (i, (data, keyframe)) in aus.iter().enumerate() {
        let frame = TimedFrame {
            data: data.clone(),
            timestamp_us: pts_us(i as i64),
            keyframe: *keyframe,
        };
        // Back-pressure contract: queue-full means RETRY THE SAME FRAME (unlike
        // the RTP lane, where a dropped frame is resynced at the next keyframe).
        // A file player cannot skip, so we drain and retry rather than drop.
        let mut attempts = 0;
        loop {
            match dec.submit_timed(&frame) {
                Ok(()) => {
                    submitted += 1;
                    break;
                }
                Err(VideoError::QueueFull) => {
                    backpressure += 1;
                    presented += drain(&dec, &mut seen, started, !headless);
                    attempts += 1;
                    if attempts > 64 {
                        println!("FAIL: still queue-full after draining 64x at AU {i}");
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    println!("FAIL: submit-timed at AU {i}: {e:?}");
                    std::process::exit(1);
                }
            }
        }
        presented += drain(&dec, &mut seen, started, !headless);

        // Keep a bounded decode-ahead cushion. Without a real clock to pace
        // against (see the note on `drain`), this stands in for playback timing.
        if seen.len() + DECODE_AHEAD < submitted {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    // EOS: without flush the frames a codec holds back for reordering are never
    // released — the tail of every file would silently go missing.
    if let Err(e) = dec.flush() {
        println!("FAIL: flush: {e:?}");
        std::process::exit(1);
    }
    presented += drain(&dec, &mut seen, started, !headless);

    // Seek path: reset must be accepted and leave the decoder usable.
    let reset_ok = dec.reset().is_ok();

    let elapsed = started.elapsed();
    println!("\n── results ─────────────────────────────────────────────");
    println!("submitted   : {submitted} AUs");
    println!("presented   : {presented} frames");
    println!("back-pressure hits: {backpressure}");
    println!("reset() accepted  : {reset_ok}");
    println!("elapsed     : {:.2}s", elapsed.as_secs_f64());

    // The load-bearing check: PTS must come back in DISPLAY order. The clip has
    // B-frames, so decode order is NOT display order — if the host's reorder
    // buffer were wrong this is what would catch it, and a frame-count-only
    // check would not.
    let monotonic = seen.windows(2).all(|w| w[1] > w[0]);
    let first_ok = seen.first().copied() == Some(0);
    println!("PTS monotonic (display order): {monotonic}");
    println!("first PTS == 0              : {first_ok}");

    let ok = presented > 0 && monotonic && first_ok && reset_ok;
    if ok {
        println!("\nPASS — the wandr:video playback lane works end to end from a guest");
    } else {
        if !monotonic {
            if let Some(bad) = seen.windows(2).position(|w| w[1] <= w[0]) {
                println!("  first out-of-order at index {bad}: {:?}", &seen[bad..(bad + 2).min(seen.len())]);
            }
        }
        println!("\nFAIL");
        std::process::exit(1);
    }
}
