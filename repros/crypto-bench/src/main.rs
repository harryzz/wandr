//! SRTP crypto hot-path micro-bench. Times per-packet decrypt for the two SRTP
//! ciphers the call engine uses: AEAD-AES-256-GCM (live Signal call — slow soft
//! path in wasm) vs AES-128-CM + HMAC-SHA1 (the 10x-faster loopback path). A real
//! call is ~50 audio packets/sec; the printed packets/sec shows the ceiling each
//! cipher imposes in the current environment (wasm soft AES vs native HW AES).
//!
//! Run the same binary three ways (see Cargo.toml header): desktop-wasm,
//! desktop-native, device-native-aarch64. The wasm-vs-native gap is the
//! host-offload win; the GCM-vs-CM gap is why the loopback was faster.

use std::time::Instant;

use aes::cipher::{KeyIvInit, StreamCipher};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hmac::{Hmac, Mac};
use sha1::Sha1;

type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;
type HmacSha1 = Hmac<Sha1>;

// Representative SRTP audio packet: ~12 B RTP header + ~160 B Opus payload @ 64 kbps/20 ms.
const HDR: usize = 12;
const PAYLOAD: usize = 160;
const ITERS: u32 = 20_000;

fn main() {
    println!("crypto-bench: SRTP per-packet decrypt — {ITERS} iters, {PAYLOAD}B Opus payload");
    println!("(a real call = ~50 audio pkt/s; '/s' below = the per-core ceiling)\n");

    bench_gcm256();
    bench_aes128cm();

    println!("\nInterpretation: GCM/s ÷ CM/s = the loopback speedup; native/s ÷ wasm/s = the");
    println!("host-offload (ARMv8 HW AES) win. If wasm GCM/s is near ~50, decrypt is the cap.");
}

fn bench_gcm256() {
    let key = [0x42u8; 32];
    let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
    let nonce = Nonce::from_slice(&[0x07u8; 12]);
    let aad = [0u8; HDR];
    let plaintext = vec![0xABu8; PAYLOAD];
    // Seal once to get a valid ciphertext+tag, then time open() (the RX hot path).
    let ct = cipher
        .encrypt(nonce, Payload { msg: &plaintext, aad: &aad })
        .expect("seal");

    let t0 = Instant::now();
    let mut acc = 0usize;
    for _ in 0..ITERS {
        let pt = cipher
            .decrypt(nonce, Payload { msg: &ct, aad: &aad })
            .expect("open");
        acc = acc.wrapping_add(pt.len());
    }
    report("AES-256-GCM open ", t0.elapsed().as_nanos(), acc);
}

fn bench_aes128cm() {
    let key = [0x42u8; 16];
    let iv = [0x07u8; 16];
    let mac_key = [0x13u8; 20];
    // Packet = header + encrypted payload; decrypt = HMAC-SHA1 verify + AES-CTR.
    let mut packet = vec![0u8; HDR + PAYLOAD];
    Aes128Ctr::new(&key.into(), &iv.into()).apply_keystream(&mut packet[HDR..]);

    let t0 = Instant::now();
    let mut acc = 0usize;
    for _ in 0..ITERS {
        // Auth: HMAC-SHA1 over the whole packet (SRTP authenticates then decrypts).
        let mut mac = <HmacSha1 as Mac>::new_from_slice(&mac_key).unwrap();
        mac.update(&packet);
        let tag = mac.finalize().into_bytes();
        acc = acc.wrapping_add(tag[0] as usize);
        // Decrypt the payload (CTR is symmetric).
        let mut out = packet.clone();
        Aes128Ctr::new(&key.into(), &iv.into()).apply_keystream(&mut out[HDR..]);
        acc = acc.wrapping_add(out.len());
    }
    report("AES-128-CM+HMAC ", t0.elapsed().as_nanos(), acc);
}

fn report(name: &str, total_ns: u128, acc: usize) {
    let per = total_ns as f64 / ITERS as f64;
    let per_s = 1_000_000_000.0 / per;
    println!(
        "{name}: {:.2} µs/pkt → {:.0} pkt/s  (×{:.1} real-time vs 50/s)   [acc={acc}]",
        per / 1000.0,
        per_s,
        per_s / 50.0,
    );
}
