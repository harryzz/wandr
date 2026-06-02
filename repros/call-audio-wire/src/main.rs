//! call-audio-wire — wire wart-call's PCM ends to the real audio hardware.
//!
//! The final piece of the call engine: connect `MediaSession`'s PCM in/out to the
//! host `audio` WIT (mic capture + AAudio playback). To keep it verifiable on a
//! single device — which can't hold input+output MMAP simultaneously — it records
//! through the call media pipeline, then plays the result back:
//!
//!   mic ─read-pcm-f32→ PCM ─MediaSession.send→ Opus/RTP/SRTP ─┐ (loopback keys)
//!   speaker ←write-pcm-f32─ PCM ←MediaSession.recv─ SRTP/RTP/Opus┘
//!
//! You speak; you hear yourself back through the exact codec+crypto a live call
//! uses. In a real two-device call the same wiring runs, but each device only
//! captures (→ peer) or plays (← peer), so the MMAP limit never bites.
//!
//!   wart-host --run-once war.probe.callaudio

wit_bindgen::generate!({
    world: "callaudio",
    path: "wit",
    generate_all,
});

use std::time::{Duration, Instant};

use wart_call::{MediaSession, SrtpKeys, OPUS_PAYLOAD_TYPE, SAMPLE_RATE};

use crate::my::skiko_gfx::audio::{self, ChannelLayout, Format, TrackConfig};

/// 20 ms @ 48 kHz mono — the Opus frame + the audio tick granularity.
const FRAME: usize = SAMPLE_RATE as usize / 1000 * 20;

/// Fixed SRTP master key/salt standing in for the DTLS-derived keys (this is the
/// media-plane wiring test; the same loopback as repros/call-media-pipeline).
const MASTER_KEY: [u8; 16] = [
    0x0d, 0xcd, 0x21, 0x3e, 0x4c, 0xbc, 0xf2, 0x8f, 0x01, 0x7f, 0x69, 0x94, 0x40, 0x1e, 0x28, 0x89,
];
const MASTER_SALT: [u8; 14] =
    [0x62, 0x77, 0x60, 0x38, 0xc0, 0x6d, 0xc9, 0x41, 0x9f, 0x6d, 0xd9, 0x43, 0x3e, 0x7c];

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|&v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}


fn main() {
    let cfg = TrackConfig {
        sample_rate: SAMPLE_RATE,
        channel_layout: ChannelLayout::Mono,
        format: Format::PcmF32,
    };

    // The PCM ⇄ encrypted-RTP pipeline (loopback keys: send ctx == recv ctx).
    let keys = SrtpKeys { key: MASTER_KEY, salt: MASTER_SALT };
    let mut media = MediaSession::new(SAMPLE_RATE, 1, OPUS_PAYLOAD_TYPE, 0xA, &keys, &keys)
        .expect("MediaSession");

    // ---- Phase 1: mic → wart-call media pipeline → buffer ----
    let cap = audio::open_capture(cfg);
    if cap == 0 {
        println!("[callaudio] open-capture failed (service unavailable / RECORD_AUDIO / SELinux)");
        std::process::exit(1);
    }
    audio::start(cap);
    println!("[callaudio] capturing ~3 s of mic → wart-call (Opus + SRTP loopback) — speak now…");

    let mut pending: Vec<f32> = Vec::new();
    let mut out: Vec<f32> = Vec::new();
    let mut in_rms_acc = 0f64;
    let mut in_frames = 0usize;
    let target_frames = 150; // ~3 s at 20 ms/frame
    let deadline = Instant::now() + Duration::from_secs(6);
    while in_frames < target_frames && Instant::now() < deadline {
        let chunk = audio::read_pcm_f32(cap, FRAME as u32);
        if chunk.is_empty() {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        pending.extend_from_slice(&chunk);
        while pending.len() >= FRAME {
            let frame: Vec<f32> = pending.drain(..FRAME).collect();
            in_rms_acc += rms(&frame) as f64;
            in_frames += 1;
            let srtp = media.send(&frame).expect("media.send");
            let pcm = media.recv(&srtp).expect("media.recv");
            out.extend_from_slice(&pcm);
        }
    }
    audio::close(cap);

    let in_rms = if in_frames > 0 { in_rms_acc / in_frames as f64 } else { 0.0 };
    println!(
        "[callaudio] {in_frames} frames through wart-call ({} samples out); mic_rms={in_rms:.4} out_rms={:.4}",
        out.len(),
        rms(&out)
    );
    if in_frames == 0 {
        println!("[callaudio] no mic frames captured — aborting");
        std::process::exit(1);
    }

    // ---- Phase 2: buffer → AAudio playback (sequential; in+out MMAP limit) ----
    // The Pixel 2 XL's MMAP output endpoint is stereo-only (a mono track is
    // rejected → -889), so play a stereo track, interleaving the mono pipeline
    // output as L = R.
    let trk_cfg = TrackConfig {
        sample_rate: SAMPLE_RATE,
        channel_layout: ChannelLayout::Stereo,
        format: Format::PcmF32,
    };
    let trk = audio::create_track(trk_cfg);
    if trk == 0 {
        println!("[callaudio] create-track failed");
        std::process::exit(1);
    }
    // Build the full interleaved-stereo playback buffer:
    //   (a) a loud 1.5 s 440 Hz reference tone — isolates the speaker path from
    //       the mic; if you hear THIS but not yourself, the mic gain is the issue,
    //   (b) the captured mic, amplified so quiet speech is audible.
    let mut all: Vec<f32> = Vec::new();
    let tone_frames = SAMPLE_RATE as usize * 3 / 2;
    for n in 0..tone_frames {
        let s = (2.0 * std::f32::consts::PI * 440.0 * n as f32 / SAMPLE_RATE as f32).sin() * 0.3;
        all.push(s);
        all.push(s);
    }
    let peak = out.iter().fold(0f32, |m, &v| m.max(v.abs()));
    let gain = if peak > 1e-6 { (0.3 / peak).min(30.0) } else { 1.0 };
    for &s in &out {
        let a = (s * gain).clamp(-1.0, 1.0);
        all.push(a);
        all.push(a);
    }
    println!(
        "[callaudio] playing 1.5 s reference tone + your captured mic ×{gain:.1} (peak {peak:.4})…"
    );

    // write-then-start: this device's AAudio output won't pull a stream that was
    // started empty — prime the ring first, THEN start, then stream the rest.
    let mut i = 0; // sample index (2 per frame)
    while i < all.len() {
        let end = (i + FRAME * 2).min(all.len());
        let wrote = audio::write_pcm_f32(trk, &all[i..end]) as usize;
        if wrote == 0 {
            break; // ring primed (full)
        }
        i += wrote * 2;
    }
    audio::start(trk);
    let deadline = Instant::now() + Duration::from_secs(15);
    while i < all.len() && Instant::now() < deadline {
        let end = (i + FRAME * 2).min(all.len());
        let wrote = audio::write_pcm_f32(trk, &all[i..end]) as usize;
        if wrote == 0 {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        i += wrote * 2;
    }
    let drain = Instant::now() + Duration::from_secs(5);
    while audio::pending_frames(trk) > 0 && Instant::now() < drain {
        std::thread::sleep(Duration::from_millis(20));
    }
    audio::close(trk);

    println!("[callaudio] DONE — mic → wart-call (Opus + SRTP) → AAudio, end-to-end on real hardware");
}
