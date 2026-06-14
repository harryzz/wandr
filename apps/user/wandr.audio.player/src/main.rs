//! wandr.audio.player — M1 spike (task 108).
//!
//! Decodes an embedded 48 kHz stereo FLAC with Symphonia (pure Rust, the
//! design's "guest-decode floor") and plays it through the host's wasi:audio
//! PCM contract. No contract change: wasi:audio already plays PCM; the
//! seekbar-driving `playback.position` is promoted in a later step.
//!
//! One binary, both targets: on the desktop host the audio backend is absent,
//! so `Playback::open` returns `unavailable` — we report the decode result and
//! skip playback (pipeline validated). On device the AudioFlinger backend
//! accepts the stream and it is audible.
//!
//!   cargo build --target wasm32-wasip2 --release
//!   wandr-host --run-once wandr.audio.player        # device: audible
//!   wasmtime run .../wandr-audio-player.wasm         # desktop: decode-only

use std::io::Cursor;
use std::time::{Duration, Instant};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

wit_bindgen::generate!({
    world: "audio-player",
    path: "wit",
    generate_all,
});

use crate::wasi::audio::pcm::{
    AudioError, ChannelLayout, Format, Playback, StreamClass, StreamConfig,
};

static FLAC: &[u8] = include_bytes!("test.flac");

struct Decoded {
    sample_rate: u32,
    channels: usize,
    /// Interleaved f32 (the wasi:audio wire format).
    samples: Vec<f32>,
}

/// Decode the embedded FLAC fully to interleaved f32 — the guest-side floor.
fn decode_flac() -> Decoded {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(FLAC.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("flac");

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .expect("probe/format failed");

    let mut format = probed.format;
    let track = format.default_track().expect("no default track").clone();
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(48_000);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("make decoder failed");

    let mut samples: Vec<f32> = Vec::new();
    let mut sbuf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = *audio.spec();
                if sbuf.is_none() {
                    sbuf = Some(SampleBuffer::<f32>::new(audio.capacity() as u64, spec));
                }
                let b = sbuf.as_mut().unwrap();
                b.copy_interleaved_ref(audio);
                samples.extend_from_slice(b.samples());
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    Decoded { sample_rate, channels, samples }
}

fn main() {
    let dec = decode_flac();
    let frames = dec.samples.len() / dec.channels.max(1);
    let secs = frames as f64 / dec.sample_rate.max(1) as f64;
    println!(
        "decode OK — {} Hz, {} ch, {} frames, {:.2} s ({} f32 samples)",
        dec.sample_rate, dec.channels, frames, secs, dec.samples.len()
    );

    // The contract is stereo today; the test asset is stereo @ 48 kHz, so no
    // resample is needed (rubato enters when a non-48 k source ships).
    let layout = match dec.channels {
        1 => ChannelLayout::Mono,
        _ => ChannelLayout::Stereo,
    };
    let config = StreamConfig {
        sample_rate: dec.sample_rate,
        channel_layout: layout,
        format: Format::PcmF32,
        class: StreamClass::Media,
    };

    let pb = match Playback::open(config) {
        Ok(pb) => pb,
        Err(AudioError::Unavailable) => {
            println!("playback SKIPPED — no audio backend (desktop host). Decode pipeline validated.");
            return;
        }
        Err(e) => {
            println!("playback open FAILED: {e:?}");
            return;
        }
    };
    if let Err(e) = pb.start() {
        println!("playback start FAILED: {e:?}");
        return;
    }
    println!("playback START ✓ — feeding {} frames", frames);

    let ch = dec.channels.max(1);
    let chunk_frames = 9_600usize; // 0.2 s @ 48 k
    let mut idx = 0usize; // sample index (interleaved)
    let started = Instant::now();
    let deadline = Duration::from_secs_f64(secs + 10.0); // generous guard
    let mut next_report = Duration::from_secs(2);

    while idx < dec.samples.len() {
        let end = (idx + chunk_frames * ch).min(dec.samples.len());
        let accepted = pb.write(&dec.samples[idx..end]); // frames accepted
        idx += accepted as usize * ch;
        if accepted == 0 {
            std::thread::sleep(Duration::from_millis(5)); // ring full — backpressure
        }
        // Validate the new position clock: it should track wall-clock at the
        // device rate (frames / sample_rate ≈ elapsed seconds).
        if started.elapsed() >= next_report {
            let pos = pb.position();
            println!(
                "  position = {} frames ({:.2} s) @ wall {:.2} s",
                pos, pos as f64 / dec.sample_rate.max(1) as f64,
                started.elapsed().as_secs_f64()
            );
            next_report += Duration::from_secs(2);
        }
        if started.elapsed() > deadline {
            println!("WARN: feed stalled (wrote {idx} / {} samples) — aborting", dec.samples.len());
            return;
        }
    }
    println!("feed COMPLETE — draining");

    // Don't return (and drop the track) until the ring has drained, or the
    // run-once teardown cuts the tail off.
    while pb.buffered_frames() > 0 {
        std::thread::sleep(Duration::from_millis(20));
        if started.elapsed() > deadline {
            break;
        }
    }
    println!("playback DONE ✓ ({:.2} s)", started.elapsed().as_secs_f64());
}
