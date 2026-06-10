//! wandr.crypto.test — a wasi:cli command guest that calls the `wandr:crypto` host
//! over the WIT boundary (task 93 Phase A end-to-end test). Focuses on the
//! AES-256-GCM SRTP profile Signal/ringrtc V4 uses, so this is "decode a Signal
//! audio AEAD frame via the host's HW crypto" exercised for real through wasm.

wit_bindgen::generate!({
    world: "crypto-test",
    path: "wit",
    generate_all,
});

use crate::wandr::crypto::aead::AeadKey;
use crate::wandr::crypto::types::{AeadAlgo, CryptoError, HashAlgo, KdfAlgo};
use crate::wandr::crypto::{caps, hash, kdf};

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn main() {
    println!("=== wandr.crypto.test — guest -> wandr:crypto host (WIT boundary) ===");

    // Capability discovery (the "what HW crypto is available" query).
    println!("-- caps.aeads (algorithm, hardware-accelerated) --");
    for (a, hw) in caps::aeads() {
        println!("   {a:?}: hw={hw}");
    }

    let mut ok = true;
    macro_rules! check {
        ($name:expr, $cond:expr) => {{
            let c = $cond;
            println!("   [{}] {}", if c { "ok " } else { "FAIL" }, $name);
            ok &= c;
        }};
    }

    println!("-- Signal SRTP profile: AES-256-GCM (AEAD_AES_256_GCM) via the host --");
    // A 32-byte SRTP key + 12-byte per-packet IV + RTP-header AAD + an "audio frame".
    let key = vec![0x42u8; 32];
    let nonce = vec![0x24u8; 12];
    let aad = b"\x80\x00rtp-header".to_vec();
    let frame = b"<<simulated Signal Opus audio frame payload>>".to_vec();

    let ctx = AeadKey::create(AeadAlgo::Aes256Gcm, &key).expect("AeadKey::create");
    let sealed = ctx.seal(&nonce, &aad, &frame).expect("seal");
    println!("   sealed {} -> {} bytes (incl. 16B GCM tag)", frame.len(), sealed.len());

    // DECODE (open) — the receive path a Signal audio call runs per packet.
    match ctx.open(&nonce, &aad, &sealed) {
        Ok(pt) => check!("AES-256-GCM decode (open) recovers the frame", pt == frame),
        Err(e) => {
            println!("   [FAIL] open errored: {e:?}");
            ok = false;
        }
    }

    // Tampered packet must be rejected (the SRTP auth guarantee).
    let mut forged = sealed.clone();
    *forged.last_mut().unwrap() ^= 0x01;
    check!(
        "tampered packet rejected (auth-failed)",
        matches!(ctx.open(&nonce, &aad, &forged), Err(CryptoError::AuthFailed))
    );

    // Wrong AAD (e.g. a rewritten RTP header) must also fail.
    check!(
        "wrong-AAD rejected",
        matches!(ctx.open(&nonce, b"different-header", &sealed), Err(CryptoError::AuthFailed))
    );

    println!("-- hash + kdf sanity (also host-side, HW SHA where available) --");
    // SHA-256("abc") known answer, computed by the host through the WIT.
    check!(
        "host SHA-256(\"abc\") KAT",
        hex(&hash::digest(HashAlgo::Sha256, b"abc"))
            == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // HKDF-SHA256 RFC5869 case 1 (the SRTP-style key-schedule primitive), host-side.
    let okm = kdf::derive(
        KdfAlgo::HkdfSha256,
        &hex_decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b"),
        &hex_decode("000102030405060708090a0b0c"),
        &hex_decode("f0f1f2f3f4f5f6f7f8f9"),
        0,
        42,
    )
    .expect("hkdf");
    check!(
        "host HKDF-SHA256 RFC5869-1",
        hex(&okm) == "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );

    println!(
        "=== RESULT: {} — wandr:crypto WIT boundary {} ===",
        if ok { "ALL PASS" } else { "FAILURES" },
        if ok { "works end-to-end" } else { "has failures" }
    );
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
