//! wandr.srtp.bench — on-device microbenchmark of the SRTP AES-256-GCM path the
//! Signal call engine runs per RTP packet. Mirrors `wandr-call::media::MediaSession`
//! exactly: build an RTP packet, `Context::encrypt_rtp`, then `Context::decrypt_rtp`,
//! using the `AeadAes256Gcm` (Signal / ringrtc V4) profile.
//!
//! Runs BOTH backends in one pass and prints them side by side:
//!   * IN-WASM   — `Context::new` → the `aes-gcm` crate compiled into the guest
//!                 (software AES; wasm can't reach the ARMv8 AES instructions).
//!   * HOST      — `Context::new_with_aead` → the GCM block delegated to the host's
//!                 hardware AES via `wandr:crypto/aead`.

wit_bindgen::generate!({
    world: "srtp-bench",
    path: "wit",
    generate_all,
});

use crate::wandr::crypto::aead_oneshot;
use crate::wandr::crypto::types::AeadAlgo;

use bytes::Bytes;
use rtc_rtp::header::Header;
use rtc_rtp::packet::Packet;
use rtc_shared::marshal::Marshal;
use rtc_srtp::context::Context;
use rtc_srtp::protection_profile::ProtectionProfile;
use rtc_srtp::{AeadCtx, AeadProvider};
use std::time::Instant;

/// AEAD_AES_256_GCM: 32 B master key, 12 B master salt (RFC 7714 / Signal V4).
const KEY: [u8; 32] = [0x42; 32];
const SALT: [u8; 12] = [0x24; 12];
const SSRC: u32 = 0x0000_07d2;
const PT: u8 = 102;

// ---- host AEAD backend: implements rtc-srtp's AeadProvider over wandr:crypto/aead ----

struct HostAeadProvider;
struct HostAeadCtx {
    algo: AeadAlgo,
    key: Vec<u8>,
}

impl AeadProvider for HostAeadProvider {
    fn new_key(&self, key: &[u8]) -> Box<dyn AeadCtx> {
        let algo = match key.len() {
            16 => AeadAlgo::Aes128Gcm,
            32 => AeadAlgo::Aes256Gcm,
            n => panic!("unexpected GCM session key length {n}"),
        };
        Box::new(HostAeadCtx { algo, key: key.to_vec() })
    }
}

impl AeadCtx for HostAeadCtx {
    fn seal(&self, nonce: &[u8], aad: &[u8], pt: &[u8]) -> Result<Vec<u8>, ()> {
        aead_oneshot::seal(self.algo, &self.key, nonce, aad, pt).map_err(|_| ())
    }
    fn open(&self, nonce: &[u8], aad: &[u8], ct: &[u8]) -> Result<Vec<u8>, ()> {
        aead_oneshot::open(self.algo, &self.key, nonce, aad, ct).map_err(|_| ())
    }
}

struct Workload {
    label: &'static str,
    payload: usize,
    pkt_per_sec: f64,
}

const WORKLOADS: &[Workload] = &[
    Workload { label: "audio (Opus 20ms)", payload: 160, pkt_per_sec: 50.0 },
    Workload { label: "video (VP8 slice)", payload: 1100, pkt_per_sec: 150.0 },
];

fn make_packet(seq: u16, payload_len: usize) -> Vec<u8> {
    let pkt = Packet {
        header: Header {
            version: 2,
            payload_type: PT,
            sequence_number: seq,
            timestamp: seq as u32 * 960,
            ssrc: SSRC,
            ..Default::default()
        },
        payload: Bytes::from(vec![0xABu8; payload_len]),
    };
    pkt.marshal().expect("marshal").to_vec()
}

/// (encrypt ns/op, decrypt ns/op) over `iters` packets, after `warmup`.
fn bench_pair(mut tx: Context, mut rx: Context, payload: usize, iters: usize, warmup: usize) -> (f64, f64) {
    let total = warmup + iters;
    let plain: Vec<Vec<u8>> =
        (0..total).map(|i| make_packet((i as u16).wrapping_add(1), payload)).collect();

    let mut sealed: Vec<Vec<u8>> = Vec::with_capacity(total);
    for p in plain.iter().take(warmup) {
        sealed.push(tx.encrypt_rtp(p).expect("warmup encrypt").to_vec());
    }
    let t0 = Instant::now();
    for p in plain.iter().skip(warmup) {
        sealed.push(tx.encrypt_rtp(p).expect("encrypt").to_vec());
    }
    let enc_ns = t0.elapsed().as_nanos() as f64 / iters as f64;

    for c in sealed.iter().take(warmup) {
        let _ = rx.decrypt_rtp(c).expect("warmup decrypt");
    }
    let t1 = Instant::now();
    for c in sealed.iter().skip(warmup) {
        let _ = rx.decrypt_rtp(c).expect("decrypt");
    }
    let dec_ns = t1.elapsed().as_nanos() as f64 / iters as f64;
    (enc_ns, dec_ns)
}

fn ctx_wasm() -> (Context, Context) {
    (
        Context::new(&KEY, &SALT, ProtectionProfile::AeadAes256Gcm, None, None).expect("tx"),
        Context::new(&KEY, &SALT, ProtectionProfile::AeadAes256Gcm, None, None).expect("rx"),
    )
}
fn ctx_host() -> (Context, Context) {
    (
        Context::new_with_aead(&KEY, &SALT, ProtectionProfile::AeadAes256Gcm, None, None, &HostAeadProvider).expect("tx"),
        Context::new_with_aead(&KEY, &SALT, ProtectionProfile::AeadAes256Gcm, None, None, &HostAeadProvider).expect("rx"),
    )
}

fn main() {
    let iters = 20_000usize;
    let warmup = 2_000usize;
    println!("=== wandr.srtp.bench — SRTP AES-256-GCM per-packet cost (Signal V4 profile) ===");
    println!("iters={iters} (warmup={warmup}) per workload\n");

    for w in WORKLOADS {
        let (we, wd) = bench_pair(ctx_wasm().0, ctx_wasm().1, w.payload, iters, warmup);
        let (he, hd) = bench_pair(ctx_host().0, ctx_host().1, w.payload, iters, warmup);

        let mbps = |ns: f64| (w.payload as f64) / (ns / 1e9) / 1e6;
        let cpu = |e: f64, d: f64| (e + d) * w.pkt_per_sec / 1e6; // ms CPU / 1 s call
        println!("{}  (payload={}B, {} pkt/s/dir)", w.label, w.payload, w.pkt_per_sec);
        println!(
            "   in-wasm  encrypt={:>8.0} ns ({:>6.1} MB/s)  decrypt={:>8.0} ns ({:>6.1} MB/s)  => {:>6.2} ms CPU/s",
            we, mbps(we), wd, mbps(wd), cpu(we, wd)
        );
        println!(
            "   host-HW  encrypt={:>8.0} ns ({:>6.1} MB/s)  decrypt={:>8.0} ns ({:>6.1} MB/s)  => {:>6.2} ms CPU/s",
            he, mbps(he), hd, mbps(hd), cpu(he, hd)
        );
        println!(
            "   speedup  encrypt={:>5.1}x  decrypt={:>5.1}x  (CPU {:>5.1}x lower)\n",
            we / he, wd / hd, cpu(we, wd) / cpu(he, hd)
        );
    }
    println!("=== done ===");
}
