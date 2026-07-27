---
name: reference_jellyfin_opus_ropus
description: "wandr.jellyfin Opus audio decode = pure-Rust `ropus` + wasm simd128 (after opus-rs=noise, oxideav/opus-decoder=too slow)"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-07-27T16:17:37.766Z
---

**wandr.jellyfin decodes Opus with the pure-Rust `ropus` crate (a full xiph/opus
port) + wasm `simd128` enabled.** Don't re-litigate the decoder choice — three
pure-Rust Opus crates were A/B-tested (decode Measure-for-Measure's stereo-CELT
Opus, compare SNR vs ffmpeg/libopus, and measure realtime factor):

| crate | correctness (SNR vs libopus) | speed | verdict |
|---|---|---|---|
| `oxideav-opus` | (never SNR'd) | ~0.3 Hz on-frame in-app | too slow, unusable |
| `opus-rs` (restsend fork) | **−1.7 dB = NOISE** (stereo CELT broken; Signal only uses it MONO) | fast | wrong |
| `opus-decoder` (Rusopus) | 40 dB clean | **1.2× realtime** (no hand-SIMD) → hiccups | correct but too slow |
| **`ropus` 0.12** | **40–42 dB clean**, bit-exact (all 24 RFC 6716/8251 vectors) | **117× realtime native**, ≈1× libopus | ✅ correct AND fast |

Key facts:
- `ropus` is fast because of the **`wide` portable-SIMD crate → wasm `simd128`**.
  That flag is **required**: `apps/user/wandr.jellyfin/.cargo/config.toml` sets
  `[target.wasm32-wasip2] rustflags=["-C","target-feature=+simd128"]`. Without it
  ropus runs scalar (~1× realtime, hiccups). The host wasmtime enables the SIMD
  proposal by default (`make_config()`), so it runs unchanged.
- `ropus` build.rs compiles C **only** for optional DNN/PLC weights; from a
  crates.io install (no xiph weights on disk) it writes a zero-byte blob → **no C
  compiled**, pure-Rust for our decode path. Only runtime dep is `wide`.
- API: `ropus::Decoder::new(48000, ropus::Channels::Stereo)` →
  `decode_float(packet, &mut [f32] sized 5760*ch, DecodeMode::Normal)` → samples/ch.
- **Symphonia has NO Opus decoder** (only AAC/MP3/FLAC/Vorbis). Don't look there.

Architecture note (also fixed this session): audio decode was moved OFF the
on-frame pump into **bg-tick** (`decode_audio` → bounded `pending_pcm`); the pump
is now codec-agnostic (drains PCM → device ring). A slow decoder used to starve
on-frame (the single wasm thread) → choppy video too. See the "one PCM pump + N
pluggable decoders" principle.

**Open TODO — 5.1 Opus:** `ropus::Decoder` is mono/stereo only, so a few 5.1 Opus
titles decode no audio. Fix = `ropus::OpusMultistreamDecoder` + 5.1→stereo
downmix (see the `channels.clamp(1,2)` comment in `setup_opus_audio`).

Related: `[[reference_dav1ddec_gstreamer_install.md]]` (AV1 SW decode on WSL),
`[[reference_gstreamer_desktop_backend_spike]]`.
